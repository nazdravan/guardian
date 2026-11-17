#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use futures_util::StreamExt;
use guardian_core::keys::KeyPair;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::{
    ComputerIdRequest, CreateEnrollCodeRequest, CreateRoomRequest, Empty, EnrollCodeKind,
    EnrollConsoleRequest, server_frame,
};
use guardian_server::testing::{SimAgent, TestServer, req};
use tonic::transport::Channel;

const SHORT: Duration = Duration::from_millis(500);

async fn admin(s: &TestServer) -> (AdminClient<Channel>, String) {
    (AdminClient::new(s.grpc().await), s.admin_token().await)
}

async fn code(a: &mut AdminClient<Channel>, t: &str, room_id: i64, uses: u32) -> String {
    a.create_enroll_code(req(
        t,
        CreateEnrollCodeRequest {
            kind: EnrollCodeKind::EnrollAgent as i32,
            room_id,
            max_uses: uses,
            ttl_seconds: 0,
        },
    ))
    .await
    .unwrap()
    .into_inner()
    .code
}

async fn room(a: &mut AdminClient<Channel>, t: &str, name: &str) -> guardian_proto::v1::Room {
    a.create_room(req(t, CreateRoomRequest { name: name.into() }))
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test]
async fn enroll_with_room_code_puts_computer_in_room_and_welcomes_it() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let r = room(&mut a, &t, "Sala 12").await;
    let c = code(&mut a, &t, r.id, 5).await;
    let agent = SimAgent::enroll(&s, &c, &["message"]).await.unwrap();
    let st = agent.welcome().state.clone().unwrap();
    assert_eq!(st.room_id, r.id);
    assert_eq!(st.effective_profile_id, r.active_profile_id);
    assert_eq!(st.profiles.len(), 3);
    let list = a
        .list_computers(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .computers;
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0].id, list[0].online, list[0].room_id),
        (agent.id(), true, r.id)
    );
}

#[tokio::test]
async fn hello_with_wrong_key_is_rejected() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let c = code(&mut a, &t, 0, 1).await;
    let agent = SimAgent::enroll(&s, &c, &[]).await.unwrap();
    let e = SimAgent::connect(&s, agent.id(), &KeyPair::generate(), &[])
        .await
        .unwrap_err();
    assert_eq!(e.code, "unauthorized");
    let e = SimAgent::connect(&s, 9999, &KeyPair::generate(), &[])
        .await
        .unwrap_err();
    assert_eq!(e.code, "unauthorized");
}

#[tokio::test]
async fn unknown_protocol_version_is_rejected() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let c = code(&mut a, &t, 0, 1).await;
    let agent = SimAgent::enroll(&s, &c, &[]).await.unwrap();
    let id = agent.id();
    let key = KeyPair::from_seed(&agent.key().seed());
    agent.close().await;
    let e = SimAgent::connect_with_version(&s, id, &key, &[], 2)
        .await
        .unwrap_err();
    assert_eq!(e.code, "protocol");
}

#[tokio::test]
async fn bad_enroll_code_is_rejected() {
    let s = TestServer::start().await;
    s.admin_token().await;
    let e = SimAgent::enroll(&s, "ABCD2345", &[]).await.unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("invalid_code", "invalid or expired enrollment code")
    );
}

#[tokio::test]
async fn handshake_times_out() {
    let s = TestServer::start().await; // handshake timeout 1 s
    let (mut ws, _) = tokio_tungstenite::connect_async(s.ws_url()).await.unwrap();
    let started = std::time::Instant::now();
    let end = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) => break,
                Some(Ok(m)) if m.is_close() => break,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(end.is_ok(), "server must close an idle handshake");
    assert!(started.elapsed() >= Duration::from_millis(900));
}

