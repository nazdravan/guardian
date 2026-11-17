//! Applies the current profile to running processes.
//!
//! Process starts come from the platform's event source; a full scan runs at
//! start, on every new policy, when the local whitelist changes and every
//! `reconcile_every`. If the event source cannot start or dies, the engine scans
//! every `fallback_poll` and retries the source every `retry_events_every`.
//! The kill/allow decision itself is `guardian_core::enforce::decide`.

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use guardian_core::enforce::{Os, ProcessInfo, Verdict, decide};
use guardian_core::policy::{AppName, Mode, Profile, ProfileId};
use guardian_core::trusted::TrustedRoots;
use guardian_platform::{Platform, PlatformError, ProcessEventSource};
use guardian_proto::v1::AgentState;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::store::{DEFAULT_PROFILE, load_whitelist};

/// The profile to enforce and the state version it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// Effective profile.
    pub profile: Profile,
    /// `AgentState.version`.
    pub version: u64,
}

impl Policy {
    /// "Lecție", blacklist, no apps.
    pub fn default_lesson() -> Policy {
        Policy {
            profile: Profile {
                id: ProfileId(0),
                name: DEFAULT_PROFILE.into(),
                mode: Mode::Blacklist,
                apps: BTreeSet::new(),
            },
            version: 0,
        }
    }

    /// The effective profile of `st`; if it is missing or invalid, the default
    /// lesson (with a warning).
    pub fn from_state(st: &AgentState) -> Policy {
        let found = st
            .profiles
            .iter()
            .find(|p| p.id == st.effective_profile_id)
            .map(Profile::try_from);
        match found {
            Some(Ok(profile)) => Policy {
                profile,
                version: st.version,
            },
            Some(Err(e)) => {
                tracing::warn!(
                    "effective profile {} is invalid ({e}); using the default",
                    st.effective_profile_id
                );
                Policy {
                    version: st.version,
                    ..Policy::default_lesson()
                }
            }
            None => {
                if st.effective_profile_id != 0 {
                    tracing::warn!(
                        "effective profile {} not in state; using the default",
                        st.effective_profile_id
                    );
                }
                Policy {
                    version: st.version,
                    ..Policy::default_lesson()
                }
            }
        }
    }
}

/// Intervals.
#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    /// Full scan period (10 s).
    pub reconcile_every: Duration,
    /// Scan period without an event source (1 s).
    pub fallback_poll: Duration,
    /// How often a failed event source is retried (60 s).
    pub retry_events_every: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            reconcile_every: Duration::from_secs(10),
            fallback_poll: Duration::from_secs(1),
            retry_events_every: Duration::from_secs(60),
        }
    }
}

/// How process starts are noticed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineMode {
    /// From OS notifications.
    Events,
    /// By scanning every `fallback_poll`.
    Polling,
}

/// Counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineStats {
    /// Processes killed.
    pub kills: u64,
    /// Full scans done.
    pub reconciles: u64,
}

enum Cmd {
    Policy(Policy),
    Stop,
}

struct Shared {
    mode: AtomicU8,
    kills: AtomicU64,
    reconciles: AtomicU64,
    worker_pid: Mutex<Option<u32>>,
    denied: Mutex<HashSet<(String, Option<PathBuf>)>>,
}

/// Handle to the enforcement task. Cheap to clone.
#[derive(Clone)]
pub struct Engine {
    shared: Arc<Shared>,
    tx: mpsc::UnboundedSender<Cmd>,
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("mode", &self.mode())
            .field("stats", &self.stats())
            .finish()
    }
}

/// What a judging pass needs (moved into blocking tasks).
#[derive(Clone)]
struct Judge {
    platform: Arc<dyn Platform>,
    os: Os,
    roots: TrustedRoots,
    policy: Policy,
    whitelist: BTreeSet<AppName>,
    shared: Arc<Shared>,
    self_pid: u32,
}

impl Judge {
    fn spared(&self, pid: u32) -> bool {
        let worker = self.shared.worker_pid.lock().ok().and_then(|g| *g);
        pid == self.self_pid || Some(pid) == worker
    }

