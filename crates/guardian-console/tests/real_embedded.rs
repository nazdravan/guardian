//! The console's embedded server driving the real `guardian-agent` binary.
//! Build the binaries first: `cargo build --workspace --bins`.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::*;
use guardian_console::backend::{Action, Backend, ConnState, Event};
use guardian_console::connect_info::agent_target;
use guardian_console::embedded::EmbeddedState;
use guardian_console::settings::{Mode, Secret};
use guardian_proto::v1::Profile;

fn bin(name: &str) -> PathBuf {
    let dir = std::env::var_os("GUARDIAN_BIN_DIR").map_or_else(
        || {
            std::env::var_os("CARGO_TARGET_DIR")
                .map_or_else(
                    || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"),
                    PathBuf::from,
                )
                .join("debug")
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

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn done(c: &Console) {
    let evs = c
        .until(|e| matches!(e, Event::Done(_) | Event::Error(_)), "result")
        .await;
    assert!(matches!(evs.last(), Some(Event::Done(_))), "{evs:?}");
}

#[cfg(target_os = "linux")]
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn console_embedded_server_drives_a_real_agent() {
    let agent_bin = bin("guardian-agent");
    let d = tempfile::tempdir().unwrap();
    let port = free_port();
    let c = Console::new();

    // 1. Embedded server, first admin.
    c.b.send(Action::StartEmbedded {
        data_dir: Some(d.path().join("server")),
        port,
    });
    let evs = c
        .until(
            |e| matches!(e, Event::Embedded(EmbeddedState::Running { .. } | EmbeddedState::Failed { .. })),
            "the embedded server",
        )
        .await;
    let Some(Event::Embedded(EmbeddedState::Running { cert_sha256: pin, .. })) = evs.last().cloned()
    else {
        panic!("{evs:?}")
    };
    c.until(
        |e| matches!(e, Event::Connection(ConnState::NeedsLogin { can_init: true })),
        "first-admin prompt",
    )
    .await;
    c.b.send(Action::Init {
        bootstrap_code: String::new(),
        login: "admin".into(),
        password: Secret::new("admin-password-1"),
    });
    c.until(|e| matches!(e, Event::LoggedIn(_)), "logged in").await;

    // 2. Room, blocked game in "Lecție", a room code.
    c.b.send(Action::CreateRoom {
        name: "Sala 1".into(),
    });
    done(&c).await;
    let f = c.fleet_where(|f| f.rooms().len() == 1).await;
    let room = f.rooms()[0].id;
    let mut lesson: Profile = f
        .profiles()
        .into_iter()
        .find(|p| p.name == "Lecție")
        .unwrap()
        .clone();
    let free = f.profiles().iter().find(|p| p.name == "Liber").unwrap().id;
    lesson.apps = vec!["guardian-test-game".into()];
    c.b.send(Action::SaveProfile(lesson));
    done(&c).await;
    c.b.send(Action::CreateAgentCode {
        room_id: room,
        max_uses: 1,
        ttl_seconds: 600,
    });
    let evs = c.until(|e| matches!(e, Event::Code(_)), "a code").await;
    let Some(Event::Code(code)) = evs.last().cloned() else {
        panic!()
    };

    // 3. The real agent, with the address Connection info would show.
    let mut target = agent_target(&Mode::Embedded { port }, "127.0.0.1", None);
    target.cert_sha256 = Some(pin);
    let agent = |args: &[&str]| {
        let mut cmd = Command::new(&agent_bin);
        cmd.arg("--config")
            .arg(d.path().join("agent/agent.toml"))
            .arg("--data-dir")
            .arg(d.path().join("agent/data"))
            .args(args);
        cmd
    };
    let out = agent(&[
        "enroll",
        "--server",
        &target.server,
        "--cert-sha256",
        target.cert_sha256.as_deref().unwrap(),
        "--code",
        &code.code,
    ])
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "enroll: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut running: Child = agent(&["run"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    c.fleet_where(|f| f.rooms().first().is_some_and(|r| r.online == 1))
        .await;

    // 4–5. Blocked under "Lecție", allowed under "Liber".
    #[cfg(target_os = "linux")]
    {
        let exe = d.path().join("guardian-test-game");
        std::fs::copy("/bin/sleep", &exe).unwrap();
        let mut g = Command::new(&exe).arg("60").spawn().unwrap();
        assert!(
            killed_within(&mut g, Duration::from_secs(5)),
            "the blocked game was not killed"
        );
    }
    c.b.send(Action::SetRoomProfile {
        room_id: room,
        profile_id: free,
    });
    done(&c).await;
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let st = agent(&["status"]).output().unwrap();
        if String::from_utf8_lossy(&st.stdout).contains("profile: Liber") {
            break;
        }
        assert!(Instant::now() < end, "the agent never saved Liber");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    #[cfg(target_os = "linux")]
    {
        let exe = d.path().join("guardian-test-game");
        let mut g = Command::new(&exe).arg("60").spawn().unwrap();
        assert!(
            !killed_within(&mut g, Duration::from_secs(3)),
            "the game was killed under Liber"
        );
        let _ = g.kill();
        let _ = g.wait();
    }

    // 6. Closing the console stops the server; the agent keeps running.
    drop(c);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(running.try_wait().unwrap().is_none(), "the agent exited");
    let _ = Command::new("kill")
        .args(["-TERM", &running.id().to_string()])
        .status();
    let _ = running.wait();
}
