//! Keeps one `guardian-worker` running in the active user session and talks to
//! it over local IPC. Only the process the agent itself started is accepted
//! (spec §10.4).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use guardian_platform::ipc::{IpcEndpoint, IpcListener, IpcStream, read_frame, write_frame};
use guardian_platform::{Platform, UserSession};
use guardian_proto::v1::{AgentToWorker, Ping, WorkerFrame, agent_to_worker, worker_frame};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::engine::Engine;

/// Time the worker has to send its hello after connecting.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// A started worker that has not connected by then is given up on.
const CONNECT_GRACE: Duration = Duration::from_secs(15);

/// How the worker is run.
#[derive(Debug, Clone)]
pub struct WorkerOptions {
    /// The worker executable.
    pub exe: PathBuf,
    /// Where the agent listens.
    pub endpoint: IpcEndpoint,
    /// How often the session is checked (2 s in production).
    pub check_every: Duration,
    /// How often the worker is pinged (5 s in production).
    pub ping_every: Duration,
}

#[derive(Default)]
struct Shared {
    connected: bool,
    pid: Option<u32>,
}

/// The running manager.
pub struct WorkerManager {
    shared: Arc<Mutex<Shared>>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl std::fmt::Debug for WorkerManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerManager")
            .field("connected", &self.connected())
            .field("pid", &self.pid())
            .finish()
    }
}

fn with<R>(m: &Mutex<Shared>, f: impl FnOnce(&mut Shared) -> R) -> R {
    let mut g = match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    f(&mut g)
}

impl WorkerManager {
    /// Starts managing the worker on the current tokio runtime.
    pub fn spawn(
        platform: Arc<dyn Platform>,
        opts: WorkerOptions,
        engine: Engine,
    ) -> WorkerManager {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (stop, stop_rx) = watch::channel(false);
        let task = tokio::spawn(manage(platform, opts, engine, shared.clone(), stop_rx));
        WorkerManager { shared, stop, task }
    }

    /// A worker has said hello and answers pings.
    pub fn connected(&self) -> bool {
        with(&self.shared, |s| s.connected)
    }

    /// The connected worker's PID.
    pub fn pid(&self) -> Option<u32> {
        with(&self.shared, |s| if s.connected { s.pid } else { None })
    }

    /// Closes the worker connection (the worker exits) and stops.
    pub async fn shutdown(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

/// The worker the agent started and is waiting for (or talking to).
struct Expected {
    pid: u32,
    session: String,
    started: Instant,
    conn: Option<Conn>,
}

struct Conn {
    generation: u64,
    close: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl Conn {
    async fn close(mut self) {
        if let Some(c) = self.close.take() {
            let _ = c.send(());
        }
        let _ = self.task.await;
    }
}

enum ConnEvent {
    Hello { generation: u64, pid: u32 },
    Ended { generation: u64 },
}

async fn active_session(platform: &Arc<dyn Platform>) -> Option<UserSession> {
    let p = platform.clone();
    match tokio::task::spawn_blocking(move || p.active_session()).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            tracing::warn!("cannot read the active session: {e}");
            None
        }
        Err(e) => {
            tracing::warn!("session lookup failed: {e}");
            None
        }
    }
}

async fn spawn_worker(
    platform: &Arc<dyn Platform>,
    opts: &WorkerOptions,
    s: UserSession,
) -> Option<u32> {
    let p = platform.clone();
    let exe = opts.exe.clone();
    let args = vec!["--endpoint".to_string(), opts.endpoint.to_arg()];
    let id = s.id.clone();
    match tokio::task::spawn_blocking(move || p.spawn_in_session(&s, &exe, &args)).await {
        Ok(Ok(pid)) => {
            tracing::info!("started the worker (pid {pid}) in session {id}");
            Some(pid)
        }
        Ok(Err(e)) => {
            tracing::warn!("cannot start the worker in session {id}: {e}");
            None
        }
        Err(e) => {
            tracing::warn!("worker start failed: {e}");
            None
        }
    }
}

async fn drop_expected(exp: &mut Option<Expected>, engine: &Engine, shared: &Mutex<Shared>) {
    if let Some(c) = exp.take().and_then(|e| e.conn) {
        c.close().await;
    }
    engine.set_worker_pid(None);
    with(shared, |s| *s = Shared::default());
}

async fn manage(
    platform: Arc<dyn Platform>,
    opts: WorkerOptions,
    engine: Engine,
    shared: Arc<Mutex<Shared>>,
    mut stop: watch::Receiver<bool>,
) {
    let mut listener = match IpcListener::bind(&opts.endpoint) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(
                "cannot listen for the worker on {}: {e}; running without a worker",
                opts.endpoint.to_arg()
            );
            let _ = stop.wait_for(|s| *s).await;
            return;
        }
    };
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<ConnEvent>();
    let mut tick = tokio::time::interval(opts.check_every);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut exp: Option<Expected> = None;
    let mut next_gen: u64 = 0;
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            _ = tick.tick() => {
                let session = active_session(&platform).await;
                let stale = match (&exp, &session) {
                    (Some(_), None) => {
                        tracing::info!("no active session: closing the worker");
                        true
                    }
                    (Some(e), Some(s)) if e.session != s.id => {
                        tracing::info!("session changed ({} → {}): restarting the worker", e.session, s.id);
                        true
                    }
                    (Some(e), Some(_)) if e.conn.is_none() && e.started.elapsed() > CONNECT_GRACE => {
                        tracing::warn!("worker pid {} never connected; starting another", e.pid);
                        true
                    }
                    _ => false,
                };
                if stale {
                    drop_expected(&mut exp, &engine, &shared).await;
                }
                if exp.is_none()
                    && let Some(s) = session
                {
                    let id = s.id.clone();
                    if let Some(pid) = spawn_worker(&platform, &opts, s).await {
                        engine.set_worker_pid(Some(pid));
                        exp = Some(Expected { pid, session: id, started: Instant::now(), conn: None });
                    }
                }
            }
            r = listener.accept() => match r {
                Ok((stream, peer)) => {
                    let accept = matches!(&exp, Some(e) if e.pid == peer && e.conn.is_none());
                    if accept {
                        next_gen += 1;
                        let generation = next_gen;
                        let (close_tx, close_rx) = oneshot::channel();
                        let task = tokio::spawn(talk(stream, peer, generation, opts.ping_every, ev_tx.clone(), close_rx));
                        if let Some(e) = exp.as_mut() {
                            e.conn = Some(Conn { generation, close: Some(close_tx), task });
                        }
                    } else {
                        tracing::warn!("rejected a worker connection from pid {peer}: not the worker this agent started");
                        drop(stream);
                    }
                }
                Err(e) => {
                    tracing::warn!("worker accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            Some(ev) = ev_rx.recv() => match ev {
                ConnEvent::Hello { generation, pid } => {
                    if matches!(&exp, Some(Expected { conn: Some(c), .. }) if c.generation == generation) {
                        with(&shared, |s| { s.connected = true; s.pid = Some(pid); });
                    }
                }
                ConnEvent::Ended { generation } => {
                    if matches!(&exp, Some(Expected { conn: Some(c), .. }) if c.generation == generation) {
                        tracing::info!("worker connection ended");
                        drop_expected(&mut exp, &engine, &shared).await;
                    }
                }
            },
        }
    }
    drop_expected(&mut exp, &engine, &shared).await;
}

