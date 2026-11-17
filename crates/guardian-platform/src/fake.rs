//! An in-memory platform for tests (feature `testing`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use guardian_core::enforce::{Os, ProcessInfo};
use guardian_core::trusted::TrustedRoots;
use guardian_proto::v1::SystemInfo;
use tokio::sync::mpsc;

use crate::{Platform, PlatformError, ProcessEventSource, ProcessStart, UserSession};

type Event = Result<ProcessStart, PlatformError>;
/// Called by `spawn_in_session` (lets tests start an in-process worker).
pub type SpawnHook = Box<dyn Fn(&UserSession, &Path, &[String]) + Send + Sync>;

#[derive(Default)]
struct Inner {
    procs: BTreeMap<u32, ProcessInfo>,
    next_pid: u32,
    next_marker: u64,
    kills: Vec<ProcessInfo>,
    fail_events: bool,
    sources: Vec<mpsc::UnboundedSender<Event>>,
    session: Option<UserSession>,
    spawned: Vec<(String, PathBuf)>,
    roots: TrustedRoots,
    spawn_pid: Option<u32>,
    on_spawn: Option<Arc<SpawnHook>>,
}

/// A fake OS: processes are entries in a map, events go through channels.
pub struct FakePlatform {
    os: Os,
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for FakePlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakePlatform")
            .field("os", &self.os)
            .finish()
    }
}

struct FakeEvents(mpsc::UnboundedReceiver<Event>);

#[async_trait::async_trait]
impl ProcessEventSource for FakeEvents {
    async fn next(&mut self) -> Result<ProcessStart, PlatformError> {
        self.0
            .recv()
            .await
            .unwrap_or_else(|| Err(PlatformError::Os("fake event source closed".into())))
    }
}

impl FakePlatform {
    /// An empty fake for `os`.
    pub fn new(os: Os) -> Arc<FakePlatform> {
        Arc::new(FakePlatform {
            os,
            inner: Mutex::new(Inner {
                next_pid: 1000,
                next_marker: 1,
                ..Inner::default()
            }),
        })
    }

    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        f(&mut g)
    }

    /// Starts a fake process and announces it to the event sources.
    pub fn spawn(&self, name: &str, exe: &str) -> u32 {
        self.with(|i| {
            let pid = i.next_pid;
            i.next_pid += 1;
            let marker = i.next_marker;
            i.next_marker += 1;
            i.procs.insert(
                pid,
                ProcessInfo {
                    pid,
                    name: name.into(),
                    exe_path: Some(exe.into()),
                    start_marker: marker,
                },
            );
            i.sources
                .retain(|s| s.send(Ok(ProcessStart { pid })).is_ok());
            pid
        })
    }

    /// Adds a process with a chosen PID without announcing it.
    pub fn insert(&self, p: ProcessInfo) {
        self.with(|i| {
            i.procs.insert(p.pid, p);
        });
    }

    /// Ends a process without killing it.
    pub fn remove(&self, pid: u32) {
        self.with(|i| {
            i.procs.remove(&pid);
        });
    }

    /// Whether `pid` still runs.
    pub fn is_running(&self, pid: u32) -> bool {
        self.with(|i| i.procs.contains_key(&pid))
    }

    /// Processes killed so far.
    pub fn kills(&self) -> Vec<ProcessInfo> {
        self.with(|i| i.kills.clone())
    }

    /// Makes event sources fail (existing ones get an error, new ones cannot start).
    pub fn fail_events(&self, on: bool) {
        self.with(|i| {
            i.fail_events = on;
            if on {
                for s in i.sources.drain(..) {
                    let _ = s.send(Err(PlatformError::Os("fake event source failed".into())));
                }
            }
        });
    }

    /// Sends "events were lost" to the event sources.
    pub fn lose_events(&self) {
        self.with(|i| {
            i.sources
                .retain(|s| s.send(Err(PlatformError::Changed)).is_ok())
        });
    }

    /// Sets the active session.
    pub fn set_session(&self, s: Option<UserSession>) {
        self.with(|i| i.session = s);
    }

    /// `(session id, exe)` of every `spawn_in_session` call.
    pub fn spawned_in_session(&self) -> Vec<(String, PathBuf)> {
        self.with(|i| i.spawned.clone())
    }

    /// The trusted roots to report.
    pub fn set_roots(&self, r: TrustedRoots) {
        self.with(|i| i.roots = r);
    }

    /// The PID `spawn_in_session` reports.
    pub fn set_spawn_pid(&self, pid: u32) {
        self.with(|i| i.spawn_pid = Some(pid));
    }

    /// Runs `hook` on every `spawn_in_session`.
    pub fn on_spawn(&self, hook: SpawnHook) {
        self.with(|i| i.on_spawn = Some(Arc::new(hook)));
    }
}

impl Platform for FakePlatform {
    fn os(&self) -> Os {
        self.os
    }

    fn list_processes(&self) -> Result<Vec<ProcessInfo>, PlatformError> {
        Ok(self.with(|i| i.procs.values().cloned().collect()))
    }

    fn process_info(&self, pid: u32) -> Result<ProcessInfo, PlatformError> {
        self.with(|i| i.procs.get(&pid).cloned())
            .ok_or(PlatformError::NotFound)
    }

    fn kill(&self, p: &ProcessInfo) -> Result<(), PlatformError> {
        self.with(|i| match i.procs.get(&p.pid) {
            None => Err(PlatformError::NotFound),
            Some(cur)
                if cur.name != p.name
                    || cur.exe_path != p.exe_path
                    || cur.start_marker != p.start_marker =>
            {
                Err(PlatformError::Changed)
            }
            Some(_) => {
                if let Some(k) = i.procs.remove(&p.pid) {
                    i.kills.push(k);
                }
                Ok(())
            }
        })
    }

    fn process_events(&self) -> Result<Box<dyn ProcessEventSource>, PlatformError> {
        self.with(|i| {
            if i.fail_events {
                return Err(PlatformError::Os("fake event source unavailable".into()));
            }
            let (tx, rx) = mpsc::unbounded_channel();
            i.sources.push(tx);
            Ok(Box::new(FakeEvents(rx)) as Box<dyn ProcessEventSource>)
        })
    }

    fn trusted_roots(&self) -> TrustedRoots {
        self.with(|i| i.roots.clone())
    }

    fn system_info(&self, agent_version: &str) -> SystemInfo {
        SystemInfo {
            hostname: "fake-pc".into(),
            os: match self.os {
                Os::Windows => "windows".into(),
                Os::Linux => "linux".into(),
            },
            agent_version: agent_version.into(),
            lan_addresses: vec!["10.0.0.9".into()],
        }
    }

    fn active_session(&self) -> Result<Option<UserSession>, PlatformError> {
        Ok(self.with(|i| i.session.clone()))
    }

    fn spawn_in_session(
        &self,
        s: &UserSession,
        exe: &Path,
        args: &[String],
    ) -> Result<u32, PlatformError> {
        let (pid, hook) = self.with(|i| {
            i.spawned.push((s.id.clone(), exe.to_path_buf()));
            let pid = i.spawn_pid.unwrap_or_else(|| {
                let p = i.next_pid;
                i.next_pid += 1;
                p
            });
            (pid, i.on_spawn.clone())
        });
        if let Some(h) = hook {
            h(s, exe, args);
        }
        Ok(pid)
    }
}
