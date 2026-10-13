//! `Guardian.exe report` against a fixture folder (spec R25, R32).

use std::process::Command;

fn fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/core/tests/fixtures/healthy")
}

#[test]
fn report_text_and_exit_code() {
    let out = Command::new(env!("CARGO_BIN_EXE_Guardian")).args(["report", "--offline", "--dir"]).arg(fixture()).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("VERDICT:"), "{text}");
    assert!(!text.contains("agt_secret_value_123"));
    // Off Windows the service is unreadable → "could not check" (Unknown), never 0 with errors hidden.
    let code = out.status.code().unwrap();
    assert!([0, 10, 11].contains(&code), "{code}");
}

#[test]
fn report_json_parses() {
    let out = Command::new(env!("CARGO_BIN_EXE_Guardian")).args(["report", "--json", "--offline", "--dir"]).arg(fixture()).output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["config"]["server_address"], "https://guardian.example.com");
}

#[test]
fn usage_error_is_2() {
    let out = Command::new(env!("CARGO_BIN_EXE_Guardian")).args(["report", "--nope"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}
