//! Guardian 3.0 server: gRPC for consoles and the CLI, WebSocket for agents,
//! SQLite for state — all on one port.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub mod auth;
pub mod batch;
pub mod config;
pub mod db;
pub mod grpc;
pub mod net;
pub mod state;
#[cfg(feature = "testing")]
pub mod testing;

pub use config::{ServerConfig, TlsMode};
pub use state::AppState;

/// Why the server could not start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// Filesystem problem (data dir, certificate files).
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// Database problem.
    #[error(transparent)]
    Db(#[from] db::DbError),
    /// TLS setup problem.
    #[error("tls: {0}")]
    Tls(String),
}

/// A running server.
#[derive(Debug)]
pub struct ServerHandle {
    addr: SocketAddr,
    state: Arc<AppState>,
    cert_sha256: Option<String>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl ServerHandle {
    /// The address actually bound (useful with port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// The bootstrap code, while no user exists.
    pub fn bootstrap_code(&self) -> Option<String> {
        self.state.bootstrap_code()
    }

    /// SHA-256 hex of the served certificate; `None` in plaintext mode.
    pub fn cert_sha256(&self) -> Option<String> {
        self.cert_sha256.clone()
    }

    /// The shared state (for embedding and tests).
    pub fn state(&self) -> &Arc<AppState> {
        &self.state
    }

    /// Stops accepting, lets connections finish briefly, flushes batched writes and
    /// closes the database.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        for t in self.tasks {
            let _ = t.await;
        }
        self.state.batcher.flush().await;
        self.state.db.close().await;
    }
}

/// Opens the database, seeds it on first start, and starts listening.
pub async fn start(config: ServerConfig) -> Result<ServerHandle, StartError> {
    if config.tls != TlsMode::InsecurePlaintext {
        return Err(StartError::Tls("TLS is not implemented yet".into()));
    }
    std::fs::create_dir_all(&config.data_dir)?;
    let db = db::Db::open(&config.data_dir.join("guardian.db")).await?;
    db::profiles::seed_defaults(&db).await?;
    let users = {
        let mut conn = db.write.acquire().await.map_err(db::DbError::from)?;
        db::users::count(&mut conn).await?
    };
    let bootstrap = (users == 0).then(guardian_core::codes::generate_code);
    if let Some(code) = &bootstrap {
        tracing::warn!("bootstrap code: {code}");
    }
    let batcher = batch::Batcher::spawn(db.clone());
    let state = Arc::new(AppState {
        db,
        batcher,
        limiter: auth::RateLimiter::new(config.auth_rate_per_minute),
        bootstrap: Mutex::new(bootstrap),
        config: config.clone(),
    });
    let listener = TcpListener::bind(config.listen).await?;
    let addr = listener.local_addr()?;
    let (tx, rx) = watch::channel(false);
    let router = grpc::router(state.clone());
    let serve = tokio::spawn(net::serve(listener, router, rx));
    Ok(ServerHandle {
        addr,
        state,
        cert_sha256: None,
        shutdown: tx,
        tasks: vec![serve],
    })
}
