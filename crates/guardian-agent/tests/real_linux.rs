//! The real thing on Linux: the `guardian-server`, `guardian-cli` and
//! `guardian-agent` binaries, a real process killed by name, and enforcement
//! that keeps going with the server down. Build the binaries first:
//! `cargo build --workspace --bins`.

#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn bin(name: &str) -> PathBuf {
    let dir = std::env::var_os("GUARDIAN_BIN_DIR").map_or_else(
        || {
            let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
                || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"),
                PathBuf::from,
            );
            target.join("debug")
        },
        PathBuf::from,
    );
    let p = dir.join(name);
    assert!(
        p.is_file(),
        "{} is missing; run: cargo build --workspace --bins",
        p.display()
    );
    p
}

fn value(line: &str, prefix: &str) -> Option<String> {
    line.find(prefix)
        .map(|i| line[i + prefix.len()..].trim().to_string())
}

struct Cli {
    session: PathBuf,
    url: String,
    pin: String,
}

impl Cli {
    fn run(&self, args: &[&str], stdin: Option<&str>) -> String {
        let mut c = Command::new(bin("guardian-cli"))
            .arg("--config")
            .arg(&self.session)
            .args(["--server", &self.url, "--cert-sha256", &self.pin])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = c.stdin.take().unwrap();
        if let Some(s) = stdin {
            input.write_all(s.as_bytes()).unwrap();
        }
        drop(input);
        let out = c.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "guardian-cli {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
}

fn sigterm_and_wait(child: &mut Child, what: &str) {
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            assert!(st.success(), "{what} exited with {st}");
            return;
        }
        assert!(Instant::now() < end, "{what} did not stop within 5 s");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Starts the test "game" (a copy of `sleep`).
fn game(exe: &Path) -> Child {
    Command::new(exe).arg("60").spawn().unwrap()
}

fn killed_within(child: &mut Child, within: Duration) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn server_agent_cli_on_real_linux() {
    let d = tempfile::tempdir().unwrap();

    // 1. The server, with a self-signed certificate.
    let mut server = Command::new(bin("guardian-server"))
        .arg("--data-dir")
        .arg(d.path().join("server"))
        .args(["--listen", "127.0.0.1:0", "--log", "info"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = server.stderr.take().unwrap();
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
    let (addr, pin, code) = (addr.unwrap(), pin.unwrap(), code.unwrap());

    // 2. The CLI sets up a room whose lesson profile blocks the game.
    let cli = Cli {
        session: d.path().join("cli.toml"),
        url: format!("https://{addr}"),
        pin: pin.clone(),
    };
    cli.run(
        &["init", "--code", &code, "--login", "admin"],
        Some("admin-password-1\n"),
    );
    cli.run(&["room", "add", "Sala 1"], None);
    cli.run(&["profile", "apps", "Lecție", "guardian-test-game"], None);
    // The first line is the code, the second says how long it is valid.
    let printed = cli.run(&["code", "agent", "--room", "Sala 1", "--uses", "1"], None);
    let enroll_code = printed.lines().next().unwrap().trim().to_string();
    assert!(!enroll_code.is_empty());

    // 3. The agent enrolls and runs.
    let agent = |args: &[&str]| {
        let mut c = Command::new(bin("guardian-agent"));
        c.arg("--config")
            .arg(d.path().join("agent/agent.toml"))
            .arg("--data-dir")
            .arg(d.path().join("agent/data"))
            .args(args);
        c
    };
    let ws = format!("wss://{addr}");
    let out = agent(&[
        "enroll",
        "--server",
        &ws,
        "--cert-sha256",
        &pin,
        "--code",
        &enroll_code,
    ])
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "enroll: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut running = agent(&["run"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // 4. The game dies.
    let exe = d.path().join("guardian-test-game");
    std::fs::copy("/bin/sleep", &exe).unwrap();
    let mut g = game(&exe);
    assert!(
        killed_within(&mut g, Duration::from_secs(10)),
        "the blocked game was not killed"
    );

    // 5. A free profile lets it live.
    cli.run(&["room", "profile", "Sala 1", "Liber"], None);
    std::thread::sleep(Duration::from_secs(1));
    let mut g = game(&exe);
    assert!(
        !killed_within(&mut g, Duration::from_secs(3)),
        "the game was killed under the free profile"
    );
    let _ = g.kill();
    let _ = g.wait();

    // 6. Back to the lesson, then the server goes away: still enforced.
    cli.run(&["room", "profile", "Sala 1", "Lecție"], None);
    std::thread::sleep(Duration::from_secs(1));
    sigterm_and_wait(&mut server, "guardian-server");
    let mut g = game(&exe);
    assert!(
        killed_within(&mut g, Duration::from_secs(3)),
        "the game survived with the server down"
    );

    // 7. The agent stops cleanly.
    sigterm_and_wait(&mut running, "guardian-agent");
}
