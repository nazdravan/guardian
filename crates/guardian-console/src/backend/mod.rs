//! The backend: a thread with its own tokio runtime that takes [`Action`]s from
//! the UI, talks gRPC to the server, and sends [`Event`]s back. The UI only
//! draws from events, so it can be tested with a fake backend.

mod conn;
mod errors;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use guardian_client::Authed;
use guardian_proto::v1::{EnrollCode, Profile, Settings, User, WatchEvent};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tonic::transport::Channel;

pub use errors::{login_error_text, status_text};

use crate::fleet::Fleet;
use crate::settings::{ConsoleSettings, Mode, Secret};

/// What the UI asks for.
#[derive(Debug, Clone)]
pub enum Action {
    /// Connect to a remote server (validated by the UI) and remember it.
    Connect {
        /// `https://host:port` (tests: `http://`).
        server: String,
        /// Pinned certificate SHA-256.
        cert_sha256: Option<String>,
    },
    /// Sign in.
    Login {
        /// Login name.
        login: String,
        /// Password.
        password: Secret,
    },
    /// Create the first administrator; an empty code = the embedded server's.
    Init {
        /// Bootstrap code.
        bootstrap_code: String,
        /// Login name.
        login: String,
        /// Password.
        password: Secret,
    },
    /// Sign out.
    Logout,
    /// Stop everything (the window is closing).
    Shutdown,
}

/// The connection as the user sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnState {
    /// Nothing chosen yet.
    Disconnected,
    /// First attempt.
    Connecting,
    /// Connected, no session.
    NeedsLogin {
        /// The server has no administrator yet and the console holds its
        /// bootstrap code (embedded server).
        can_init: bool,
    },
    /// Signed in and following the server.
    Online,
    /// The connection broke; retrying.
    Reconnecting {
        /// Attempt number.
        attempt: u32,
        /// Why.
        error: String,
    },
}

/// What the backend tells the UI.
#[derive(Debug, Clone)]
pub enum Event {
    /// Connection state.
    Connection(ConnState),
    /// Signed in as.
    LoggedIn(User),
    /// Signed out; `reason` when the server ended the session.
    LoggedOut {
        /// Why, if not asked for.
        reason: Option<String>,
    },
    /// The whole fleet after a change.
    Fleet(Fleet),
    /// Profiles (admin).
    Profiles(Vec<Profile>),
    /// Users (admin).
    Users(Vec<User>),
    /// School settings (admin).
    Settings(Settings),
    /// A new enrollment code.
    Code(EnrollCode),
    /// A short confirmation.
    Done(String),
    /// Something failed.
    Error(String),
}

/// The UI's view of a backend.
pub trait Backend {
    /// Queues an action.
    fn send(&self, a: Action);
    /// Events since the last call; never blocks.
    fn poll(&self) -> Vec<Event>;
}

type Repaint = Box<dyn Fn() + Send + Sync>;

/// Shared state of the backend thread.
pub(crate) struct Core {
    events: std::sync::mpsc::Sender<Event>,
    repaint: Repaint,
    settings_path: PathBuf,
    settings: Mutex<ConsoleSettings>,
    channel: Mutex<Option<Channel>>,
    fleet: Mutex<Fleet>,
    session: Mutex<Option<JoinHandle<()>>>,
    bootstrap: Mutex<Option<String>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

impl Core {
    pub(crate) fn emit(&self, e: Event) {
        if self.events.send(e).is_ok() {
            (self.repaint)();
        }
    }

    pub(crate) fn emit_needs_login(&self) {
        let can_init = lock(&self.bootstrap).is_some();
        self.emit(Event::Connection(ConnState::NeedsLogin { can_init }));
    }

    fn save_settings(&self, s: &ConsoleSettings) {
        if let Err(e) = s.save(&self.settings_path) {
            tracing::warn!("cannot save the console settings: {e}");
            self.emit(Event::Error(format!("Cannot save the settings: {e}")));
        }
    }

    pub(crate) fn token(&self) -> Option<String> {
        lock(&self.settings).token().map(str::to_string)
    }

    pub(crate) fn clear_token(&self) {
        let mut s = lock(&self.settings);
        if s.token().is_some() {
            s.set_token(None);
            let copy = s.clone();
            drop(s);
            self.save_settings(&copy);
        }
    }

    pub(crate) fn save_login(&self, login: &str, token: String) {
        let mut s = lock(&self.settings);
        s.login = Some(login.to_string());
        s.set_token(Some(token));
        let copy = s.clone();
        drop(s);
        self.save_settings(&copy);
    }

    pub(crate) fn set_mode(&self, mode: Mode) {
        let mut s = lock(&self.settings);
        if s.mode.as_ref() != Some(&mode) {
            s.mode = Some(mode);
            s.set_token(None);
            let copy = s.clone();
            drop(s);
            self.save_settings(&copy);
        }
    }

    pub(crate) fn set_channel(&self, c: Channel) {
        *lock(&self.channel) = Some(c);
    }

    pub(crate) fn channel(&self) -> Option<Channel> {
        lock(&self.channel).clone()
    }

