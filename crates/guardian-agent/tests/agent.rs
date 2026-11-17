#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use guardian_agent::config::{AgentConfig, AgentPaths};
use guardian_agent::engine::EngineConfig;
use guardian_agent::store::load_state;
use guardian_agent::{AgentHandle, AgentOptions, enroll, run};
use guardian_core::enforce::Os;
use guardian_platform::fake::FakePlatform;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::{
    AgentState, Command, CommandEnvelope, ComputerIdRequest, CreateEnrollCodeRequest,
    CreateRoomRequest, Empty, EnrollCodeKind, MessageCmd, ResultStatus, ServerFrame, command,
    server_frame,
};
use guardian_server::hub::HubEvent;
use guardian_server::testing::{TestServer, req};

fn engine_cfg() -> EngineConfig {
    EngineConfig {
        reconcile_every: Duration::from_millis(200),
        fallback_poll: Duration::from_millis(50),
        retry_events_every: Duration::from_millis(300),
    }
}

fn opts(paths: &AgentPaths, f: &Arc<FakePlatform>) -> AgentOptions {
    AgentOptions {
        paths: paths.clone(),
        platform: f.clone(),
        engine: engine_cfg(),
        backoff_min: Duration::from_millis(100),
    }
}

async fn until(within: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f()
}

fn base_url(s: &TestServer) -> String {
    s.ws_url().trim_end_matches("/agent/ws").to_string()
}

