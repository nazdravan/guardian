#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Stdio;
use std::time::{Duration, Instant};

use guardian_agent::config::AgentConfig;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::{CreateEnrollCodeRequest, CreateRoomRequest, Empty, EnrollCodeKind};
use guardian_server::testing::{TestServer, req};

const BIN: &str = env!("CARGO_BIN_EXE_guardian-agent");

fn agent(dir: &std::path::Path) -> std::process::Command {
    let mut c = std::process::Command::new(BIN);
    c.arg("--config")
        .arg(dir.join("agent.toml"))
        .arg("--data-dir")
        .arg(dir.join("data"));
    c
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enroll_run_status_and_sigterm() {
    let s = TestServer::start().await;
    let t = s.admin_token().await;
    let mut a = AdminClient::new(s.grpc().await);
    let room = a
        .create_room(req(&t, CreateRoomRequest { name: "R".into() }))
        .await
        .unwrap()
        .into_inner();
    let code = a
        .create_enroll_code(req(
            &t,
            CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollAgent as i32,
                room_id: room.id,
                max_uses: 2,
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code;
    let server = s.ws_url().trim_end_matches("/agent/ws").to_string();
    let d = tempfile::tempdir().unwrap();

    let out = agent(d.path())
        .args(["enroll", "--server", &server, "--code", &code])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let id = AgentConfig::load(&d.path().join("agent.toml"))
        .unwrap()
        .computer_id
        .unwrap();

    let again = agent(d.path())
        .args(["enroll", "--server", &server, "--code", &code])
        .output()
        .unwrap();
    assert_eq!(again.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&again.stderr).contains("already enrolled"));

    let mut child = agent(d.path())
        .arg("run")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    let mut online = false;
    while Instant::now() < end && !online {
        let list = a
            .list_computers(req(&t, Empty {}))
            .await
            .unwrap()
            .into_inner()
            .computers;
        online = list.iter().any(|c| c.id == id && c.online);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(online, "agent never came online");

    let st = agent(d.path()).arg("status").output().unwrap();
    assert!(st.status.success());
    assert!(String::from_utf8_lossy(&st.stdout).contains(&format!("computer_id: {id}")));

    let kill = std::process::Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(kill.success());
    let end = Instant::now() + Duration::from_secs(5);
    let code = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st.code();
        }
        assert!(Instant::now() < end, "agent did not exit on SIGTERM");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(code, Some(0));
}
