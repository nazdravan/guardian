#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use guardian_agent::engine::{Engine, EngineConfig, EngineMode, Policy};
use guardian_core::enforce::{Os, ProcessInfo};
use guardian_core::policy::{AppName, Mode, Profile, ProfileId};
use guardian_core::trusted::TrustedRoots;
use guardian_platform::fake::FakePlatform;

fn cfg() -> EngineConfig {
    EngineConfig {
        reconcile_every: Duration::from_millis(200),
        fallback_poll: Duration::from_millis(50),
        retry_events_every: Duration::from_millis(300),
    }
}

fn policy(mode: Mode, apps: &[&str], version: u64) -> Policy {
    Policy {
        profile: Profile {
            id: ProfileId(1),
            name: "t".into(),
            mode,
            apps: apps
                .iter()
                .map(|a| AppName::parse(a).unwrap())
                .collect::<BTreeSet<_>>(),
        },
        version,
    }
}

fn fake() -> Arc<FakePlatform> {
    let f = FakePlatform::new(Os::Windows);
    f.set_roots(TrustedRoots {
        recursive: vec![r"C:\Windows\System32".into()],
        ..TrustedRoots::none()
    });
    f
}

async fn until(within: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    f()
}

fn wl(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("whitelist.txt")
}

#[tokio::test]
async fn event_triggers_kill_in_blacklist() {
    let f = fake();
    let d = tempfile::tempdir().unwrap();
    let e = Engine::spawn(
        f.clone(),
        policy(Mode::Blacklist, &["game.exe"], 1),
        wl(&d),
        cfg(),
    );
    assert!(
        until(Duration::from_millis(500), || e.mode()
            == EngineMode::Events)
        .await
    );
    let game = f.spawn("game.exe", r"C:\g\game.exe");
    let note = f.spawn("notepad.exe", r"C:\Windows\System32\notepad.exe");
    assert!(until(Duration::from_millis(200), || !f.is_running(game)).await);
    assert!(f.is_running(note));
    e.shutdown().await;
}

#[tokio::test]
async fn reconcile_kills_processes_started_before() {
    let f = fake();
    let d = tempfile::tempdir().unwrap();
    let game = f.spawn("game.exe", r"C:\g\game.exe");
    let e = Engine::spawn(
        f.clone(),
        policy(Mode::Blacklist, &["game.exe"], 1),
        wl(&d),
        cfg(),
    );
    assert!(until(Duration::from_millis(300), || !f.is_running(game)).await);
    e.shutdown().await;
}

#[tokio::test]
async fn set_policy_reapplies_immediately() {
    let f = fake();
    let d = tempfile::tempdir().unwrap();
    let game = f.spawn("game.exe", r"C:\g\game.exe");
    let real_svc = f.spawn("svchost.exe", r"C:\Windows\System32\svchost.exe");
    let fake_svc = f.spawn("svchost.exe", r"C:\Users\elev\Downloads\svchost.exe");
    let e = Engine::spawn(f.clone(), policy(Mode::Free, &[], 1), wl(&d), cfg());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(f.kills().is_empty());
    e.set_policy(policy(Mode::Whitelist, &[], 2));
    assert!(
        until(Duration::from_millis(100), || !f.is_running(game)
            && !f.is_running(fake_svc))
        .await
    );
    assert!(f.is_running(real_svc));
    e.shutdown().await;
}

#[tokio::test]
async fn falls_back_to_polling_and_recovers() {
    let f = fake();
    f.fail_events(true);
    let d = tempfile::tempdir().unwrap();
    let e = Engine::spawn(
        f.clone(),
        policy(Mode::Blacklist, &["game.exe"], 1),
        wl(&d),
        cfg(),
    );
    assert!(
        until(Duration::from_millis(200), || e.mode()
            == EngineMode::Polling)
        .await
    );
    let game = f.spawn("game.exe", r"C:\g\game.exe");
    assert!(
        until(Duration::from_millis(150), || !f.is_running(game)).await,
        "killed by polling"
    );
    f.fail_events(false);
    assert!(
        until(Duration::from_millis(500), || e.mode()
            == EngineMode::Events)
        .await
    );
    e.shutdown().await;
}

#[tokio::test]
async fn source_dying_midway_switches_to_polling() {
    let f = fake();
    let d = tempfile::tempdir().unwrap();
    let e = Engine::spawn(
        f.clone(),
        policy(Mode::Blacklist, &["game.exe"], 1),
        wl(&d),
        cfg(),
    );
    assert!(
        until(Duration::from_millis(500), || e.mode()
            == EngineMode::Events)
        .await
    );
    f.fail_events(true);
    assert!(
        until(Duration::from_millis(200), || e.mode()
            == EngineMode::Polling)
        .await
    );
    let game = f.spawn("game.exe", r"C:\g\game.exe");
    assert!(until(Duration::from_millis(150), || !f.is_running(game)).await);
    e.shutdown().await;
}

#[tokio::test]
async fn lost_events_trigger_a_scan() {
    let f = fake();
    let d = tempfile::tempdir().unwrap();
    let e = Engine::spawn(
        f.clone(),
        policy(Mode::Blacklist, &["game.exe"], 1),
        wl(&d),
        EngineConfig {
            reconcile_every: Duration::from_secs(60),
            ..cfg()
        },
    );
    assert!(
        until(Duration::from_millis(500), || e.mode()
            == EngineMode::Events)
        .await
    );
    // A process whose start event was lost.
    f.insert(ProcessInfo {
        pid: 77,
        name: "game.exe".into(),
        exe_path: Some(r"C:\g\game.exe".into()),
        start_marker: 5,
    });
    f.lose_events();
    assert!(until(Duration::from_millis(200), || !f.is_running(77)).await);
    assert_eq!(e.mode(), EngineMode::Events);
    e.shutdown().await;
}

