//! Guardian worker: runs in the logged-in user's session and talks to the agent
//! over local IPC. Stage 2 only says hello and answers pings; stage 4 adds the
//! visible features (messages, screen lock).

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

use guardian_platform::ipc::{self, IpcEndpoint, IpcStream, read_frame, write_frame};
use guardian_proto::v1::{
    AgentToWorker, Pong, WorkerFrame, WorkerHello, agent_to_worker, worker_frame,
};

/// The worker's version, sent in the hello.
pub const WORKER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Why the worker stopped.
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// The agent could not be reached within the connect timeout.
    #[error("cannot reach the agent at {endpoint}: {source}")]
    Connect {
        /// Where it tried.
        endpoint: String,
        /// The last error.
        source: std::io::Error,
    },
    /// The connection broke in an unexpected way.
    #[error("connection to the agent failed: {0}")]
    Io(#[from] std::io::Error),
}

async fn connect(ep: &IpcEndpoint, timeout: Duration) -> Result<IpcStream, WorkerError> {
    let end = Instant::now() + timeout;
    loop {
        match ipc::connect(ep).await {
            Ok(s) => return Ok(s),
            Err(e) if Instant::now() >= end => {
                return Err(WorkerError::Connect {
                    endpoint: ep.to_arg(),
                    source: e,
                });
            }
            Err(e) => {
                tracing::debug!("agent not reachable yet: {e}");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

fn env_or_empty(names: &[&str]) -> String {
    names
        .iter()
        .find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default()
}

/// Connects (retrying for `connect_timeout`), sends the hello, answers every
/// ping, and returns `Ok` when the agent closes the connection.
pub async fn run(endpoint: IpcEndpoint, connect_timeout: Duration) -> Result<(), WorkerError> {
    let mut s = connect(&endpoint, connect_timeout).await?;
    let hello = WorkerFrame {
        msg: Some(worker_frame::Msg::Hello(WorkerHello {
            pid: std::process::id(),
            session_id: env_or_empty(&["GUARDIAN_SESSION"]),
            user: env_or_empty(&["USER", "USERNAME"]),
            worker_version: WORKER_VERSION.to_string(),
        })),
    };
    write_frame(&mut s, &hello).await?;
    loop {
        let m: AgentToWorker = match read_frame(&mut s).await {
            Ok(m) => m,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionReset
                ) =>
            {
                tracing::info!("agent closed the connection");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        match m.msg {
            Some(agent_to_worker::Msg::Ping(p)) => {
                let pong = WorkerFrame {
                    msg: Some(worker_frame::Msg::Pong(Pong { seq: p.seq })),
                };
                write_frame(&mut s, &pong).await?;
            }
            None => tracing::debug!("empty frame from the agent"),
        }
    }
}
