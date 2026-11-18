#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

fn setup() -> Command {
    Command::new(env!("CARGO_BIN_EXE_guardian-setup"))
}

#[test]
fn version_prints() {
    let out = setup().arg("version").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("guardian-setup 3."));
}

#[test]
fn uninstall_without_yes_exits_2() {
    let out = setup().arg("uninstall").output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

#[test]
fn relaunch_happens_once() {
    use guardian_setup::elevate::{relaunch_args, should_relaunch};
    let plain: Vec<std::ffi::OsString> = vec!["guardian-setup.exe".into()];
    let again: Vec<std::ffi::OsString> =
        vec!["guardian-setup.exe".into(), "--elevated-relaunch".into()];
    assert!(should_relaunch(false, &plain));
    assert!(!should_relaunch(false, &again));
    assert!(!should_relaunch(true, &plain));
    let a = relaunch_args(std::path::Path::new(
        r"C:\Users\o'brien\Downloads\guardian-setup.exe",
    ));
    assert_eq!(a[0], "-NoProfile");
    assert_eq!(a[1], "-Command");
    assert_eq!(
        a[2],
        r"Start-Process -Verb RunAs -FilePath 'C:\Users\o''brien\Downloads\guardian-setup.exe' -ArgumentList '--elevated-relaunch'"
    );
}
