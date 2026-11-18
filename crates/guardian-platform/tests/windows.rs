#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use guardian_core::enforce::{Os, ProcessInfo};
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

fn spawn_game(dir: &Path) -> std::process::Child {
    let exe = dir.join("guardian-test-game.exe");
    std::fs::copy(r"C:\Windows\System32\PING.EXE", &exe).unwrap();
    Command::new(&exe)
        .args(["-n", "30", "127.0.0.1"])
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
    let exe = me.exe_path.unwrap();
    assert_eq!(
        exe.to_string_lossy().to_lowercase(),
        std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .to_lowercase()
    );
    assert!(me.start_marker > 0);
}

#[test]
fn kills_a_spawned_process_by_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_game(dir.path());
    std::thread::sleep(Duration::from_millis(200));
    let p = guardian_platform::current();
    let info = find(&*p, child.id());
    assert_eq!(info.name.to_lowercase(), "guardian-test-game.exe");
    p.kill(&info).unwrap();
    assert!(wait_exit(&mut child, Duration::from_secs(2)));
}

#[test]
fn refuses_to_kill_on_stale_start_marker() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_game(dir.path());
    std::thread::sleep(Duration::from_millis(200));
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
fn trusted_roots_cover_system32_but_not_temp() {
    let r = guardian_platform::current().trusted_roots();
    assert!(r.contains(Os::Windows, Path::new(r"C:\Windows\System32\svchost.exe")));
    assert!(r.contains(Os::Windows, Path::new(r"C:\Windows\explorer.exe")));
    assert!(!r.contains(Os::Windows, Path::new(r"C:\Windows\Temp\svchost.exe")));
    assert!(!r.contains(
        Os::Windows,
        Path::new(r"C:\Users\elev\Downloads\svchost.exe")
    ));
}

#[test]
fn user_writable_system_folders_are_not_trusted() {
    let r = guardian_platform::trusted_roots_from(
        Path::new(r"C:\Windows"),
        Path::new(r"C:\Program Files"),
        Path::new(r"C:\ProgramData"),
    );
    for p in [
        r"C:\Windows\System32\FxsTmp\svchost.exe",
        r"C:\Windows\SysWOW64\FxsTmp\svchost.exe",
        r"C:\Windows\System32\Tasks_Migrated\svchost.exe",
        r"C:\Windows\SysWOW64\Tasks_Migrated\svchost.exe",
    ] {
        assert!(!r.contains(Os::Windows, Path::new(p)), "{p} is trusted");
    }
}

#[test]
fn system_info_has_hostname_and_os() {
    let i = guardian_platform::current().system_info("3.0.0");
    assert_eq!(i.os, "windows");
    assert!(!i.hostname.is_empty());
}

#[tokio::test]
#[ignore = "needs administrator rights (ETW)"]
async fn etw_reports_new_processes() {
    let p = guardian_platform::current();
    let mut src = p.process_events().unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut child = Command::new("cmd").args(["/c", "exit"]).spawn().unwrap();
    let want = child.id();
    let _ = child.wait();
    let got = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Ok(ev) = src.next().await
                && ev.pid == want
            {
                return ev.pid;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(got, want);
}
