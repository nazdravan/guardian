#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common_local;

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use common_local::*;
use guardian_agent::config::AgentConfig;
use guardian_agent::{LocalOptions, run};
use guardian_core::keys::{KeyPair, PublicKey};
use guardian_local::channel::{LocalStream, connect};
use guardian_local::discover;
use guardian_proto::v1::{
    DiscoverReply, LocalReply, LocalRequest, LocalSetProfile, StatusRequest, local_reply,
    local_request,
};
use prost::Message;
use tokio::net::{TcpStream, UdpSocket};

fn agent_key(w: &World) -> PublicKey {
    let seed: [u8; 32] = std::fs::read(w.paths.key()).unwrap()[..32]
        .try_into()
        .unwrap();
    KeyPair::from_seed(&seed).public()
}

async fn dial(w: &World, addr: SocketAddr) -> LocalStream {
    connect(
        addr,
        &w.console_key,
        w.console_id,
        &agent_key(w),
        test_limits(),
    )
    .await
    .unwrap()
}

fn status(w: &World) -> LocalRequest {
    LocalRequest {
        id: "s".into(),
        actor: Some(w.teacher.clone()),
        kind: Some(local_request::Kind::Status(StatusRequest {})),
    }
}

#[tokio::test]
async fn console_switches_profile_over_tcp() {
    let mut w = world().await;
    let h = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let (tcp, _udp) = h.local_addrs().unwrap();
    offline(&mut w, &h).await;
    let mut c = dial(&w, tcp).await;
    c.send(&LocalRequest {
        id: "1".into(),
        actor: Some(w.teacher.clone()),
        kind: Some(local_request::Kind::SetProfile(LocalSetProfile {
            room_id: w.room,
            profile_id: w.exam.id,
        })),
    })
    .await
    .unwrap();
    let r: LocalReply = c.recv().await.unwrap();
    match r.kind {
        Some(local_reply::Kind::Status(s)) => assert!(s.local_override),
        other => panic!("{other:?}"),
    }
    assert_eq!(h.status().profile_name, "Examen");
    // The same channel keeps working.
    c.send(&status(&w)).await.unwrap();
    let r: LocalReply = c.recv().await.unwrap();
    assert_eq!(r.request_id, "s");
    h.shutdown().await;
}

#[tokio::test]
async fn unknown_console_gets_nothing() {
    let mut w = world().await;
    let h = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let (tcp, _) = h.local_addrs().unwrap();
    offline(&mut w, &h).await;
    let stranger = KeyPair::generate();
    let r = connect(tcp, &stranger, w.console_id, &agent_key(&w), test_limits()).await;
    assert!(r.is_err());
    h.shutdown().await;
}

