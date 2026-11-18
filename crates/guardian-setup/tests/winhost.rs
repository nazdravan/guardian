//! The real Windows host (CI `windows` job).
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_setup::host::Host;
use guardian_setup::winhost::WinHost;

#[test]
fn elevation_is_reported() {
    // Only that it answers; CI may or may not run elevated.
    let _ = WinHost.is_elevated();
}

#[test]
fn missing_service_is_none() {
    assert_eq!(WinHost.service("GuardianNoSuchService42").unwrap(), None);
}

#[test]
fn processes_include_this_test() {
    let me = std::process::id();
    assert!(WinHost.processes().unwrap().iter().any(|p| p.pid == me));
}

#[test]
fn setup_dir_is_the_exe_folder() {
    assert!(WinHost.setup_dir().is_dir());
}