/// Server with a room whose profile ("Lecție") blacklists game.exe; an enrolled agent dir.
async fn setup() -> (
    TestServer,
    String,
    tempfile::TempDir,
    AgentPaths,
    Arc<FakePlatform>,
    u64,
) {
    let s = TestServer::start().await;
    let t = s.admin_token().await;
    let mut a = AdminClient::new(s.grpc().await);
    let room = a
        .create_room(req(&t, CreateRoomRequest { name: "R".into() }))
        .await
        .unwrap()
        .into_inner();
    let mut lesson = a
        .list_profiles(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .profiles
        .into_iter()
        .find(|p| p.name == "Lecție")
        .unwrap();
    lesson.apps = vec!["game.exe".into()];
    a.update_profile(req(&t, lesson)).await.unwrap();
    let code = a
        .create_enroll_code(req(
            &t,
            CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollAgent as i32,
                room_id: room.id,
                max_uses: 1,
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code;
    let dir = tempfile::tempdir().unwrap();
    let paths = AgentPaths::under(dir.path());
    let f = FakePlatform::new(Os::Windows);
    let id = enroll(&paths, &base_url(&s), s.pin().as_deref(), &code, &*f)
        .await
        .unwrap();
    (s, t, dir, paths, f, id)
}

async fn online(h: &AgentHandle) {
    assert!(
        until(Duration::from_secs(5), || h.status().online).await,
        "agent never came online"
    );
}

#[tokio::test]
async fn enrolled_agent_applies_room_profile_and_kills() {
    let (s, _t, _d, paths, f, _id) = setup().await;
    let h = run(opts(&paths, &f)).await.unwrap();
    online(&h).await;
    assert!(
        until(Duration::from_secs(2), || h.status().profile_name
            == "Lecție"
            && h.status().state_version > 0)
        .await
    );
    let game = f.spawn("game.exe", r"C:\g\game.exe");
    assert!(until(Duration::from_secs(1), || !f.is_running(game)).await);
    let saved = load_state(&paths.state()).unwrap();
    assert_eq!(saved.state.unwrap().version, h.status().state_version);
    h.shutdown().await;
    drop(s);
}

#[tokio::test]
async fn boots_offline_with_saved_state() {
    let (s, _t, _d, paths, f, _id) = setup().await;
    let h = run(opts(&paths, &f)).await.unwrap();
    online(&h).await;
    assert!(until(Duration::from_secs(2), || h.status().state_version > 0).await);
    h.shutdown().await;
    let TestServer { handle, .. } = s;
    handle.shutdown().await;
    // The server is gone; the agent restarts from its files.
    let f2 = FakePlatform::new(Os::Windows);
    let h = run(opts(&paths, &f2)).await.unwrap();
    let game = f2.spawn("game.exe", r"C:\g\game.exe");
    assert!(until(Duration::from_secs(1), || !f2.is_running(game)).await);
    assert!(!h.status().online);
    h.shutdown().await;
}

#[tokio::test]
async fn first_boot_without_server_uses_default_lesson() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AgentPaths::under(dir.path());
    AgentConfig {
        server: "ws://127.0.0.1:1".into(),
        cert_sha256: None,
        computer_id: Some(1),
    }
    .save(&paths.config)
    .unwrap();
    let f = FakePlatform::new(Os::Windows);
    let game = f.spawn("game.exe", r"C:\g\game.exe");
    let h = run(opts(&paths, &f)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let st = h.status();
    assert_eq!((st.profile_name.as_str(), st.online), ("Lecție", false));
    assert!(f.is_running(game));
    h.shutdown().await;
}

#[tokio::test]
async fn not_enrolled_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let paths = AgentPaths::under(dir.path());
    let f = FakePlatform::new(Os::Windows);
    assert!(run(opts(&paths, &f)).await.is_err());
}

#[tokio::test]
async fn older_state_is_ignored() {
    let (s, _t, _d, paths, f, id) = setup().await;
    let h = run(opts(&paths, &f)).await.unwrap();
    online(&h).await;
    assert!(until(Duration::from_secs(2), || h.status().state_version > 0).await);
    let (v, prof) = (h.status().state_version, h.status().profile_id);
    let push = |version: u64| ServerFrame {
        msg: Some(server_frame::Msg::State(AgentState {
            version,
            effective_profile_id: 1,
            profiles: vec![guardian_proto::v1::Profile {
                id: 1,
                name: "Liber".into(),
                mode: guardian_proto::v1::Mode::Free as i32,
                apps: vec![],
            }],
            room_id: 0,
        })),
    };
    s.handle.state().hub.send(id, push(v - 1));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(h.status().profile_id, prof);
    s.handle.state().hub.send(id, push(v + 1));
    assert!(until(Duration::from_secs(1), || h.status().profile_id == 1).await);
    assert_eq!(h.status().state_version, v + 1);
    h.shutdown().await;
}

#[tokio::test]
async fn resent_command_is_answered_once() {
    let (s, _t, _d, paths, f, id) = setup().await;
    let h = run(opts(&paths, &f)).await.unwrap();
    online(&h).await;
    let db = &s.handle.state().db;
    let env = CommandEnvelope {
        id: uuid_like(),
        expires_at_unix: guardian_server::db::now_unix() + 600,
        command: Some(Command {
            kind: Some(command::Kind::Message(MessageCmd {
                title: String::new(),
                text: "x".into(),
            })),
        }),
    };
    guardian_server::db::commands::create(db, &env, 1, &[(id, ResultStatus::ResultDelivered)])
        .await
        .unwrap();
    let mut events = s.handle.state().hub.events();
    let frame = || ServerFrame {
        msg: Some(server_frame::Msg::Command(env.clone())),
    };
    s.handle.state().hub.send(id, frame());
    s.handle.state().hub.send(id, frame());
    let mut answers = 0;
    let end = Instant::now() + Duration::from_millis(800);
    while Instant::now() < end {
        if let Ok(Ok(HubEvent::CommandResult {
            command_id, status, ..
        })) = tokio::time::timeout(Duration::from_millis(100), events.recv()).await
            && command_id.to_string() == env.id
        {
            assert_eq!(status, ResultStatus::ResultUnsupported);
            answers += 1;
        }
    }
    assert_eq!(answers, 1);
    // After a restart the id is still remembered.
    h.shutdown().await;
    let h = run(opts(&paths, &f)).await.unwrap();
    online(&h).await;
    s.handle.state().hub.send(id, frame());
    let end = Instant::now() + Duration::from_millis(500);
    while Instant::now() < end {
        if let Ok(Ok(HubEvent::CommandResult { command_id, .. })) =
            tokio::time::timeout(Duration::from_millis(100), events.recv()).await
        {
            assert_ne!(command_id.to_string(), env.id, "answered twice");
        }
    }
    h.shutdown().await;
}

fn uuid_like() -> String {
    let b = guardian_core::random::random_bytes::<16>();
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-4{}-a{}-{}",
        &h[0..8],
        &h[8..12],
        &h[13..16],
        &h[17..20],
        &h[20..32]
    )
}

#[tokio::test]
async fn revoked_agent_keeps_last_policy() {
    let (s, t, _d, paths, f, id) = setup().await;
    let h = run(opts(&paths, &f)).await.unwrap();
    online(&h).await;
    assert!(until(Duration::from_secs(2), || h.status().state_version > 0).await);
    AdminClient::new(s.grpc().await)
        .revoke_computer(req(&t, ComputerIdRequest { id }))
        .await
        .unwrap();
    assert!(until(Duration::from_secs(3), || h.status().revoked).await);
    let game = f.spawn("game.exe", r"C:\g\game.exe");
    assert!(until(Duration::from_secs(1), || !f.is_running(game)).await);
    h.shutdown().await;
}