#[tokio::test]
async fn reconnect_replaces_old_connection() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let c = code(&mut a, &t, 0, 1).await;
    let mut first = SimAgent::enroll(&s, &c, &[]).await.unwrap();
    let id = first.id();
    let key = KeyPair::from_seed(&first.key().seed());
    let mut second = SimAgent::connect(&s, id, &key, &[]).await.unwrap();
    assert!(
        first.wait_closed(Duration::from_secs(3)).await,
        "old connection is closed"
    );
    assert!(s.handle.state().hub.is_online(id));
    // A state push now only reaches the new connection.
    s.handle.state().hub.send(
        id,
        guardian_proto::v1::ServerFrame {
            msg: Some(server_frame::Msg::State(Default::default())),
        },
    );
    let f = second.next_frame(SHORT).await.unwrap();
    assert!(matches!(f.msg, Some(server_frame::Msg::State(_))));
    // The old connection closing did not mark the computer offline.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let list = a
        .list_computers(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .computers;
    assert!(list[0].online);
}

#[tokio::test]
async fn disconnect_marks_offline_and_records_last_seen() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let c = code(&mut a, &t, 0, 1).await;
    let agent = SimAgent::enroll(&s, &c, &[]).await.unwrap();
    let id = agent.id();
    agent.close().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!s.handle.state().hub.is_online(id));
    let list = a
        .list_computers(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .computers;
    assert!(!list[0].online);
    assert!(list[0].last_seen_unix > 0);
}

#[tokio::test]
async fn silent_agent_is_dropped_after_idle_timeout() {
    let s = TestServer::start().await; // idle 2 s
    let (mut a, t) = admin(&s).await;
    let c = code(&mut a, &t, 0, 1).await;
    let agent = SimAgent::enroll(&s, &c, &[]).await.unwrap();
    let id = agent.id();
    agent.go_silent();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(!s.handle.state().hub.is_online(id));
}

#[tokio::test]
async fn revoke_disconnects_and_blocks_reconnect() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let c = code(&mut a, &t, 0, 1).await;
    let mut agent = SimAgent::enroll(&s, &c, &[]).await.unwrap();
    let id = agent.id();
    let key = KeyPair::from_seed(&agent.key().seed());
    a.revoke_computer(req(&t, ComputerIdRequest { id }))
        .await
        .unwrap();
    let f = agent.next_frame(Duration::from_secs(2)).await.unwrap();
    match f.msg {
        Some(server_frame::Msg::Error(e)) => assert_eq!(e.code, "revoked"),
        other => panic!("expected revoked, got {other:?}"),
    }
    assert!(agent.wait_closed(Duration::from_secs(2)).await);
    let e = SimAgent::connect(&s, id, &key, &[]).await.unwrap_err();
    assert_eq!(e.code, "unauthorized");
}

#[tokio::test]
async fn same_public_key_cannot_enroll_twice() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let key = KeyPair::generate();
    let c1 = code(&mut a, &t, 0, 1).await;
    SimAgent::enroll_with_key(&s, &c1, &key, &[]).await.unwrap();
    let c2 = code(&mut a, &t, 0, 1).await;
    let e = SimAgent::enroll_with_key(&s, &c2, &key, &[])
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_code");
}

#[tokio::test]
async fn welcome_lists_trusted_consoles_and_room_teachers() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let r1 = room(&mut a, &t, "R1").await;
    let r2 = room(&mut a, &t, "R2").await;
    s.teacher_token("ion", &[r1.id]).await;
    s.teacher_token("maria", &[r2.id]).await;
    let cc = a
        .create_enroll_code(req(
            &t,
            CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollConsole as i32,
                ..Default::default()
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code;
    AuthClient::new(s.grpc().await)
        .enroll_console(EnrollConsoleRequest {
            code: cc,
            public_key: KeyPair::generate().public().as_bytes().to_vec(),
            name: "L".into(),
        })
        .await
        .unwrap();
    let c = code(&mut a, &t, r1.id, 1).await;
    let agent = SimAgent::enroll(&s, &c, &[]).await.unwrap();
    let w = agent.welcome();
    assert_eq!(w.consoles.len(), 1);
    let mut logins: Vec<_> = w.users.iter().map(|u| u.login.as_str()).collect();
    logins.sort();
    assert_eq!(logins, ["admin", "ion"]);
}