#[tokio::test]
async fn half_open_connections_do_not_lock_out_the_console() {
    let mut w = world().await;
    let h = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let (tcp, _) = h.local_addrs().unwrap();
    offline(&mut w, &h).await;
    let mut silent = Vec::new();
    for _ in 0..20 {
        silent.push(TcpStream::connect(tcp).await.unwrap());
    }
    let start = Instant::now();
    let deadline = start + test_limits().handshake + Duration::from_secs(1);
    let mut got = None;
    while Instant::now() < deadline {
        if let Ok(mut c) = connect(
            tcp,
            &w.console_key,
            w.console_id,
            &agent_key(&w),
            test_limits(),
        )
        .await
        {
            c.send(&status(&w)).await.unwrap();
            if let Ok(r) = c.recv::<LocalReply>().await {
                got = Some(r);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(got.is_some(), "locked out for {:?}", start.elapsed());
    drop(silent);
    h.shutdown().await;
}

async fn ask(udp_agent: SocketAddr, bytes: &[u8]) -> Option<DiscoverReply> {
    let s = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    s.send_to(bytes, udp_agent).await.unwrap();
    let mut buf = [0u8; 2048];
    match tokio::time::timeout(Duration::from_millis(500), s.recv_from(&mut buf)).await {
        Ok(Ok((n, _))) => DiscoverReply::decode(&buf[..n]).ok(),
        _ => None,
    }
}

#[tokio::test]
async fn discovery_answers_known_console_only() {
    let w = world().await;
    let h = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let (tcp, udp) = h.local_addrs().unwrap();
    let (r, nonce) = discover::request(&w.console_key, w.console_id);
    let reply = ask(udp, &r.encode_to_vec()).await.expect("no reply");
    assert!(discover::verify_reply(&reply, &nonce, &agent_key(&w)));
    assert_eq!(reply.computer_id, w.computer_id);
    assert_eq!(reply.port, u32::from(tcp.port()));
    assert!(reply.online);
    let (stranger, _) = discover::request(&KeyPair::generate(), w.console_id);
    assert!(ask(udp, &stranger.encode_to_vec()).await.is_none());
    h.shutdown().await;
}

#[tokio::test]
async fn oversized_datagram_is_ignored() {
    let w = world().await;
    let h = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let (_, udp) = h.local_addrs().unwrap();
    assert!(ask(udp, &[0x0a; 1500]).await.is_none());
    assert!(ask(udp, &[0xff, 0x01, 0x02]).await.is_none());
    // Still answers afterwards.
    let (r, _) = discover::request(&w.console_key, w.console_id);
    assert!(ask(udp, &r.encode_to_vec()).await.is_some());
    h.shutdown().await;
}

#[tokio::test]
async fn busy_port_does_not_stop_the_agent() {
    let w = world().await;
    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut o = opts_with_listener(&w.paths, &w.f);
    o.local = Some(LocalOptions {
        tcp: busy.local_addr().unwrap(),
        ..o.local.unwrap()
    });
    let h = run(o).await.unwrap();
    assert_eq!(h.local_addrs(), None);
    assert!(until(Duration::from_secs(5), || h.status().online).await);
    let game = w.f.spawn("game.exe", r"C:\g\game.exe");
    assert!(until(Duration::from_secs(2), || !w.f.is_running(game)).await);
    h.shutdown().await;
}

#[test]
fn config_without_local_port_still_loads() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("agent.toml");
    std::fs::write(&p, "server = \"wss://s:8443\"\ncomputer_id = 3\n").unwrap();
    let c = AgentConfig::load(&p).unwrap();
    assert_eq!(c.local_port, None);
    assert_eq!(c.local_port(), 11500);
    std::fs::write(
        &p,
        "server = \"wss://s:8443\"\ncomputer_id = 3\nlocal_port = 0\n",
    )
    .unwrap();
    assert_eq!(AgentConfig::load(&p).unwrap().local_port(), 0);
}

#[tokio::test]
async fn reconnecting_attacker_does_not_lock_out_the_console() {
    let mut w = world().await;
    let h = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let (tcp, _) = h.local_addrs().unwrap();
    offline(&mut w, &h).await;
    // A script that opens a silent connection every millisecond, forever.
    let attacker = tokio::spawn(async move {
        let mut held = std::collections::VecDeque::new();
        loop {
            if let Ok(s) = TcpStream::connect(tcp).await {
                held.push_back(s);
                if held.len() > 200 {
                    held.pop_front();
                }
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut got = 0;
    for _ in 0..10 {
        if let Ok(mut c) = connect(
            tcp,
            &w.console_key,
            w.console_id,
            &agent_key(&w),
            test_limits(),
        )
        .await
            && c.send(&status(&w)).await.is_ok()
            && c.recv::<LocalReply>().await.is_ok()
        {
            got += 1;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    attacker.abort();
    assert!(got >= 9, "the console got in {got} times out of 10");
    h.shutdown().await;
}

#[tokio::test]
async fn replayed_handshake_is_closed_quickly() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut w = world().await;
    let h = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let (tcp, _) = h.local_addrs().unwrap();
    offline(&mut w, &h).await;
    // A first handshake message as a recorded one would be: valid, but the
    // sender cannot follow it with a transport message.
    let secret = w.console_key.x25519_secret();
    let remote = agent_key(&w).x25519().unwrap();
    let mut hs = snow::Builder::new(guardian_local::channel::NOISE_PATTERN.parse().unwrap())
        .local_private_key(secret.as_slice())
        .unwrap()
        .remote_public_key(&remote)
        .unwrap()
        .build_initiator()
        .unwrap();
    let mut buf = vec![0u8; 1024];
    let n = hs
        .write_message(&w.console_id.to_le_bytes(), &mut buf)
        .unwrap();
    let mut s = TcpStream::connect(tcp).await.unwrap();
    let mut frame = (n as u16).to_be_bytes().to_vec();
    frame.extend_from_slice(&buf[..n]);
    s.write_all(&frame).await.unwrap();
    let t = Instant::now();
    let mut sink = vec![0u8; 4096];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), s.read(&mut sink)).await {
            Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(_)) => continue,
            Err(_) => panic!("never closed"),
        }
    }
    let limit = test_limits().handshake + Duration::from_millis(500);
    assert!(t.elapsed() < limit, "held for {:?}", t.elapsed());
    h.shutdown().await;
}

#[tokio::test]
async fn replayed_discovery_is_answered_once() {
    let w = world().await;
    let h = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let (_, udp) = h.local_addrs().unwrap();
    let (r, _) = discover::request(&w.console_key, w.console_id);
    let bytes = r.encode_to_vec();
    let s = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    for _ in 0..5 {
        s.send_to(&bytes, udp).await.unwrap();
    }
    let mut buf = [0u8; 2048];
    let mut replies = 0;
    while let Ok(Ok(_)) =
        tokio::time::timeout(Duration::from_millis(500), s.recv_from(&mut buf)).await
    {
        replies += 1;
    }
    assert_eq!(replies, 1);
    let (fresh, _) = discover::request(&w.console_key, w.console_id);
    assert!(ask(udp, &fresh.encode_to_vec()).await.is_some());
    h.shutdown().await;
}
