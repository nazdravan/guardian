#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! A server with a room, a teacher, an enrolled console and an enrolled agent.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use guardian_agent::config::AgentPaths;
use guardian_agent::engine::EngineConfig;
use guardian_agent::{AgentHandle, AgentOptions, LocalOptions, enroll, run};
use guardian_core::enforce::Os;
use guardian_core::keys::KeyPair;
use guardian_local::channel::Limits;
use guardian_platform::fake::FakePlatform;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::{
    CreateEnrollCodeRequest, CreateRoomRequest, Empty, EnrollCodeKind, EnrollConsoleRequest,
    LocalActor, Profile,
};
use guardian_server::testing::{StoppedServer, TestServer, req};
use tonic::transport::Channel;

pub const OFFLINE_AFTER: Duration = Duration::from_millis(300);

pub fn opts(paths: &AgentPaths, f: &Arc<FakePlatform>) -> AgentOptions {
    AgentOptions {
        paths: paths.clone(),
        platform: f.clone(),
        engine: EngineConfig {
            reconcile_every: Duration::from_millis(200),
            fallback_poll: Duration::from_millis(50),
            retry_events_every: Duration::from_millis(300),
        },
        backoff_min: Duration::from_millis(100),
        worker: None,
        offline_after: OFFLINE_AFTER,
        local: None,
    }
}

/// Short channel limits for tests.
pub fn test_limits() -> Limits {
    Limits {
        handshake: Duration::from_millis(300),
        idle: Duration::from_secs(2),
    }
}

/// Options with the local listener on 127.0.0.1, ephemeral ports.
pub fn opts_with_listener(paths: &AgentPaths, f: &Arc<FakePlatform>) -> AgentOptions {
    AgentOptions {
        local: Some(LocalOptions {
            tcp: SocketAddr::from(([127, 0, 0, 1], 0)),
            udp: SocketAddr::from(([127, 0, 0, 1], 0)),
            limits: test_limits(),
        }),
        ..opts(paths, f)
    }
}

pub async fn until(within: Duration, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + within;
    while Instant::now() < end {
        if f() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    f()
}

pub struct World {
    pub s: Option<TestServer>,
    pub stopped: Option<StoppedServer>,
    pub admin: String,
    pub room: i64,
    pub lesson: Profile,
    pub exam: Profile,
    pub free: Profile,
    pub teacher: LocalActor,
    pub console_id: i64,
    pub console_key: KeyPair,
    pub _dir: tempfile::TempDir,
    pub paths: AgentPaths,
    pub f: Arc<FakePlatform>,
    pub computer_id: u64,
}

impl World {
    pub fn server(&self) -> &TestServer {
        self.s.as_ref().unwrap()
    }
    pub async fn admin_client(&self) -> AdminClient<Channel> {
        AdminClient::new(self.server().grpc().await)
    }
    pub async fn stop_server(&mut self) {
        let s = self.s.take().unwrap();
        self.stopped = Some(s.stop().await);
    }
    pub async fn start_server(&mut self) {
        let st = self.stopped.take().unwrap();
        self.s = Some(st.start().await);
    }
}

pub async fn world() -> World {
    let s = TestServer::start().await;
    let t = s.admin_token().await;
    let mut a = AdminClient::new(s.grpc().await);
    let room = a
        .create_room(req(
            &t,
            CreateRoomRequest {
                name: "Sala 1".into(),
            },
        ))
        .await
        .unwrap()
        .into_inner();
    let profiles = a
        .list_profiles(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .profiles;
    let by = |n: &str| profiles.iter().find(|p| p.name == n).unwrap().clone();
    let mut lesson = by("Lecție");
    lesson.apps = vec!["game.exe".into()];
    a.update_profile(req(&t, lesson.clone())).await.unwrap();
    let mut exam = by("Examen");
    exam.apps = vec!["notes.exe".into()];
    a.update_profile(req(&t, exam.clone())).await.unwrap();
    let free = by("Liber");
    let teacher_token = s.teacher_token("prof", &[room.id]).await;
    let me = AuthClient::new(s.grpc().await)
        .who_am_i(req(&teacher_token, Empty {}))
        .await
        .unwrap()
        .into_inner();
    let console_code = a
        .create_enroll_code(req(
            &t,
            CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollConsole as i32,
                room_id: 0,
                max_uses: 1,
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code;
    let console_key = KeyPair::generate();
    let console_id = AuthClient::new(s.grpc().await)
        .enroll_console(EnrollConsoleRequest {
            code: console_code,
            public_key: console_key.public().as_bytes().to_vec(),
            name: "teacher-pc".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .console_id;
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
    let base = s.ws_url().trim_end_matches("/agent/ws").to_string();
    let computer_id = enroll(&paths, &base, s.pin().as_deref(), &code, &*f)
        .await
        .unwrap();
    World {
        s: Some(s),
        stopped: None,
        admin: t,
        room: room.id,
        lesson,
        exam,
        free,
        teacher: LocalActor {
            user_id: me.id,
            login: me.login,
        },
        console_id,
        console_key,
        _dir: dir,
        paths,
        f,
        computer_id,
    }
}

pub async fn started(w: &World) -> AgentHandle {
    started_with(w, opts(&w.paths, &w.f)).await
}

pub async fn started_with(_w: &World, o: AgentOptions) -> AgentHandle {
    let h = run(o).await.unwrap();
    assert!(
        until(Duration::from_secs(5), || h.status().online
            && h.status().profile_name == "Lecție")
        .await,
        "agent never came online"
    );
    h
}

pub async fn offline(w: &mut World, h: &AgentHandle) {
    w.stop_server().await;
    assert!(
        until(Duration::from_secs(5), || h
            .status()
            .offline_for
            .is_some_and(|d| d > OFFLINE_AFTER))
        .await,
        "agent never went offline"
    );
}