    fn judge(&self, p: &ProcessInfo) {
        if self.spared(p.pid) {
            return;
        }
        if decide(
            self.os,
            &self.policy.profile,
            &self.whitelist,
            &self.roots,
            p,
        ) != Verdict::Kill
        {
            return;
        }
        match self.platform.kill(p) {
            Ok(()) => {
                self.shared.kills.fetch_add(1, Ordering::Relaxed);
                tracing::info!(
                    "killed {} (pid {}) under profile {:?}",
                    p.name,
                    p.pid,
                    self.policy.profile.name
                );
            }
            Err(PlatformError::Changed | PlatformError::NotFound) => {
                tracing::debug!("pid {} changed or exited before it was killed", p.pid);
            }
            Err(PlatformError::AccessDenied) => {
                let key = (p.name.clone(), p.exe_path.clone());
                let first = self.shared.denied.lock().is_ok_and(|mut d| d.insert(key));
                if first {
                    tracing::warn!("cannot kill {} (pid {}): access denied", p.name, p.pid);
                }
            }
            Err(e) => tracing::warn!("cannot kill {} (pid {}): {e}", p.name, p.pid),
        }
    }

    /// Full scan; `Err(Unsupported)` on a platform without process control.
    fn reconcile(&self) -> Result<(), PlatformError> {
        let list = self.platform.list_processes()?;
        self.shared.reconciles.fetch_add(1, Ordering::Relaxed);
        for p in &list {
            self.judge(p);
        }
        Ok(())
    }

    fn one(&self, pid: u32) {
        if let Ok(p) = self.platform.process_info(pid) {
            self.judge(&p);
        }
    }
}

const MODE_EVENTS: u8 = 0;
const MODE_POLLING: u8 = 1;

impl Engine {
    /// Starts enforcing `policy` on the current tokio runtime.
    pub fn spawn(
        platform: Arc<dyn Platform>,
        policy: Policy,
        whitelist_path: PathBuf,
        cfg: EngineConfig,
    ) -> Engine {
        let shared = Arc::new(Shared {
            mode: AtomicU8::new(MODE_POLLING),
            kills: AtomicU64::new(0),
            reconciles: AtomicU64::new(0),
            worker_pid: Mutex::new(None),
            denied: Mutex::new(HashSet::new()),
        });
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run(
            platform,
            policy,
            whitelist_path,
            cfg,
            shared.clone(),
            rx,
        ));
        Engine {
            shared,
            tx,
            task: Arc::new(Mutex::new(Some(task))),
        }
    }

    /// Switches profile and rescans at once.
    pub fn set_policy(&self, p: Policy) {
        let _ = self.tx.send(Cmd::Policy(p));
    }

    /// The worker's PID, which is never killed.
    pub fn set_worker_pid(&self, pid: Option<u32>) {
        if let Ok(mut g) = self.shared.worker_pid.lock() {
            *g = pid;
        }
    }

    /// Events or polling.
    pub fn mode(&self) -> EngineMode {
        if self.shared.mode.load(Ordering::Relaxed) == MODE_EVENTS {
            EngineMode::Events
        } else {
            EngineMode::Polling
        }
    }

    /// Counters.
    pub fn stats(&self) -> EngineStats {
        EngineStats {
            kills: self.shared.kills.load(Ordering::Relaxed),
            reconciles: self.shared.reconciles.load(Ordering::Relaxed),
        }
    }

    /// Stops the task and waits for it.
    pub async fn shutdown(self) {
        let _ = self.tx.send(Cmd::Stop);
        let task = self.task.lock().ok().and_then(|mut g| g.take());
        if let Some(t) = task {
            let _ = t.await;
        }
    }
}

async fn next_event(
    src: &mut Option<Box<dyn ProcessEventSource>>,
) -> Option<Result<u32, PlatformError>> {
    match src {
        Some(s) => Some(s.next().await.map(|e| e.pid)),
        None => std::future::pending().await,
    }
}

