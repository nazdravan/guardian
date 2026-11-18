//! Fallback mode with the real `guardian-agent` binary: the server goes away,
//! the console switches the room locally, the server comes back and wins.
//! Build the binaries first: `cargo build --workspace --bins`. Takes about
//! 70 s: the agent accepts local requests only after 60 s without the server.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use common::*;
use guardian_console::backend::{Action, Backend, BackendOptions, Event, FallbackState};
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::{
    CreateEnrollCodeRequest, CreateRoomRequest, Empty, EnrollCodeKind, Mode as PMode, Profile,
};
use guardian_server::testing::{TEACHER_PASSWORD, TestServer, req};

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

async fn code(s: &TestServer, t: &str, kind: EnrollCodeKind, room_id: i64) -> String {
    AdminClient::new(s.grpc().await)
        .create_enroll_code(req(
            t,
            CreateEnrollCodeRequest {
                kind: kind as i32,
                room_id,
                max_uses: 1,
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code
}

async fn result(c: &Console) -> Result<String, String> {
    let evs = c
        .until(|e| matches!(e, Event::Done(_) | Event::Error(_)), "result")
        .await;
    match evs.last() {
        Some(Event::Done(m)) => Ok(m.clone()),
        Some(Event::Error(m)) => Err(m.clone()),
        _ => unreachable!(),
    }
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
async fn real_agent_follows_the_console_while_the_server_is_down() {
    let agent_bin = bin("guardian-agent");
    let d = tempfile::tempdir().unwrap();
    let local_port = free_port();

    // 1. Server, a room on "Lecție", a blacklist profile to switch to locally.
    let s = TestServer::start_tls().await;
    let t = s.admin_token().await;
    let mut a = AdminClient::new(s.grpc().await);
    let room = a
        .create_room(req(
            &t,
            CreateRoomRequest {
                name: "Sala 1".into(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .id;
    let blocking = a
        .create_profile(req(
            &t,
            Profile {
                id: 0,
                name: "Blocare".into(),
                mode: PMode::Blacklist as i32,
                apps: vec!["guardian-test-other".into()],
            },
        ))
        .await
        .unwrap()
        .into_inner();
    let lesson = a
        .list_profiles(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .profiles
        .into_iter()
        .find(|p| p.name == "Lecție")
        .unwrap();
    s.teacher_token("prof", &[room]).await;

    // 2. The real agent, its local listener on a free port.
    let agent = |args: &[&str]| {
        let mut cmd = Command::new(&agent_bin);
        cmd.arg("--config")
            .arg(d.path().join("agent/agent.toml"))
            .arg("--data-dir")
            .arg(d.path().join("agent/data"))
            .args(args);
        cmd
    };
    let ws = s.ws_url().trim_end_matches("/agent/ws").to_string();
    let agent_code = code(&s, &t, EnrollCodeKind::EnrollAgent, room).await;
    let out = agent(&[
        "enroll",
        "--server",
        &ws,
        "--cert-sha256",
        s.pin().as_deref().unwrap(),
        "--code",
        &agent_code,
    ])
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "enroll: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cfg = d.path().join("agent/agent.toml");
    let mut text = std::fs::read_to_string(&cfg).unwrap();
    text.push_str(&format!("local_port = {local_port}\n"));
    std::fs::write(&cfg, text).unwrap();
    let mut running: Child = agent(&["run"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // 3. The teacher's console, fallback enabled with a console code.
    let cdir = tempfile::tempdir().unwrap();
    std::fs::write(
        cdir.path().join("console.toml"),
        format!("fallback_port = {local_port}\n"),
    )
    .unwrap();
    let c = Console::at_with(
        cdir,
        BackendOptions {
            fallback_broadcast: Some(vec![std::net::SocketAddr::from((
                [127, 0, 0, 1],
                local_port,
            ))]),
            fallback_sweep_every: Duration::from_secs(2),
        },
    );
    c.login(&s, "prof", TEACHER_PASSWORD).await;
    c.fleet_where(|f| f.rooms().first().is_some_and(|r| r.online == 1))
        .await;
    let console_code = code(&s, &t, EnrollCodeKind::EnrollConsole, 0).await;
    c.b.send(Action::EnableFallback {
        code: Some(console_code),
    });
    c.until(
        |e| matches!(e, Event::FallbackEnabled(Some(_))),
        "fallback enabled",
    )
    .await;
    // Let the agent receive the new console key.
    tokio::time::sleep(Duration::from_secs(1)).await;

    // 4. The server goes away.
    let stopped = s.stop().await;
    c.until(
        |e| matches!(e, Event::Fallback(FallbackState::Available)),
        "fallback available",
    )
    .await;
    c.b.send(Action::EnterFallback);
    c.until(
        |e| matches!(e, Event::Fallback(FallbackState::Active { .. })),
        "fallback active",
    )
    .await;

    // 5. The agent takes local requests 60 s after losing the server.
    let end = Instant::now() + Duration::from_secs(120);
    loop {
        c.b.send(Action::SetRoomProfile {
            room_id: room,
            profile_id: blocking.id,
        });
        match result(&c).await {
            Ok(m) if m == "Blocare applied on 1 of 1 computers in Sala 1." => break,
            other => {
                assert!(Instant::now() < end, "never applied: {other:?}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
    let st = String::from_utf8_lossy(&agent(&["status"]).output().unwrap().stdout).into_owned();
    assert!(st.contains("local switch: profile"), "{st}");
    #[cfg(target_os = "linux")]
    {
        let exe = d.path().join("guardian-test-other");
        std::fs::copy("/bin/sleep", &exe).unwrap();
        let mut g = Command::new(&exe).arg("60").spawn().unwrap();
        assert!(
            killed_within(&mut g, Duration::from_secs(5)),
            "the program blocked locally was not killed"
        );
    }

    // 6. The server comes back: its profile wins and the journal reaches it.
    let s = stopped.start().await;
    // The console retries at most every 30 s.
    c.until_within(
        Duration::from_secs(45),
        |e| matches!(e, Event::Fallback(FallbackState::Off)),
        "fallback off",
    )
    .await;
    let end = Instant::now() + Duration::from_secs(90);
    loop {
        let st = String::from_utf8_lossy(&agent(&["status"]).output().unwrap().stdout).into_owned();
        if !st.contains("local switch:") && !st.contains("not yet sent") {
            assert!(st.contains(&format!("profile: {}", lesson.name)), "{st}");
            break;
        }
        assert!(
            Instant::now() < end,
            "the agent kept the local switch: {st}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    // The audit batcher writes shortly after the ack.
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let db = s.open_db_read_only().await;
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT action FROM audit_log WHERE source = 'local' AND action = 'local.set_profile'",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        if !rows.is_empty() {
            break;
        }
        assert!(Instant::now() < end, "no local audit row");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    #[cfg(target_os = "linux")]
    {
        let exe = d.path().join("guardian-test-other");
        let mut g = Command::new(&exe).arg("60").spawn().unwrap();
        assert!(
            !killed_within(&mut g, Duration::from_secs(3)),
            "still blocked after the server came back"
        );
        let _ = g.kill();
        let _ = g.wait();
    }
    let _ = Command::new("kill")
        .args(["-TERM", &running.id().to_string()])
        .status();
    let _ = running.wait();
}
