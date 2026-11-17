#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use guardian_core::enforce::ProcessInfo;
use guardian_platform::{Platform, PlatformError};

fn wait_exit(child: &mut std::process::Child, within: Duration) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn spawn_game(dir: &std::path::Path) -> std::process::Child {
    let exe = dir.join("guardian-test-game");
    std::fs::copy("/bin/sleep", &exe).unwrap();
    Command::new(&exe)
        .arg("30")
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}

fn find(p: &dyn Platform, pid: u32) -> ProcessInfo {
    p.list_processes()
        .unwrap()
        .into_iter()
        .find(|x| x.pid == pid)
        .unwrap()
}

#[test]
fn lists_this_process() {
    let p = guardian_platform::current();
    let me = find(&*p, std::process::id());
    assert_eq!(me.exe_path.unwrap(), std::env::current_exe().unwrap());
    assert!(me.start_marker > 0);
}

#[test]
fn kills_a_spawned_process_by_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_game(dir.path());
    std::thread::sleep(Duration::from_millis(100));
    let p = guardian_platform::current();
    let info = find(&*p, child.id());
    // The name is the executable's file name, not the 15-character `comm`.
    assert_eq!(info.name, "guardian-test-game");
    assert_eq!(
        info.exe_path.as_deref(),
        Some(dir.path().join("guardian-test-game").as_path())
    );
    p.kill(&info).unwrap();
    assert!(wait_exit(&mut child, Duration::from_secs(2)));
}

#[test]
fn refuses_to_kill_on_stale_start_marker() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_game(dir.path());
    std::thread::sleep(Duration::from_millis(100));
    let p = guardian_platform::current();
    let info = find(&*p, child.id());
    let stale = ProcessInfo {
        start_marker: info.start_marker + 1,
        ..info.clone()
    };
    assert!(matches!(p.kill(&stale), Err(PlatformError::Changed)));
    assert!(child.try_wait().unwrap().is_none(), "still running");
    p.kill(&info).unwrap();
    assert!(wait_exit(&mut child, Duration::from_secs(2)));
}

#[test]
fn system_info_has_hostname_and_os() {
    let i = guardian_platform::current().system_info("3.0.0");
    assert_eq!(i.os, "linux");
    assert!(!i.hostname.is_empty());
    assert!(
        i.lan_addresses
            .iter()
            .all(|a| !a.starts_with("127.") && a != "::1")
    );
}

#[test]
fn trusted_roots_cover_usr_not_home() {
    use guardian_core::enforce::Os;
    let r = guardian_platform::current().trusted_roots();
    assert!(r.contains(Os::Linux, std::path::Path::new("/usr/bin/bash")));
    assert!(!r.contains(Os::Linux, std::path::Path::new("/home/elev/bash")));
    assert!(!r.contains(Os::Linux, std::path::Path::new("/tmp/bash")));
}
