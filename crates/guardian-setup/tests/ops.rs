#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_setup::cli::main_cli;
use guardian_setup::fake::FakeHost;
use guardian_setup::lifecycle::OpError;
use guardian_setup::ops::{OPS, Op, hold_lock, run_op, ui_ops};

fn cli(h: &FakeHost, args: &[&str]) -> (i32, String) {
    cli_with_stdin(h, args, "")
}

fn cli_with_stdin(h: &FakeHost, args: &[&str], stdin: &str) -> (i32, String) {
    let mut out = Vec::new();
    let mut input = stdin.as_bytes();
    let mut argv: Vec<std::ffi::OsString> = vec!["guardian-setup".into()];
    argv.extend(args.iter().map(|a| a.into()));
    let code = main_cli(h, argv, &mut out, &mut input);
    (code, String::from_utf8(out).unwrap())
}

const INSTALL: &[&str] = &[
    "install",
    "--server",
    "wss://srv:8443",
    "--code",
    "ABCD-EFGH",
];

#[test]
fn every_op_has_cli_help_and_ui_binding_or_is_cli_only() {
    let h = FakeHost::new();
    for op in OPS {
        assert!(!op.help.is_empty(), "{} has no help", op.name);
        let mut args: Vec<&str> = op.name.split(' ').collect();
        args.push("--help");
        let (code, out) = cli(&h, &args);
        assert_eq!(code, 0, "{}: {out}", op.name);
        assert!(out.contains("Usage"), "{}: {out}", op.name);
        assert_eq!(op.ui, ui_ops().contains(&op.name), "{} ui binding", op.name);
    }
    for name in ui_ops() {
        assert!(OPS.iter().any(|o| o.name == *name && o.ui), "{name}");
    }
}

#[test]
fn mutating_op_without_admin_exits_5() {
    let h = FakeHost::new();
    h.not_elevated();
    let (code, out) = cli(&h, INSTALL);
    assert_eq!(code, 5, "{out}");
    assert!(out.contains("Run this from an elevated prompt"), "{out}");
    assert!(h.calls().is_empty());
    // Reading works without admin.
    assert_eq!(cli(&h, &["status"]).0, 0);
}

#[test]
fn uninstall_needs_yes() {
    let h = FakeHost::new();
    cli(&h, INSTALL);
    let (code, out) = cli(&h, &["uninstall"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("--yes"), "{out}");
    assert_eq!(cli(&h, &["uninstall", "--yes"]).0, 0);
}

#[test]
fn json_output_is_stable() {
    let h = FakeHost::new();
    let (code, out) = cli(&h, &["--json", "status"]);
    assert_eq!(code, 0, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["op"], "status");
    assert!(v["error"].is_null());
    let r = &v["result"];
    for key in [
        "installed",
        "service",
        "computer_id",
        "server",
        "local_port",
        "firewall_rule",
        "legacy_agent",
        "agent_version",
        "problems",
    ] {
        assert!(r.get(key).is_some(), "missing {key} in {r}");
    }
    assert_eq!(r["installed"], false);
    assert_eq!(r["local_port"], 11500);
    h.not_elevated();
    let (code, out) = cli(&h, &["--json", "update"]);
    assert_eq!(code, 5);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(
        v["error"],
        "Run this from an elevated prompt (Run as administrator)."
    );
}

#[test]
fn code_file_dash_reads_stdin() {
    let h = FakeHost::new();
    let (code, out) = cli_with_stdin(
        &h,
        &["install", "--server", "wss://srv:8443", "--code-file", "-"],
        "ABCD-EFGH\n",
    );
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("ABCD-EFGH"), "{out}");
    assert!(h.calls().iter().any(|c| c.contains("enroll")));
    // Neither or both is a usage error.
    assert_eq!(cli(&h, &["install", "--server", "wss://srv:8443"]).0, 2);
}

#[test]
fn second_mutating_op_is_busy() {
    let h = FakeHost::new();
    let _held = hold_lock(&h).unwrap();
    assert!(matches!(
        run_op(&h, Op::Update, &|_| {}),
        Err(OpError::Busy)
    ));
    let (code, out) = cli(&h, &["update"]);
    assert_eq!(code, 1);
    assert!(
        out.contains("Another Guardian Setup operation is in progress."),
        "{out}"
    );
    // Reading does not take the lock.
    assert!(run_op(&h, Op::Status { check: false }, &|_| {}).is_ok());
}

#[test]
fn panic_in_op_is_internal_error() {
    let h = FakeHost::new();
    match run_op(&h, Op::Panic, &|_| {}) {
        Err(OpError::Internal(m)) => assert!(m.contains("test panic"), "{m}"),
        other => panic!("{other:?}"),
    }
    // The lock was released.
    assert!(hold_lock(&h).is_ok());
}

#[test]
fn status_check_exit_10() {
    let h = FakeHost::new();
    assert_eq!(cli(&h, &["status", "--check"]).0, 10);
    cli(&h, INSTALL);
    let (code, out) = cli(&h, &["status", "--check"]);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn whitelist_add_remove_round_trip() {
    let h = FakeHost::new();
    cli(&h, INSTALL);
    assert_eq!(cli(&h, &["whitelist", "add", "Notes.exe"]).0, 0);
    let (_, out) = cli(&h, &["whitelist", "list"]);
    assert!(out.lines().any(|l| l == "notes.exe"), "{out}");
    assert_eq!(cli(&h, &["whitelist", "add", "C:\\x\\a.exe"]).0, 1);
    assert_eq!(cli(&h, &["whitelist", "remove", "notes.exe"]).0, 0);
    let (_, out) = cli(&h, &["whitelist", "list"]);
    assert!(!out.contains("notes.exe"), "{out}");
}

#[test]
fn bad_usage_exits_2() {
    let h = FakeHost::new();
    assert_eq!(cli(&h, &["frobnicate"]).0, 2);
    assert_eq!(cli(&h, &["install", "--local-port", "x"]).0, 2);
}