#[tokio::test]
async fn local_whitelist_counts_in_exam_and_reloads() {
    let f = fake();
    let d = tempfile::tempdir().unwrap();
    let e = Engine::spawn(f.clone(), policy(Mode::Whitelist, &[], 1), wl(&d), cfg());
    tokio::time::sleep(Duration::from_millis(50)).await;
    std::fs::write(wl(&d), "printer-tool.exe\n").unwrap();
    tokio::time::sleep(Duration::from_millis(350)).await; // one reconcile picks it up
    let tool = f.spawn("printer-tool.exe", r"C:\Tools\printer-tool.exe");
    let game = f.spawn("game.exe", r"C:\g\game.exe");
    assert!(until(Duration::from_millis(200), || !f.is_running(game)).await);
    assert!(f.is_running(tool));
    e.shutdown().await;
}

#[tokio::test]
async fn unsupported_platform_stays_quiet() {
    let p = Arc::new(guardian_platform_unsupported());
    let d = tempfile::tempdir().unwrap();
    let e = Engine::spawn(p, policy(Mode::Whitelist, &[], 1), wl(&d), cfg());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(e.stats().reconciles <= 1, "{:?}", e.stats());
    e.shutdown().await;
}

/// A platform whose every call is `Unsupported`.
fn guardian_platform_unsupported() -> UnsupportedPlatform {
    UnsupportedPlatform
}

struct UnsupportedPlatform;

impl guardian_platform::Platform for UnsupportedPlatform {
    fn os(&self) -> Os {
        Os::Linux
    }
    fn list_processes(&self) -> Result<Vec<ProcessInfo>, guardian_platform::PlatformError> {
        Err(guardian_platform::PlatformError::Unsupported)
    }
    fn process_info(&self, _: u32) -> Result<ProcessInfo, guardian_platform::PlatformError> {
        Err(guardian_platform::PlatformError::Unsupported)
    }
    fn kill(&self, _: &ProcessInfo) -> Result<(), guardian_platform::PlatformError> {
        Err(guardian_platform::PlatformError::Unsupported)
    }
    fn process_events(
        &self,
    ) -> Result<Box<dyn guardian_platform::ProcessEventSource>, guardian_platform::PlatformError>
    {
        Err(guardian_platform::PlatformError::Unsupported)
    }
    fn trusted_roots(&self) -> TrustedRoots {
        TrustedRoots::none()
    }
    fn system_info(&self, v: &str) -> guardian_proto::v1::SystemInfo {
        guardian_proto::v1::SystemInfo {
            agent_version: v.into(),
            ..Default::default()
        }
    }
    fn active_session(
        &self,
    ) -> Result<Option<guardian_platform::UserSession>, guardian_platform::PlatformError> {
        Err(guardian_platform::PlatformError::Unsupported)
    }
    fn spawn_in_session(
        &self,
        _: &guardian_platform::UserSession,
        _: &std::path::Path,
        _: &[String],
    ) -> Result<u32, guardian_platform::PlatformError> {
        Err(guardian_platform::PlatformError::Unsupported)
    }
}

#[tokio::test]
async fn never_kills_self_or_worker() {
    let f = fake();
    let d = tempfile::tempdir().unwrap();
    f.insert(ProcessInfo {
        pid: std::process::id(),
        name: "game.exe".into(),
        exe_path: Some(r"C:\g\game.exe".into()),
        start_marker: 1,
    });
    f.insert(ProcessInfo {
        pid: 4242,
        name: "game.exe".into(),
        exe_path: Some(r"C:\g\game.exe".into()),
        start_marker: 1,
    });
    let e = Engine::spawn(f.clone(), policy(Mode::Free, &[], 1), wl(&d), cfg());
    e.set_worker_pid(Some(4242));
    e.set_policy(policy(Mode::Blacklist, &["game.exe"], 2));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(f.kills().is_empty(), "{:?}", f.kills());
    e.shutdown().await;
}

#[test]
fn policy_from_state_picks_effective_or_default() {
    use guardian_proto::v1 as pb;
    let st = pb::AgentState {
        version: 3,
        effective_profile_id: 2,
        room_id: 0,
        profiles: vec![
            pb::Profile {
                id: 1,
                name: "Liber".into(),
                mode: pb::Mode::Free as i32,
                apps: vec![],
            },
            pb::Profile {
                id: 2,
                name: "Examen".into(),
                mode: pb::Mode::Whitelist as i32,
                apps: vec!["word.exe".into()],
            },
        ],
    };
    let p = Policy::from_state(&st);
    assert_eq!(
        (p.profile.name.as_str(), p.profile.mode, p.version),
        ("Examen", Mode::Whitelist, 3)
    );
    let missing = pb::AgentState {
        effective_profile_id: 9,
        ..st
    };
    let p = Policy::from_state(&missing);
    assert_eq!(
        (p.profile.name.as_str(), p.profile.mode),
        ("Lecție", Mode::Blacklist)
    );
    assert!(p.profile.apps.is_empty());
}
