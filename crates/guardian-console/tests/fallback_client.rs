#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../../guardian-agent/tests/common_local/mod.rs"]
mod common_local;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use common_local::*;
use guardian_console::fallback::cache::Reach;
use guardian_console::fallback::client::LocalClient;
use guardian_core::keys::KeyPair;
use guardian_local::discover;
use guardian_proto::v1::{CachedComputer, CachedUser, DiscoverRequest, Role};
use prost::Message;
use tokio::net::UdpSocket;

fn client(w: &World, udp: SocketAddr) -> LocalClient {
    LocalClient::new(
        KeyPair::from_seed(&w.console_key.seed()),
        w.console_id,
        11500,
    )
    .with_limits(test_limits())
    .with_broadcast(vec![udp])
}

fn cached(w: &World, lan: &[&str]) -> CachedComputer {
    CachedComputer {
        id: w.computer_id,
        name: "pc-1".into(),
        room_id: w.room,
        public_key: agent_public(&w.paths).as_bytes().to_vec(),
        lan_addresses: lan.iter().map(|s| s.to_string()).collect(),
        exception_profile_id: 0,
    }
}

fn teacher(w: &World) -> CachedUser {
    CachedUser {
        user_id: w.teacher.user_id,
        login: w.teacher.login.clone(),
        role: Role::Teacher as i32,
        room_ids: vec![w.room],
        verifier: String::new(),
    }
}

#[tokio::test]
async fn probe_reads_status() {
    let mut w = world().await;
    let h = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let (tcp, udp) = h.local_addrs().unwrap();
    offline(&mut w, &h).await;
    let (r, addr) = client(&w, udp)
        .probe(&cached(&w, &[]), &teacher(&w), Some(tcp))
        .await;
    match r {
        Reach::Reachable(s) => {
            assert_eq!(s.computer_id, w.computer_id);
            assert_eq!(s.effective_profile_id, w.lesson.id);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(addr, Some(tcp));
    h.shutdown().await;
}

#[tokio::test]
async fn probe_of_online_agent_says_still_online() {
    let w = world().await;
    let h = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let (tcp, udp) = h.local_addrs().unwrap();
    let (r, _) = client(&w, udp)
        .probe(&cached(&w, &[]), &teacher(&w), Some(tcp))
        .await;
    assert_eq!(r, Reach::StillOnline);
    h.shutdown().await;
}

#[tokio::test]
async fn set_profile_applies() {
    let mut w = world().await;
    let h = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let (tcp, udp) = h.local_addrs().unwrap();
    offline(&mut w, &h).await;
    let c = client(&w, udp);
    match c
        .set_profile(tcp, &cached(&w, &[]), &teacher(&w), w.room, w.exam.id)
        .await
    {
        Reach::Reachable(s) => assert!(s.local_override),
        other => panic!("{other:?}"),
    }
    assert_eq!(h.status().profile_name, "Examen");
    let refused = c
        .set_profile(tcp, &cached(&w, &[]), &teacher(&w), w.room, w.free.id)
        .await;
    assert_eq!(
        refused,
        Reach::Refused("A free profile cannot be set in fallback mode.".into())
    );
    h.shutdown().await;
}

#[tokio::test]
async fn stale_address_is_found_by_discovery() {
    let mut w = world().await;
    let h = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let (tcp, udp) = h.local_addrs().unwrap();
    offline(&mut w, &h).await;
    let pc = cached(&w, &["127.0.0.2"]);
    let out = client(&w, udp)
        .sweep(&[pc], &teacher(&w), &BTreeMap::new())
        .await;
    let (reach, addr) = out.get(&w.computer_id).unwrap();
    assert!(matches!(reach, Reach::Reachable(_)), "{reach:?}");
    assert_eq!(*addr, Some(tcp));
    h.shutdown().await;
}

#[tokio::test]
async fn reply_from_impostor_is_ignored() {
    let w = world().await;
    // A fake responder that answers every request with another key.
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let udp = sock.local_addr().unwrap();
    let id = w.computer_id;
    tokio::spawn(async move {
        let imp = KeyPair::generate();
        let mut buf = [0u8; 2048];
        while let Ok((n, from)) = sock.recv_from(&mut buf).await {
            if let Ok(r) = DiscoverRequest::decode(&buf[..n]) {
                let rep = discover::reply(&imp, id, &r.nonce, 1, false);
                let _ = sock.send_to(&rep.encode_to_vec(), from).await;
            }
        }
    });
    let found = client(&w, udp)
        .discover(&[cached(&w, &[])], Duration::from_millis(500))
        .await;
    assert!(found.is_empty(), "{found:?}");
}

#[tokio::test]
async fn unreachable_is_unreachable_within_3s() {
    let w = world().await;
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead = free.local_addr().unwrap();
    drop(free);
    let quiet = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let t = Instant::now();
    let out = client(&w, quiet.local_addr().unwrap())
        .sweep(&[cached(&w, &["127.0.0.2"])], &teacher(&w), &{
            let mut m = BTreeMap::new();
            m.insert(w.computer_id, dead);
            m
        })
        .await;
    assert_eq!(out.get(&w.computer_id).unwrap().0, Reach::Unreachable);
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
}