/// One worker connection: hello within [`HELLO_TIMEOUT`], then pings; ends on
/// close, a missed pong (`2 × ping_every`) or a broken connection.
async fn talk(
    stream: IpcStream,
    peer: u32,
    generation: u64,
    ping_every: Duration,
    ev: mpsc::UnboundedSender<ConnEvent>,
    mut close: oneshot::Receiver<()>,
) {
    let (mut rd, mut wr) = tokio::io::split(stream);
    match tokio::time::timeout(HELLO_TIMEOUT, read_frame::<_, WorkerFrame>(&mut rd)).await {
        Ok(Ok(WorkerFrame {
            msg: Some(worker_frame::Msg::Hello(h)),
        })) => {
            tracing::info!(
                "worker pid {peer} connected (session {:?}, user {:?}, version {})",
                h.session_id,
                h.user,
                h.worker_version
            );
            let _ = ev.send(ConnEvent::Hello {
                generation,
                pid: peer,
            });
        }
        other => {
            tracing::warn!("worker pid {peer} sent no hello: {other:?}");
            let _ = ev.send(ConnEvent::Ended { generation });
            return;
        }
    }
    let (fr_tx, mut fr_rx) = mpsc::channel::<WorkerFrame>(8);
    let reader = tokio::spawn(async move {
        while let Ok(f) = read_frame::<_, WorkerFrame>(&mut rd).await {
            if fr_tx.send(f).await.is_err() {
                break;
            }
        }
    });
    let mut tick = tokio::time::interval(ping_every);
    let mut seq: u64 = 0;
    let mut last_pong = Instant::now();
    loop {
        tokio::select! {
            _ = &mut close => break,
            _ = tick.tick() => {
                if last_pong.elapsed() > 2 * ping_every {
                    tracing::warn!("worker pid {peer} stopped answering pings");
                    break;
                }
                seq += 1;
                let ping = AgentToWorker { msg: Some(agent_to_worker::Msg::Ping(Ping { seq })) };
                if write_frame(&mut wr, &ping).await.is_err() {
                    break;
                }
            }
            f = fr_rx.recv() => match f {
                Some(WorkerFrame { msg: Some(worker_frame::Msg::Pong(_)) }) => last_pong = Instant::now(),
                Some(_) => tracing::debug!("unexpected frame from the worker"),
                None => break,
            },
        }
    }
    reader.abort();
    let _ = ev.send(ConnEvent::Ended { generation });
}
