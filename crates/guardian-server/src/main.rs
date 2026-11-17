//! `guardian-server`: the Guardian 3.0 server as a Linux daemon or Windows service.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use guardian_server::{ServerConfig, TlsMode};

#[cfg(windows)]
mod service_windows;

/// Guardian 3.0 server: gRPC for consoles, WebSocket for agents, on one TLS port.
#[derive(Debug, Clone, Parser)]
#[command(version, about)]
pub struct Cli {
    /// Directory for guardian.db and tls/ (Linux: /var/lib/guardian,
    /// Windows: %ProgramData%\Guardian Server).
    #[arg(long, env = "GUARDIAN_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// Address and port to listen on.
    #[arg(long, env = "GUARDIAN_LISTEN", default_value = "0.0.0.0:8443")]
    pub listen: SocketAddr,

    /// PEM certificate chain (with --tls-key). Without both, a self-signed
    /// certificate is generated and its SHA-256 is printed for pinning.
    #[arg(long, env = "GUARDIAN_TLS_CERT", requires = "tls_key")]
    pub tls_cert: Option<PathBuf>,

    /// PEM private key (with --tls-cert).
    #[arg(long, env = "GUARDIAN_TLS_KEY", requires = "tls_cert")]
    pub tls_key: Option<PathBuf>,

    /// Serve without TLS (only behind a TLS-terminating proxy).
    #[arg(
        long,
        env = "GUARDIAN_INSECURE_PLAINTEXT",
        action = clap::ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new(),
        conflicts_with_all = ["tls_cert", "tls_key"]
    )]
    pub insecure_plaintext: bool,

    /// Log filter (tracing syntax, e.g. `info` or `guardian_server=debug`).
    #[arg(long, env = "GUARDIAN_LOG", default_value = "info")]
    pub log: String,

    #[cfg(windows)]
    #[command(subcommand)]
    pub command: Option<service_windows::ServiceCmd>,
}

fn default_data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        let base = std::env::var_os("ProgramData")
            .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
        base.join("Guardian Server")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/var/lib/guardian")
    }
}

impl Cli {
    /// The server config these options describe.
    pub fn config(&self) -> ServerConfig {
        let tls = if self.insecure_plaintext {
            TlsMode::InsecurePlaintext
        } else if let (Some(cert), Some(key)) = (&self.tls_cert, &self.tls_key) {
            TlsMode::Files {
                cert: cert.clone(),
                key: key.clone(),
            }
        } else {
            TlsMode::SelfSigned
        };
        let mut c = ServerConfig::new(self.data_dir.clone().unwrap_or_else(default_data_dir), tls);
        c.listen = self.listen;
        c
    }
}

/// Logs to stderr, or (as a Windows service, where stderr goes nowhere) appends
/// to `file`.
fn init_logging(filter: &str, file: Option<&std::path::Path>) {
    use std::io::IsTerminal;
    let filter = tracing_subscriber::EnvFilter::try_new(filter)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let opened = file.and_then(|path| {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    });
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    let _ = match opened {
        Some(f) => builder
            .with_writer(std::sync::Mutex::new(f))
            .with_ansi(false)
            .try_init(),
        None => builder
            .with_writer(std::io::stderr)
            .with_ansi(std::io::stderr().is_terminal())
            .try_init(),
    };
}

/// Starts the server, logs the lines operators look for, calls `ready` once it is
/// listening, and runs until `stop` resolves.
pub async fn run_until(
    cli: &Cli,
    ready: impl FnOnce(),
    stop: impl std::future::Future<Output = ()>,
) -> Result<(), String> {
    let config = cli.config();
    if config.tls == TlsMode::InsecurePlaintext {
        tracing::warn!("--insecure-plaintext: run this only behind a TLS-terminating proxy");
    }
    let handle = guardian_server::start(config)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!("listening on {}", handle.local_addr());
    if let Some(pin) = handle.cert_sha256() {
        tracing::info!("certificate sha256: {pin}");
    }
    if let Some(code) = handle.bootstrap_code() {
        tracing::warn!("bootstrap code: {code}");
    }
    ready();
    stop.await;
    tracing::info!("shutting down");
    handle.shutdown().await;
    Ok(())
}

async fn signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = term.recv() => {}
                    _ = tokio::signal::ctrl_c() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    #[cfg(windows)]
    let log_file =
        service_windows::is_run(&cli).then(|| cli.config().data_dir.join("guardian-server.log"));
    #[cfg(not(windows))]
    let log_file: Option<PathBuf> = None;
    init_logging(&cli.log, log_file.as_deref());
    #[cfg(windows)]
    if let Some(cmd) = &cli.command {
        return service_windows::dispatch(cmd, &cli);
    }
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("cannot start the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(run_until(&cli, || {}, signal())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}
