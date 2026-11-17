#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use guardian_core::enforce::{Os, ProcessInfo};
use guardian_platform::fake::FakePlatform;
use guardian_platform::{Platform, PlatformError};

#[test]
fn fake_kill_checks_identity() {
    let f = FakePlatform::new(Os::Windows);
    let pid = f.spawn("game.exe", r"C:\g\game.exe");
    let info = f.process_info(pid).unwrap();
    let stale = ProcessInfo {
        start_marker: info.start_marker + 1,
        ..info.clone()
    };
    assert!(matches!(f.kill(&stale), Err(PlatformError::Changed)));
    assert!(f.is_running(pid));
    f.kill(&info).unwrap();
    assert!(!f.is_running(pid));
    assert_eq!(f.kills().len(), 1);
    assert!(matches!(f.kill(&info), Err(PlatformError::NotFound)));
}

#[tokio::test]
async fn fake_events_report_spawns() {
    let f = FakePlatform::new(Os::Linux);
    let mut src = f.process_events().unwrap();
    let pid = f.spawn("game", "/home/elev/game");
    let ev = tokio::time::timeout(Duration::from_secs(1), src.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ev.pid, pid);
}

#[tokio::test]
async fn fake_events_can_fail() {
    let f = FakePlatform::new(Os::Linux);
    let mut src = f.process_events().unwrap();
    f.fail_events(true);
    assert!(f.process_events().is_err());
    assert!(
        tokio::time::timeout(Duration::from_secs(1), src.next())
            .await
            .unwrap()
            .is_err()
    );
    f.fail_events(false);
    assert!(f.process_events().is_ok());
}

#[test]
fn current_platform_reports_its_os() {
    let p = guardian_platform::current();
    #[cfg(target_os = "linux")]
    assert_eq!(p.os(), Os::Linux);
    #[cfg(windows)]
    assert_eq!(p.os(), Os::Windows);
    #[cfg(not(any(target_os = "linux", windows)))]
    assert!(matches!(
        p.list_processes(),
        Err(PlatformError::Unsupported)
    ));
}
