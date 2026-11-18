//! `guardian-agent`: enrolls this computer and enforces its profile, as a Linux
//! daemon or a Windows service.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use guardian_agent::config::{AgentConfig, AgentPaths};
use guardian_agent::engine::{EngineConfig, Policy};
use guardian_agent::store::{self, SeenIds};
use guardian_agent::worker::WorkerOptions;
use guardian_agent::{AgentError, AgentOptions};
use guardian_platform::ipc::IpcEndpoint;

#[cfg(windows)]
mod service_windows;

/// Guardian 3.0 agent: keeps a connection to the Guardian server and enforces
/// this computer's profile.
#[derive(Debug, Clone, Parser)]
#[command(name = "guardian-agent", version, about)]
pub struct Cli {
    /// Config file (Linux: /etc/guardian-agent/agent.toml,
    /// Windows: %ProgramData%\Guardian Agent\agent.toml).
    #[arg(long, global = true, env = "GUARDIAN_AGENT_CONFIG")]
    pub config: Option<PathBuf>,

    /// Directory for the key, the saved state and the local whitelist
    /// (Linux: /var/lib/guardian-agent, Windows: %ProgramData%\Guardian Agent).
    #[arg(long, global = true, env = "GUARDIAN_AGENT_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// Log filter (tracing syntax, e.g. `info` or `guardian_agent=debug`).
    #[arg(
        long,
        global = true,
        env = "GUARDIAN_AGENT_LOG",
        default_value = "info"
    )]
    pub log: String,

    #[command(subcommand)]
    pub command: Cmd,
}

/// What to do.
#[derive(Debug, Clone, Subcommand)]
pub enum Cmd {
    /// Register this computer with the server using an enrollment code.
    Enroll {
        /// The server, `wss://host:port`.
        #[arg(long, env = "GUARDIAN_AGENT_SERVER")]
        server: String,
        /// The enrollment code from the console.
        #[arg(long, env = "GUARDIAN_AGENT_CODE")]
        code: String,
        /// SHA-256 of the server certificate (self-signed servers print it).
        #[arg(long, env = "GUARDIAN_AGENT_CERT_SHA256")]
        cert_sha256: Option<String>,
    },
    /// Run in the foreground until SIGINT/SIGTERM (Ctrl+C on Windows).
    Run,
    /// Show the saved configuration and state.
    Status,
    /// Manage the Windows service.
    #[cfg(windows)]
    Service {
        #[command(subcommand)]
        action: service_windows::ServiceAction,
    },
}

impl Cli {
    /// The file locations these options describe.
    pub fn paths(&self) -> AgentPaths {
        let d = AgentPaths::system_default();
        AgentPaths {
            config: self.config.clone().unwrap_or(d.config),
            data: self.data_dir.clone().unwrap_or(d.data),
        }
    }
}

/// Logs to stderr, or (as a Windows service) appends to `file`.
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

/// The worker next to this executable, if it is installed.
fn worker_options() -> Option<WorkerOptions> {
    let name = if cfg!(windows) {
        "guardian-worker.exe"
    } else {
        "guardian-worker"
    };
    let exe = std::env::current_exe().ok()?.parent()?.join(name);
    if !exe.is_file() {
        tracing::info!("{} not found: running without a worker", exe.display());
        return None;
    }
    Some(WorkerOptions {
        exe,
        endpoint: IpcEndpoint::system_default(),
        check_every: Duration::from_secs(2),
        ping_every: Duration::from_secs(5),
    })
}

/// Starts the agent, calls `ready` once the saved state is enforced, and runs
/// until `stop` resolves.
pub async fn run_until(
    cli: &Cli,
    ready: impl FnOnce(),
    stop: impl std::future::Future<Output = ()>,
) -> Result<(), AgentError> {
    let h = guardian_agent::run(AgentOptions {
        paths: cli.paths(),
        platform: guardian_platform::current(),
        engine: EngineConfig::default(),
        backoff_min: Duration::from_secs(1),
        worker: worker_options(),
        offline_after: guardian_core::local::OFFLINE_AFTER,
    })
    .await?;
    ready();
    stop.await;
    tracing::info!("shutting down");
    h.shutdown().await;
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

fn status(paths: &AgentPaths) -> Result<(), AgentError> {
    let cfg = AgentConfig::load(&paths.config).map_err(|_| AgentError::NotEnrolled)?;
    println!("config: {}", paths.config.display());
    println!("server: {}", cfg.server);
    match cfg.computer_id {
        Some(id) => println!("computer_id: {id}"),
        None => println!("computer_id: (not enrolled)"),
    }
    let stored = store::load_state(&paths.state());
    let saved = stored.is_some();
    let stored = stored.unwrap_or_else(store::default_state);
    let st = stored.state.clone().unwrap_or_default();
    let p = Policy::from_state(&st);
    if saved {
        println!("state version: {}", st.version);
    } else {
        println!("state version: (none saved; the default lesson applies)");
    }
    println!("profile: {} (id {})", p.profile.name, p.profile.id.0);
    println!("seen commands: {}", SeenIds::load(&paths.seen()).len());
    Ok(())
}

fn runtime() -> Result<tokio::runtime::Runtime, ExitCode> {
    tokio::runtime::Runtime::new().map_err(|e| {
        eprintln!("cannot start the async runtime: {e}");
        ExitCode::FAILURE
    })
}

fn fail(e: impl std::fmt::Display) -> ExitCode {
    eprintln!("error: {e}");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    #[cfg(windows)]
    let log_file = service_windows::is_run(&cli).then(|| cli.paths().log());
    #[cfg(not(windows))]
    let log_file: Option<PathBuf> = None;
    init_logging(&cli.log, log_file.as_deref());
    let paths = cli.paths();
    match &cli.command {
        Cmd::Enroll {
            server,
            code,
            cert_sha256,
        } => {
            // The key is written next: protect the directories first.
            #[cfg(windows)]
            for dir in [Some(paths.data.as_path()), paths.config.parent()]
                .into_iter()
                .flatten()
            {
                if let Err(e) = service_windows::protect_dir(dir) {
                    return fail(e);
                }
            }
            let rt = match runtime() {
                Ok(rt) => rt,
                Err(c) => return c,
            };
            let platform = guardian_platform::current();
            match rt.block_on(guardian_agent::enroll(
                &paths,
                server,
                cert_sha256.as_deref(),
                code,
                &*platform,
            )) {
                Ok(id) => {
                    println!("enrolled as computer {id}");
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        Cmd::Run => {
            let rt = match runtime() {
                Ok(rt) => rt,
                Err(c) => return c,
            };
            match rt.block_on(run_until(&cli, || {}, signal())) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    tracing::error!("{e}");
                    fail(e)
                }
            }
        }
        Cmd::Status => match status(&paths) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(e),
        },
        #[cfg(windows)]
        Cmd::Service { action } => service_windows::dispatch(*action, &cli),
    }
}
