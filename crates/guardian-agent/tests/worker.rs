#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use guardian_agent::engine::{Engine, EngineConfig, Policy};
use guardian_agent::worker::{WorkerManager, WorkerOptions};
use guardian_core::enforce::Os;
use guardian_platform::UserSession;
use guardian_platform::fake::FakePlatform;
use guardian_platform::ipc::IpcEndpoint;

async fn until(within: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f()
}

fn session() -> UserSession {
    UserSession {
        id: "2".into(),
        user: "elev".into(),
        uid: Some(1000),
        env: vec![],
    }
}

struct Setup {
    _d: tempfile::TempDir,
    f: Arc<FakePlatform>,
    m: WorkerManager,
    workers: Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

fn setup(spawn_pid: u32, with_session: bool) -> Setup {
    let d = tempfile::tempdir().unwrap();
    let ep = IpcEndpoint::Unix(d.path().join("w.sock"));
    let f = FakePlatform::new(Os::Linux);
    f.set_spawn_pid(spawn_pid);
    if with_session {
        f.set_session(Some(session()));
    }
    let workers: Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>> = Arc::default();
    let w2 = workers.clone();
    let rt = tokio::runtime::Handle::current();
    f.on_spawn(Box::new(move |_s, _exe, args| {
        let ep = IpcEndpoint::parse(&args[1]);
        let h = rt.spawn(async move {
            let _ = guardian_worker::run(ep, Duration::from_secs(5)).await;
        });
        w2.lock().unwrap().push(h);
    }));
    let engine = Engine::spawn(
        f.clone(),
        Policy::default_lesson(),
        d.path().join("wl.txt"),
        EngineConfig::default(),
    );
    let m = WorkerManager::spawn(
        f.clone(),
        WorkerOptions {
            exe: "/opt/guardian-agent/guardian-worker".into(),
            endpoint: ep,
            check_every: Duration::from_millis(100),
            ping_every: Duration::from_millis(200),
        },
        engine,
    );
    Setup {
        _d: d,
        f,
        m,
        workers,
    }
}

#[tokio::test]
async fn manager_spawns_worker_in_session_and_accepts_it() {
    let s = setup(std::process::id(), true);
    assert!(until(Duration::from_secs(2), || s.m.connected()).await);
    assert_eq!(s.m.pid(), Some(std::process::id()));
    assert_eq!(s.f.spawned_in_session()[0].0, "2");
    s.m.shutdown().await;
}

#[tokio::test]
async fn foreign_peer_is_rejected() {
    let s = setup(999_999, true);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!s.m.connected());
    s.m.shutdown().await;
}

#[tokio::test]
async fn no_session_means_no_worker() {
    let s = setup(std::process::id(), false);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(s.f.spawned_in_session().is_empty());
    assert!(!s.m.connected());
    s.m.shutdown().await;
}

#[tokio::test]
async fn dead_worker_is_restarted() {
    let s = setup(std::process::id(), true);
    assert!(until(Duration::from_secs(2), || s.m.connected()).await);
    for h in s.workers.lock().unwrap().drain(..) {
        h.abort();
    }
    assert!(
        until(Duration::from_secs(2), || s.f.spawned_in_session().len()
            >= 2)
        .await
    );
    assert!(until(Duration::from_secs(2), || s.m.connected()).await);
    s.m.shutdown().await;
}

#[tokio::test]
async fn logout_closes_the_worker() {
    let s = setup(std::process::id(), true);
    assert!(until(Duration::from_secs(2), || s.m.connected()).await);
    s.f.set_session(None);
    assert!(until(Duration::from_secs(2), || !s.m.connected()).await);
    s.m.shutdown().await;
}
