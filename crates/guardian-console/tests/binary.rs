#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

const BIN: &str = env!("CARGO_BIN_EXE_guardian-console");

#[test]
fn version_flag() {
    let out = std::process::Command::new(BIN)
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("guardian-console {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn unknown_flag_is_usage_error() {
    let out = std::process::Command::new(BIN)
        .arg("--bogus")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
