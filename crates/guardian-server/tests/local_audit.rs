#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use futures_util::StreamExt;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::watch_client::WatchClient;
use guardian_proto::v1::{
    CreateEnrollCodeRequest, Empty, EnrollCodeKind, LocalAuditEntry, Profile, ServerFrame,
    UpdateComputerRequest, server_frame, watch_event,
};
use guardian_server::testing::{SimAgent, TestServer, req};
use tonic::transport::Channel;

const WAIT: Duration = Duration::from_secs(3);

async fn agent(s: &TestServer, a: &mut AdminClient<Channel>, t: &str) -> SimAgent {
    let code = a
        .create_enroll_code(req(
            t,
            CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollAgent as i32,
                room_id: 0,
                max_uses: 1,
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code;
    SimAgent::enroll(s, &code, &[]).await.unwrap()
}

fn entry(seq: u64, at: i64) -> LocalAuditEntry {
    LocalAuditEntry {
        seq,
        at_unix: at,
        console_id: 4,
        user_id: 1,
        login: "admin".into(),
        action: "local.set_profile".into(),
        detail: format!("room 3 → profile 5 (Examen) #{seq}"),
    }
}

async fn next_ack(sim: &mut SimAgent) -> u64 {
    let end = tokio::time::Instant::now() + WAIT;
    loop {
        let left = end.saturating_duration_since(tokio::time::Instant::now());
        match sim.next_frame(left).await {
            Some(ServerFrame {
                msg: Some(server_frame::Msg::LocalAuditAck(a)),
            }) => return a.up_to_seq,
            Some(_) => {}
            None => panic!("no LocalAuditAck"),
        }
    }
}

async fn local_rows(s: &TestServer) -> Vec<(i64, Option<i64>, String, String, String)> {
    let db = s.open_db_read_only().await;
    sqlx::query_as(
        "SELECT at, actor_user_id, action, target, detail FROM audit_log WHERE source = 'local' ORDER BY id",
    )
    .fetch_all(&db)
    .await
    .unwrap()
}

async fn until_rows(s: &TestServer, n: usize) -> Vec<(i64, Option<i64>, String, String, String)> {
    let end = std::time::Instant::now() + WAIT;
    loop {
        let rows = local_rows(s).await;
        if rows.len() >= n || std::time::Instant::now() > end {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn local_audit_is_stored_with_source_local_and_its_time() {
    let s = TestServer::start().await;
    let mut a = AdminClient::new(s.grpc().await);
    let t = s.admin_token().await;
    let mut sim = agent(&s, &mut a, &t).await;
    sim.send_local_audit(vec![entry(1, 1_000), entry(2, 2_000)])
        .await;
    assert_eq!(next_ack(&mut sim).await, 2);
    let rows = until_rows(&s, 2).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, 1_000);
    assert_eq!(rows[1].0, 2_000);
    assert_eq!(rows[0].1, Some(1));
    assert_eq!(rows[0].2, "local.set_profile");
    assert_eq!(rows[0].3, format!("computer:{}", sim.id()));
    assert!(rows[0].4.contains("Examen"), "{}", rows[0].4);
    assert!(rows[0].4.contains("console 4"), "{}", rows[0].4);
    assert!(rows[0].4.contains("admin"), "{}", rows[0].4);
}

#[tokio::test]
async fn oversized_journal_is_capped() {
    let s = TestServer::start().await;
    let mut a = AdminClient::new(s.grpc().await);
    let t = s.admin_token().await;
    let mut sim = agent(&s, &mut a, &t).await;
    sim.send_local_audit((1..=1500).map(|i| entry(i, 10)).collect())
        .await;
    assert_eq!(next_ack(&mut sim).await, 1000);
    let rows = until_rows(&s, 1000).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(local_rows(&s).await.len(), 1000, "{}", rows.len());
}

#[tokio::test]
async fn long_texts_are_cut() {
    let s = TestServer::start().await;
    let mut a = AdminClient::new(s.grpc().await);
    let t = s.admin_token().await;
    let mut sim = agent(&s, &mut a, &t).await;
    let mut e = entry(1, 5);
    e.action = "a".repeat(500);
    e.detail = "d".repeat(5000);
    sim.send_local_audit(vec![e]).await;
    next_ack(&mut sim).await;
    let rows = until_rows(&s, 1).await;
    assert!(rows[0].2.chars().count() <= 200);
    assert!(rows[0].4.chars().count() <= 200);
}

#[tokio::test]
async fn computer_carries_its_public_key() {
    let s = TestServer::start().await;
    let mut a = AdminClient::new(s.grpc().await);
    let t = s.admin_token().await;
    let sim = agent(&s, &mut a, &t).await;
    let key = sim.key().public().as_bytes().to_vec();
    let list = a
        .list_computers(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(list.computers[0].public_key, key);
    let mut w = WatchClient::new(s.grpc().await)
        .watch(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner();
    match w.next().await.unwrap().unwrap().event {
        Some(watch_event::Event::Snapshot(snap)) => {
            assert_eq!(snap.computers[0].public_key, key);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn exception_is_in_agent_state() {
    let s = TestServer::start().await;
    let mut a = AdminClient::new(s.grpc().await);
    let t = s.admin_token().await;
    let mut sim = agent(&s, &mut a, &t).await;
    assert_eq!(
        sim.welcome().state.as_ref().unwrap().exception_profile_id,
        0
    );
    let profiles: Vec<Profile> = a
        .list_profiles(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .profiles;
    let free = profiles.iter().find(|p| p.name == "Liber").unwrap().id;
    a.update_computer(req(
        &t,
        UpdateComputerRequest {
            id: sim.id(),
            exception_profile_id: Some(free),
            ..Default::default()
        },
    ))
    .await
    .unwrap();
    let end = tokio::time::Instant::now() + WAIT;
    loop {
        let left = end.saturating_duration_since(tokio::time::Instant::now());
        match sim.next_frame(left).await {
            Some(ServerFrame {
                msg: Some(server_frame::Msg::State(st)),
            }) => {
                assert_eq!(st.exception_profile_id, free);
                assert_eq!(st.effective_profile_id, free);
                return;
            }
            Some(_) => {}
            None => panic!("no state"),
        }
    }
}

#[tokio::test]
async fn stopped_server_restarts_on_the_same_port() {
    let s = TestServer::start().await;
    let url = s.url();
    let addr = s.handle.local_addr();
    let stopped = s.stop().await;
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
    let s = stopped.start().await;
    assert_eq!(s.url(), url);
    let t = s.admin_token().await;
    assert!(!t.is_empty());
}
