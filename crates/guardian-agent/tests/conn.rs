#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use guardian_agent::conn::{ConnConfig, ConnError, ConnEvent, Connection, enroll};
use guardian_core::keys::KeyPair;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::control_client::ControlClient;
use guardian_proto::v1::{
    ComputerIdRequest, CreateEnrollCodeRequest, CreateRoomRequest, Empty, EnrollCodeKind,
    SetRoomProfileRequest, SystemInfo,
};
use guardian_server::testing::{TestServer, req};
use tokio::sync::mpsc;

fn info() -> SystemInfo {
    SystemInfo {
        hostname: "pc-1".into(),
        os: "linux".into(),
        agent_version: "3.0.0".into(),
        lan_addresses: vec![],
    }
}

async fn room_code(s: &TestServer) -> (String, i64) {
    let t = s.admin_token().await;
    let mut a = AdminClient::new(s.grpc().await);
    let r = a
        .create_room(req(&t, CreateRoomRequest { name: "R".into() }))
        .await
        .unwrap()
        .into_inner();
    let code = a
        .create_enroll_code(req(
            &t,
            CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollAgent as i32,
                room_id: r.id,
                max_uses: 5,
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code;
    (code, r.id)
}

fn cfg(s: &TestServer, id: u64) -> ConnConfig {
    ConnConfig {
        server: s.ws_url().trim_end_matches("/agent/ws").to_string(),
        pin: s.pin(),
        computer_id: id,
        capabilities: vec![],
        backoff_min: Duration::from_millis(100),
        backoff_max: Duration::from_secs(2),
    }
}

async fn next(rx: &mut mpsc::Receiver<ConnEvent>, within: Duration) -> Option<ConnEvent> {
    tokio::time::timeout(within, rx.recv()).await.ok().flatten()
}

async fn until_online(rx: &mut mpsc::Receiver<ConnEvent>) -> guardian_proto::v1::Welcome {
    loop {
        match next(rx, Duration::from_secs(5)).await {
            Some(ConnEvent::Online { welcome }) => return welcome,
            Some(_) => {}
            None => panic!("not online in time"),
        }
    }
}

async fn setup(s: &TestServer) -> (u64, KeyPair, i64) {
    let (code, room) = room_code(s).await;
    let key = KeyPair::generate();
    let id = enroll(&cfg(s, 0).server, s.pin().as_deref(), &code, &key, info())
        .await
        .unwrap();
    (id, key, room)
}

#[tokio::test]
async fn enrolls_and_goes_online() {
    let s = TestServer::start().await;
    let (id, key, room) = setup(&s).await;
    let (tx, mut rx) = mpsc::channel(16);
    let c = Connection::spawn(cfg(&s, id), key, info(), tx);
    let w = until_online(&mut rx).await;
    assert_eq!(w.state.unwrap().room_id, room);
    assert!(c.online_since().is_some());
    let t = s.admin_token().await;
    let list = AdminClient::new(s.grpc().await)
        .list_computers(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .computers;
    assert!(list[0].online);
    c.shutdown().await;
}

#[tokio::test]
async fn bad_code_is_rejected() {
    let s = TestServer::start().await;
    s.admin_token().await;
    let e = enroll(
        &cfg(&s, 0).server,
        None,
        "ABCD2345",
        &KeyPair::generate(),
        info(),
    )
    .await
    .unwrap_err();
    match e {
        ConnError::Rejected { code, .. } => assert_eq!(code, "invalid_code"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn reconnects_after_server_restart() {
    let s = TestServer::start().await;
    let (id, key, _) = setup(&s).await;
    let (tx, mut rx) = mpsc::channel(16);
    let c = Connection::spawn(cfg(&s, id), key, info(), tx);
    until_online(&mut rx).await;
    let s = s.restart_same_port().await;
    let mut saw_offline = false;
    loop {
        match next(&mut rx, Duration::from_secs(5)).await {
            Some(ConnEvent::Offline { .. }) => saw_offline = true,
            Some(ConnEvent::Online { .. }) => break,
            Some(_) => {}
            None => panic!("did not come back"),
        }
    }
    assert!(saw_offline);
    c.shutdown().await;
    drop(s);
}

#[tokio::test]
async fn revocation_stops_reconnecting() {
    let s = TestServer::start().await;
    let (id, key, _) = setup(&s).await;
    let (tx, mut rx) = mpsc::channel(16);
    let c = Connection::spawn(cfg(&s, id), key, info(), tx);
    until_online(&mut rx).await;
    let t = s.admin_token().await;
    AdminClient::new(s.grpc().await)
        .revoke_computer(req(&t, ComputerIdRequest { id }))
        .await
        .unwrap();
    loop {
        match next(&mut rx, Duration::from_secs(3)).await {
            Some(ConnEvent::Revoked) => break,
            Some(_) => {}
            None => panic!("no Revoked"),
        }
    }
    while let Some(ev) = next(&mut rx, Duration::from_secs(2)).await {
        assert!(
            !matches!(ev, ConnEvent::Online { .. }),
            "reconnected after revocation"
        );
    }
    c.shutdown().await;
}

#[tokio::test]
async fn state_push_is_forwarded() {
    let s = TestServer::start().await;
    let (id, key, room) = setup(&s).await;
    let (tx, mut rx) = mpsc::channel(16);
    let c = Connection::spawn(cfg(&s, id), key, info(), tx);
    until_online(&mut rx).await;
    let t = s.admin_token().await;
    let profiles = AdminClient::new(s.grpc().await)
        .list_profiles(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .profiles;
    let exam = profiles.iter().find(|p| p.name == "Examen").unwrap().id;
    ControlClient::new(s.grpc().await)
        .set_room_profile(req(
            &t,
            SetRoomProfileRequest {
                room_id: room,
                profile_id: exam,
            },
        ))
        .await
        .unwrap();
    loop {
        match next(&mut rx, Duration::from_secs(3)).await {
            Some(ConnEvent::State(st)) => {
                assert_eq!(st.effective_profile_id, exam);
                break;
            }
            Some(_) => {}
            None => panic!("no State"),
        }
    }
    c.shutdown().await;
}

#[tokio::test]
async fn works_over_tls_with_pin() {
    let s = TestServer::start_tls().await;
    assert!(s.ws_url().starts_with("wss://"));
    let (id, key, _) = setup(&s).await;
    let (tx, mut rx) = mpsc::channel(16);
    let c = Connection::spawn(cfg(&s, id), key, info(), tx);
    until_online(&mut rx).await;
    c.shutdown().await;
}
