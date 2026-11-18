#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use guardian_core::keys::{KeyPair, PublicKey};
use guardian_local::channel::{ChannelError, Limits, LocalStream, accept, connect};
use guardian_proto::v1::{
    LocalReply, LocalRequest, LocalStatus, StatusRequest, local_reply, local_request,
};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

fn limits() -> Limits {
    Limits {
        handshake: Duration::from_millis(300),
        idle: Duration::from_millis(500),
    }
}

fn status_req() -> LocalRequest {
    LocalRequest {
        id: "r1".into(),
        actor: None,
        kind: Some(local_request::Kind::Status(StatusRequest {})),
    }
}

/// An agent that accepts one connection with `trusted`; returns what `accept` gave.
async fn agent(
    agent_key: KeyPair,
    trusted: Vec<(i64, PublicKey)>,
) -> (
    std::net::SocketAddr,
    tokio::task::JoinHandle<Result<(LocalStream, i64), ChannelError>>,
) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let h = tokio::spawn(async move {
        let (tcp, _) = l.accept().await.unwrap();
        accept(tcp, &agent_key, &trusted, limits()).await
    });
    (addr, h)
}

#[tokio::test]
async fn handshake_and_round_trip() {
    let (ak, ck) = (KeyPair::generate(), KeyPair::generate());
    let apub = ak.public();
    let (addr, h) = agent(ak, vec![(3, ck.public())]).await;
    let mut c = connect(addr, &ck, 3, &apub, limits()).await.unwrap();
    let (mut a, console_id) = h.await.unwrap().unwrap();
    assert_eq!(console_id, 3);
    c.send(&status_req()).await.unwrap();
    let got: LocalRequest = a.recv().await.unwrap();
    assert_eq!(got, status_req());
    let reply = LocalReply {
        request_id: "r1".into(),
        kind: Some(local_reply::Kind::Status(LocalStatus {
            computer_id: 9,
            ..Default::default()
        })),
    };
    a.send(&reply).await.unwrap();
    let back: LocalReply = c.recv().await.unwrap();
    assert_eq!(back, reply);
}

#[tokio::test]
async fn unknown_console_is_refused() {
    let (ak, ck) = (KeyPair::generate(), KeyPair::generate());
    let apub = ak.public();
    let (addr, h) = agent(ak, vec![(3, KeyPair::generate().public())]).await;
    let c = connect(addr, &ck, 3, &apub, limits()).await;
    assert!(matches!(h.await.unwrap(), Err(ChannelError::UnknownPeer)));
    assert!(c.is_err(), "the console must not get a channel");
}

#[tokio::test]
async fn console_id_must_match_key() {
    let (ak, c1, c2) = (
        KeyPair::generate(),
        KeyPair::generate(),
        KeyPair::generate(),
    );
    let apub = ak.public();
    let (addr, h) = agent(ak, vec![(1, c1.public()), (2, c2.public())]).await;
    let _ = connect(addr, &c1, 2, &apub, limits()).await;
    assert!(matches!(h.await.unwrap(), Err(ChannelError::UnknownPeer)));
}

#[tokio::test]
async fn wrong_agent_key_fails() {
    let (ak, ck) = (KeyPair::generate(), KeyPair::generate());
    let (addr, h) = agent(ak, vec![(3, ck.public())]).await;
    let other = KeyPair::generate().public();
    let c = connect(addr, &ck, 3, &other, limits()).await;
    assert!(c.is_err());
    assert!(h.await.unwrap().is_err());
}

#[tokio::test]
async fn silent_peer_times_out() {
    let ak = KeyPair::generate();
    let (addr, h) = agent(ak, vec![]).await;
    let _silent = TcpStream::connect(addr).await.unwrap();
    let t = Instant::now();
    assert!(matches!(h.await.unwrap(), Err(ChannelError::Timeout)));
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn silent_agent_times_out_for_the_console() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let _keep = tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
        drop(s);
    });
    let ck = KeyPair::generate();
    let r = connect(addr, &ck, 1, &KeyPair::generate().public(), limits()).await;
    assert!(matches!(r, Err(ChannelError::Timeout)));
}

#[tokio::test]
async fn garbage_is_an_error_not_a_panic() {
    for junk in [vec![1u8, 2, 3], {
        let mut v = vec![0xff, 0xff];
        v.extend(std::iter::repeat_n(0x5a, 65535));
        v
    }] {
        let ak = KeyPair::generate();
        let (addr, h) = agent(ak, vec![(1, KeyPair::generate().public())]).await;
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(&junk).await.unwrap();
        assert!(h.await.unwrap().is_err());
    }
}

#[tokio::test]
async fn garbage_after_handshake_is_an_error() {
    let (ak, ck) = (KeyPair::generate(), KeyPair::generate());
    let apub = ak.public();
    let (addr, h) = agent(ak, vec![(3, ck.public())]).await;
    let c = connect(addr, &ck, 3, &apub, limits()).await.unwrap();
    let (mut a, _) = h.await.unwrap().unwrap();
    let mut raw = c.into_inner();
    raw.write_all(&[0, 4, 1, 2, 3, 4]).await.unwrap();
    let r: Result<LocalRequest, _> = a.recv().await;
    assert!(r.is_err());
}

#[tokio::test]
async fn oversized_message_is_refused() {
    let (ak, ck) = (KeyPair::generate(), KeyPair::generate());
    let apub = ak.public();
    let (addr, h) = agent(ak, vec![(3, ck.public())]).await;
    let mut c = connect(addr, &ck, 3, &apub, limits()).await.unwrap();
    let _a = h.await.unwrap().unwrap();
    let big = LocalRequest {
        id: "x".repeat(70_000),
        ..status_req()
    };
    assert!(matches!(
        c.send(&big).await,
        Err(ChannelError::FrameTooLarge)
    ));
}

#[tokio::test]
async fn idle_channel_times_out() {
    let (ak, ck) = (KeyPair::generate(), KeyPair::generate());
    let apub = ak.public();
    let (addr, h) = agent(ak, vec![(3, ck.public())]).await;
    let _c = connect(addr, &ck, 3, &apub, limits()).await.unwrap();
    let (mut a, _) = h.await.unwrap().unwrap();
    let r: Result<LocalRequest, _> = a.recv().await;
    assert!(matches!(r, Err(ChannelError::Timeout)));
}
