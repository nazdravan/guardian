#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_platform::ipc::{read_frame, write_frame};
use guardian_proto::v1::{Ping, WorkerFrame, WorkerHello, worker_frame};
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn frames_roundtrip_and_reject_oversize() {
    let (mut a, mut b) = tokio::io::duplex(1 << 20);
    let f = WorkerFrame {
        msg: Some(worker_frame::Msg::Hello(WorkerHello {
            pid: 5,
            ..Default::default()
        })),
    };
    write_frame(&mut a, &f).await.unwrap();
    let back: WorkerFrame = read_frame(&mut b).await.unwrap();
    assert_eq!(back, f);
    a.write_all(&70_000u32.to_be_bytes()).await.unwrap();
    let e = read_frame::<_, Ping>(&mut b).await.unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
}

#[cfg(unix)]
#[tokio::test]
async fn listener_reports_peer_pid() {
    use guardian_platform::ipc::{IpcEndpoint, IpcListener, connect};
    let d = tempfile::tempdir().unwrap();
    let ep = IpcEndpoint::Unix(d.path().join("sock/worker.sock"));
    let mut l = IpcListener::bind(&ep).unwrap();
    let client = tokio::spawn(async move { connect(&ep).await.unwrap() });
    let (_s, pid) = l.accept().await.unwrap();
    assert_eq!(pid, std::process::id());
    client.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn stale_socket_is_replaced() {
    use guardian_platform::ipc::{IpcEndpoint, IpcListener};
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("worker.sock");
    std::fs::write(&path, b"stale").unwrap();
    IpcListener::bind(&IpcEndpoint::Unix(path)).unwrap();
}

#[test]
fn endpoint_arg_roundtrip() {
    use guardian_platform::ipc::IpcEndpoint;
    for ep in [
        IpcEndpoint::Unix("/run/x/w.sock".into()),
        IpcEndpoint::Pipe(r"\\.\pipe\guardian-agent-worker".into()),
    ] {
        assert_eq!(IpcEndpoint::parse(&ep.to_arg()), ep);
    }
}