fn mtime(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

async fn blocking<F: FnOnce() -> R + Send + 'static, R: Send + 'static>(f: F) -> Option<R> {
    tokio::task::spawn_blocking(f).await.ok()
}

async fn run(
    platform: Arc<dyn Platform>,
    policy: Policy,
    whitelist_path: PathBuf,
    cfg: EngineConfig,
    shared: Arc<Shared>,
    mut rx: mpsc::UnboundedReceiver<Cmd>,
) {
    let mut judge = Judge {
        os: platform.os(),
        roots: platform.trusted_roots(),
        platform: platform.clone(),
        policy,
        whitelist: load_whitelist(&whitelist_path),
        shared: shared.clone(),
        self_pid: std::process::id(),
    };
    let mut wl_mtime = mtime(&whitelist_path);

    // Returns false when the platform has no process control at all.
    let reconcile = |j: Judge| async move {
        match blocking(move || j.reconcile()).await {
            Some(Err(PlatformError::Unsupported)) => false,
            Some(Err(e)) => {
                tracing::warn!("process scan failed: {e}");
                true
            }
            _ => true,
        }
    };
    if !reconcile(judge.clone()).await {
        tracing::warn!("process control is not supported on this OS; enforcement is inactive");
        while let Some(cmd) = rx.recv().await {
            if matches!(cmd, Cmd::Stop) {
                return;
            }
        }
        return;
    }

    let start_source = |shared: &Shared| -> Option<Box<dyn ProcessEventSource>> {
        match platform.process_events() {
            Ok(s) => {
                shared.mode.store(MODE_EVENTS, Ordering::Relaxed);
                Some(s)
            }
            Err(e) => {
                shared.mode.store(MODE_POLLING, Ordering::Relaxed);
                tracing::warn!(
                    "process events unavailable ({e}); scanning every {:?}",
                    cfg.fallback_poll
                );
                None
            }
        }
    };
    let mut source = start_source(&shared);
    if source.is_some() {
        tracing::info!("process events active");
    }

    let mut reconcile_tick = tokio::time::interval(cfg.reconcile_every);
    let mut poll_tick = tokio::time::interval(cfg.fallback_poll);
    let mut retry_tick = tokio::time::interval(cfg.retry_events_every);
    for t in [&mut reconcile_tick, &mut poll_tick, &mut retry_tick] {
        t.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        t.reset();
    }

    loop {
        tokio::select! {
            cmd = rx.recv() => match cmd {
                Some(Cmd::Policy(p)) => {
                    judge.policy = p;
                    reconcile(judge.clone()).await;
                }
                Some(Cmd::Stop) | None => return,
            },
            ev = next_event(&mut source) => match ev {
                Some(Ok(pid)) => {
                    let j = judge.clone();
                    blocking(move || j.one(pid)).await;
                }
                Some(Err(PlatformError::Changed)) => {
                    tracing::debug!("process events were lost; scanning");
                    reconcile(judge.clone()).await;
                }
                Some(Err(e)) => {
                    source = None;
                    shared.mode.store(MODE_POLLING, Ordering::Relaxed);
                    tracing::warn!("process events stopped ({e}); scanning every {:?}", cfg.fallback_poll);
                    retry_tick.reset();
                    reconcile(judge.clone()).await;
                }
                None => {}
            },
            _ = poll_tick.tick(), if source.is_none() => {
                reconcile(judge.clone()).await;
            }
            _ = retry_tick.tick(), if source.is_none() => {
                source = start_source(&shared);
                if source.is_some() {
                    tracing::warn!("process events active again");
                }
            }
            _ = reconcile_tick.tick() => {
                let now = mtime(&whitelist_path);
                if now != wl_mtime {
                    wl_mtime = now;
                    judge.whitelist = load_whitelist(&whitelist_path);
                    tracing::info!("local whitelist reloaded ({} entries)", judge.whitelist.len());
                }
                reconcile(judge.clone()).await;
            }
        }
    }
}
