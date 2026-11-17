//! `guardian-worker --endpoint <path|pipe>`: started by the agent in the user's
//! session. Exit code 0 when the agent closes the connection, 2 on failure.

#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use guardian_platform::ipc::IpcEndpoint;

#[derive(Parser)]
#[command(
    name = "guardian-worker",
    version,
    about = "Guardian worker (started by guardian-agent)"
)]
struct Args {
    /// The agent's socket path or pipe name.
    #[arg(long)]
    endpoint: String,
    /// Seconds to keep trying to reach the agent.
    #[arg(long, default_value_t = 10)]
    connect_timeout: u64,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    match guardian_worker::run(
        IpcEndpoint::parse(&args.endpoint),
        Duration::from_secs(args.connect_timeout),
    )
    .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::from(2)
        }
    }
}
