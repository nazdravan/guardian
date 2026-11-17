//! The agent: stored state → engine → server connection, and the rules for what
//! arrives (newer states only, each command answered once).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use guardian_core::keys::KeyPair;
use guardian_platform::Platform;
use guardian_proto::v1::{AgentState, CommandEnvelope, CommandResult, ResultStatus, StoredState};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::config::{AgentConfig, AgentPaths};
use crate::conn::{self, ConnConfig, ConnEvent, Connection};
use crate::engine::{Engine, EngineConfig, EngineMode, Policy};
use crate::store::{self, SeenIds, StoreError};
use crate::worker::{WorkerManager, WorkerOptions};

/// Features this agent supports (stage 4 adds them).
pub const CAPABILITIES: &[&str] = &[];
/// The agent's version, sent to the server.
pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Why the agent could not start or enroll.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// No `computer_id` in the config (or no config).
    #[error("this computer is not enrolled; run `guardian-agent enroll` first")]
    NotEnrolled,
    /// `enroll` on an enrolled computer.
    #[error("already enrolled (computer {0}); delete agent.key and the config to re-enroll")]
    AlreadyEnrolled(u64),
    /// Files.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The server refused or could not be reached.
    #[error(transparent)]
    Conn(#[from] conn::ConnError),
}

/// How to run.
#[derive(Clone)]
pub struct AgentOptions {
    /// Files.
    pub paths: AgentPaths,
    /// OS access.
    pub platform: Arc<dyn Platform>,
    /// Engine intervals.
    pub engine: EngineConfig,
    /// First reconnect delay (1 s in production).
    pub backoff_min: Duration,
    /// The worker in the user session (`None` = no worker).
    pub worker: Option<WorkerOptions>,
}

impl std::fmt::Debug for AgentOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOptions")
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

/// What the agent is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatus {
    /// Authenticated connection right now.
    pub online: bool,
    /// Time without an authenticated connection.
    pub offline_for: Option<Duration>,
    /// Version of the applied state.
    pub state_version: u64,
    /// Effective profile id.
    pub profile_id: i64,
    /// Effective profile name.
    pub profile_name: String,
    /// How process starts are seen.
    pub engine_mode: EngineMode,
    /// Processes killed since start.
    pub kills: u64,
    /// The server revoked this computer.
    pub revoked: bool,
    /// A worker is connected from the user session.
    pub worker_connected: bool,
}

/// Registers this computer with `code` and writes the config (key created if needed).
pub async fn enroll(
    paths: &AgentPaths,
    server: &str,
    pin: Option<&str>,
    code: &str,
    platform: &dyn Platform,
) -> Result<u64, AgentError> {
    if let Ok(AgentConfig {
        computer_id: Some(id),
        ..
    }) = AgentConfig::load(&paths.config)
    {
        return Err(AgentError::AlreadyEnrolled(id));
    }
    std::fs::create_dir_all(&paths.data).map_err(|e| StoreError::io(&paths.data, e))?;
    let key = store::load_or_create_key(&paths.key())?;
    let id = conn::enroll(server, pin, code, &key, platform.system_info(AGENT_VERSION)).await?;
    AgentConfig {
        server: server.to_string(),
        cert_sha256: pin.map(str::to_string),
        computer_id: Some(id),
    }
    .save(&paths.config)?;
    tracing::info!("enrolled as computer {id}");
    Ok(id)
}

struct Live {
    stored: StoredState,
    online: bool,
    revoked: bool,
}

/// A running agent.
pub struct AgentHandle {
    live: Arc<Mutex<Live>>,
    engine: Engine,
    conn: Arc<Connection>,
    worker: Option<WorkerManager>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl std::fmt::Debug for AgentHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentHandle")
            .field("status", &self.status())
            .finish()
    }
}

fn state_of(s: &StoredState) -> AgentState {
    s.state.clone().unwrap_or_default()
}

impl AgentHandle {
    /// A snapshot of the agent's status.
    pub fn status(&self) -> AgentStatus {
        let (stored, online, revoked) = match self.live.lock() {
            Ok(l) => (l.stored.clone(), l.online, l.revoked),
            Err(_) => (store::default_state(), false, false),
        };
        let policy = Policy::from_state(&state_of(&stored));
        let stats = self.engine.stats();
        AgentStatus {
            online,
            offline_for: if online {
                None
            } else {
                self.conn.offline_for()
            },
            state_version: policy.version,
            profile_id: policy.profile.id.0,
            profile_name: policy.profile.name,
            engine_mode: self.engine.mode(),
            kills: stats.kills,
            revoked,
            worker_connected: self.worker.as_ref().is_some_and(WorkerManager::connected),
        }
    }

    /// The engine (the worker manager needs it).
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Stops the connection and the engine.
    pub async fn shutdown(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
        if let Some(w) = self.worker {
            w.shutdown().await;
        }
        if let Ok(c) = Arc::try_unwrap(self.conn) {
            c.shutdown().await;
        }
        self.engine.shutdown().await;
    }
}