    /// The channel with the session token, if signed in.
    pub(crate) fn authed(&self) -> Option<Authed> {
        let ch = self.channel()?;
        let t = self.token()?;
        guardian_client::with_token(ch, &t).ok()
    }

    pub(crate) fn apply(&self, ev: WatchEvent) -> Fleet {
        let mut f = lock(&self.fleet);
        f.apply(ev);
        f.clone()
    }

    pub(crate) fn reset_fleet(&self) {
        *lock(&self.fleet) = Fleet::default();
    }

    pub(crate) fn bootstrap(&self) -> Option<String> {
        lock(&self.bootstrap).clone()
    }

    pub(crate) fn set_bootstrap(&self, code: Option<String>) {
        *lock(&self.bootstrap) = code;
    }

    pub(crate) fn abort_session(&self) {
        if let Some(h) = lock(&self.session).take() {
            h.abort();
        }
    }

    pub(crate) fn replace_session(&self, h: JoinHandle<()>) {
        if let Some(old) = lock(&self.session).replace(h) {
            old.abort();
        }
    }

    /// Drops the current connection and connects to `server`.
    pub(crate) fn start_session(self: &Arc<Self>, server: String, pin: Option<String>) {
        self.abort_session();
        *lock(&self.channel) = None;
        self.reset_fleet();
        let c = self.clone();
        self.replace_session(tokio::spawn(conn::run_session(c, server, pin)));
    }
}

async fn handle(core: Arc<Core>, a: Action) {
    match a {
        Action::Connect {
            server,
            cert_sha256,
        } => {
            core.set_mode(Mode::Remote {
                server: server.clone(),
                cert_sha256: cert_sha256.clone(),
            });
            core.start_session(server, cert_sha256);
        }
        Action::Login { login, password } => conn::login(core, login, password).await,
        Action::Init {
            bootstrap_code,
            login,
            password,
        } => conn::init(core, bootstrap_code, login, password).await,
        Action::Logout => conn::logout(core).await,
        Action::Shutdown => {}
    }
}

/// Runs `a` so that a panic becomes an error event instead of killing the
/// backend.
async fn guarded(core: Arc<Core>, a: Action) {
    let name = format!("{a:?}")
        .split([' ', '{', '('])
        .next()
        .unwrap_or_default()
        .to_string();
    let r = std::panic::AssertUnwindSafe(handle(core.clone(), a))
        .catch_unwind()
        .await;
    if let Err(p) = r {
        let msg = p
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        tracing::error!("panic in {name}: {msg}");
        core.emit(Event::Error(format!("Internal error in {name}: {msg}")));
    }
}

/// The real backend.
pub struct RealBackend {
    actions: mpsc::UnboundedSender<Action>,
    events: Mutex<std::sync::mpsc::Receiver<Event>>,
    done: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl std::fmt::Debug for RealBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RealBackend")
    }
}

impl RealBackend {
    /// Starts the backend thread. `repaint` is called after every event.
    pub fn start(settings_path: PathBuf, repaint: Repaint) -> RealBackend {
        let (atx, mut arx) = mpsc::unbounded_channel::<Action>();
        let (etx, erx) = std::sync::mpsc::channel();
        let (dtx, drx) = std::sync::mpsc::channel();
        let settings = match ConsoleSettings::load(&settings_path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("{e}; starting with empty settings");
                let _ = etx.send(Event::Error(format!("Settings ignored: {e}")));
                ConsoleSettings::default()
            }
        };
        let spawned = std::thread::Builder::new()
            .name("guardian-console-backend".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = etx.send(Event::Error(format!("Cannot start: {e}")));
                        let _ = dtx.send(());
                        return;
                    }
                };
                let core = Arc::new(Core {
                    events: etx,
                    repaint,
                    settings_path,
                    settings: Mutex::new(settings),
                    channel: Mutex::new(None),
                    fleet: Mutex::new(Fleet::default()),
                    session: Mutex::new(None),
                    bootstrap: Mutex::new(None),
                });
                rt.block_on(async {
                    while let Some(a) = arx.recv().await {
                        if matches!(a, Action::Shutdown) {
                            break;
                        }
                        tokio::spawn(guarded(core.clone(), a));
                    }
                    core.abort_session();
                });
                rt.shutdown_timeout(Duration::from_secs(2));
                let _ = dtx.send(());
            });
        if let Err(e) = spawned {
            tracing::error!("cannot start the backend thread: {e}");
        }
        RealBackend {
            actions: atx,
            events: Mutex::new(erx),
            done: Mutex::new(drx),
        }
    }
}

impl Backend for RealBackend {
    fn send(&self, a: Action) {
        let _ = self.actions.send(a);
    }

    fn poll(&self) -> Vec<Event> {
        lock(&self.events).try_iter().collect()
    }
}

impl Drop for RealBackend {
    fn drop(&mut self) {
        let _ = self.actions.send(Action::Shutdown);
        let _ = lock(&self.done).recv_timeout(Duration::from_secs(5));
    }
}
