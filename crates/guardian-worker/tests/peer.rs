#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use guardian_platform::ipc::{IpcEndpoint, IpcListener, read_frame, write_frame};
use guardian_proto::v1::{AgentToWorker, Ping, WorkerFrame, agent_to_worker, worker_frame};

#[tokio::test]
async fn real_worker_process_says_hello() {
    let d = tempfile::tempdir().unwrap();
    let ep = IpcEndpoint::Unix(d.path().join("w.sock"));
    let mut l = IpcListener::bind(&ep).unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_guardian-worker"))
        .args(["--endpoint", &ep.to_arg()])
        .env("GUARDIAN_SESSION", "s7")
        .spawn()
        .unwrap();
    let (mut s, pid) = tokio::time::timeout(Duration::from_secs(5), l.accept())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(Some(pid), child.id());
    let hello: WorkerFrame = read_frame(&mut s).await.unwrap();
    match hello.msg {
        Some(worker_frame::Msg::Hello(h)) => {
            assert_eq!((h.pid, h.session_id.as_str()), (pid, "s7"))
        }
        other => panic!("{other:?}"),
    }
    write_frame(
        &mut s,
        &AgentToWorker {
            msg: Some(agent_to_worker::Msg::Ping(Ping { seq: 7 })),
        },
    )
    .await
    .unwrap();
    let pong: WorkerFrame = read_frame(&mut s).await.unwrap();
    assert!(matches!(pong.msg, Some(worker_frame::Msg::Pong(p)) if p.seq == 7));
    drop(s);
    let st = tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(st.success());
}

#[tokio::test]
async fn worker_without_agent_exits_with_2() {
    let d = tempfile::tempdir().unwrap();
    let st = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_guardian-worker"))
            .args([
                "--endpoint",
                d.path().join("none.sock").to_str().unwrap(),
                "--connect-timeout",
                "1",
            ])
            .status(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(st.code(), Some(2));
}
