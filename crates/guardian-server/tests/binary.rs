#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(unix)]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use guardian_proto::v1::InitRequest;
use guardian_proto::v1::auth_client::AuthClient;

fn value(line: &str, prefix: &str) -> Option<String> {
    line.find(prefix)
        .map(|i| line[i + prefix.len()..].trim().to_string())
}

#[tokio::test]
async fn binary_starts_prints_pin_and_code_and_stops_on_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_guardian-server"))
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "--listen",
            "127.0.0.1:0",
        ])
        .env("GUARDIAN_LOG", "info")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let (mut addr, mut pin, mut code) = (None, None, None);
    while addr.is_none() || pin.is_none() || code.is_none() {
        let line = rx
            .recv_timeout(Duration::from_secs(20))
            .expect("server output");
        addr = addr.or_else(|| value(&line, "listening on "));
        pin = pin.or_else(|| value(&line, "certificate sha256: "));
        code = code.or_else(|| value(&line, "bootstrap code: "));
    }
    let url = format!("https://{}", addr.unwrap());
    let ch = guardian_client::connect(&url, pin.as_deref())
        .await
        .unwrap();
    let r = AuthClient::new(ch)
        .init(InitRequest {
            bootstrap_code: code.unwrap(),
            login: "admin".into(),
            password: "admin-password".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!r.token.is_empty());
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "exit status {status}");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "server did not stop within 5 s"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[test]
fn help_lists_the_options() {
    let out = Command::new(env!("CARGO_BIN_EXE_guardian-server"))
        .arg("--help")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    for flag in [
        "--data-dir",
        "--listen",
        "--tls-cert",
        "--tls-key",
        "--insecure-plaintext",
        "--log",
    ] {
        assert!(text.contains(flag), "missing {flag} in:\n{text}");
    }
}