/// Starts the agent: the saved (or default) state is enforced before any
/// connection attempt.
pub async fn run(opts: AgentOptions) -> Result<AgentHandle, AgentError> {
    let cfg = AgentConfig::load(&opts.paths.config).map_err(|_| AgentError::NotEnrolled)?;
    let computer_id = cfg.computer_id.ok_or(AgentError::NotEnrolled)?;
    let key: KeyPair = store::load_or_create_key(&opts.paths.key())?;
    let stored = store::load_state(&opts.paths.state()).unwrap_or_else(store::default_state);
    let engine = Engine::spawn(
        opts.platform.clone(),
        Policy::from_state(&state_of(&stored)),
        opts.paths.whitelist(),
        opts.engine,
    );
    tracing::info!(
        "enforcing {:?} (state version {}) before connecting",
        Policy::from_state(&state_of(&stored)).profile.name,
        state_of(&stored).version
    );
    let (tx, mut rx) = mpsc::channel(64);
    let conn = Arc::new(Connection::spawn(
        ConnConfig {
            server: cfg.server.clone(),
            pin: cfg.cert_sha256.clone(),
            computer_id,
            capabilities: CAPABILITIES.iter().map(|c| c.to_string()).collect(),
            backoff_min: opts.backoff_min,
            backoff_max: Duration::from_secs(60),
        },
        key,
        opts.platform.system_info(AGENT_VERSION),
        tx,
    ));
    let live = Arc::new(Mutex::new(Live {
        stored,
        online: false,
        revoked: false,
    }));
    let (stop, mut stop_rx) = watch::channel(false);
    let mut seen = SeenIds::load(&opts.paths.seen());
    let task = tokio::spawn({
        let live = live.clone();
        let engine = engine.clone();
        let conn = conn.clone();
        let paths = opts.paths.clone();
        async move {
            loop {
                let ev = tokio::select! {
                    _ = stop_rx.changed() => return,
                    ev = rx.recv() => match ev { Some(ev) => ev, None => return },
                };
                handle_event(ev, computer_id, &live, &engine, &conn, &paths, &mut seen);
            }
        }
    });
    let worker = opts
        .worker
        .map(|w| WorkerManager::spawn(opts.platform.clone(), w, engine.clone()));
    Ok(AgentHandle {
        live,
        engine,
        conn,
        worker,
        stop,
        task,
    })
}

fn with<R>(live: &Mutex<Live>, f: impl FnOnce(&mut Live) -> R) -> R {
    let mut g = match live.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    f(&mut g)
}

fn save(paths: &AgentPaths, stored: &StoredState) {
    if let Err(e) = store::save_state(&paths.state(), stored) {
        tracing::error!("cannot save state: {e}");
    }
}

/// Applies `st` if it is newer (or the current one is the default, version 0).
fn apply_state(
    st: AgentState,
    computer_id: u64,
    live: &Mutex<Live>,
    engine: &Engine,
    paths: &AgentPaths,
) {
    let updated = with(live, |l| {
        let current = state_of(&l.stored).version;
        if st.version <= current && current != 0 {
            tracing::debug!("ignoring state version {} (have {current})", st.version);
            return None;
        }
        l.stored.state = Some(st.clone());
        l.stored.computer_id = computer_id;
        l.stored.saved_at_unix = unix_now();
        Some(l.stored.clone())
    });
    if let Some(stored) = updated {
        save(paths, &stored);
        let p = Policy::from_state(&st);
        tracing::info!(
            "applying profile {:?} (state version {})",
            p.profile.name,
            p.version
        );
        engine.set_policy(p);
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(0))
        .unwrap_or(0)
}

fn answer(cmd: &CommandEnvelope, conn: &Connection, paths: &AgentPaths, seen: &mut SeenIds) {
    if !seen.insert(&cmd.id) {
        tracing::debug!("command {} already handled", cmd.id);
        return;
    }
    if let Err(e) = seen.save(&paths.seen()) {
        tracing::warn!("cannot save seen commands: {e}");
    }
    tracing::info!("command {} is not supported by this agent version", cmd.id);
    conn.send_result(CommandResult {
        command_id: cmd.id.clone(),
        status: ResultStatus::ResultUnsupported as i32,
        detail: "not supported by this agent version".into(),
    });
}

fn handle_event(
    ev: ConnEvent,
    computer_id: u64,
    live: &Mutex<Live>,
    engine: &Engine,
    conn: &Connection,
    paths: &AgentPaths,
    seen: &mut SeenIds,
) {
    match ev {
        ConnEvent::Online { welcome } => {
            let trust = with(live, |l| {
                l.online = true;
                l.stored.consoles = welcome.consoles.clone();
                l.stored.users = welcome.users.clone();
                l.stored.clone()
            });
            save(paths, &trust);
            if let Some(st) = welcome.state {
                apply_state(st, computer_id, live, engine, paths);
            }
            for cmd in &welcome.pending {
                answer(cmd, conn, paths, seen);
            }
        }
        ConnEvent::State(st) => apply_state(st, computer_id, live, engine, paths),
        ConnEvent::Trust(w) => {
            let stored = with(live, |l| {
                l.stored.consoles = w.consoles;
                l.stored.users = w.users;
                l.stored.clone()
            });
            save(paths, &stored);
        }
        ConnEvent::Command(cmd) => answer(&cmd, conn, paths, seen),
        ConnEvent::Offline { .. } => with(live, |l| l.online = false),
        ConnEvent::Revoked => {
            tracing::warn!("revoked: keeping the last policy");
            with(live, |l| {
                l.online = false;
                l.revoked = true;
            });
        }
    }
}
