#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use guardian_agent::config::AgentPaths;
use guardian_agent::engine::EngineConfig;
use guardian_agent::store::load_local;
use guardian_agent::{AgentHandle, AgentOptions, enroll, run};
use guardian_core::enforce::Os;
use guardian_core::keys::KeyPair;
use guardian_platform::fake::FakePlatform;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::{
    Command, CommandEnvelope, CreateEnrollCodeRequest, CreateRoomRequest, Empty, EnrollCodeKind,
    EnrollConsoleRequest, LocalActor, LocalReply, LocalRequest, LocalSetProfile, MessageCmd,
    Profile, ResultStatus, SetUserRoomsRequest, StatusRequest, UpdateComputerRequest, command,
    local_reply, local_request,
};
use guardian_server::testing::{StoppedServer, TestServer, req};
use tonic::transport::Channel;

const OFFLINE_AFTER: Duration = Duration::from_millis(300);

fn opts(paths: &AgentPaths, f: &Arc<FakePlatform>) -> AgentOptions {
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

struct World {
    s: Option<TestServer>,
    stopped: Option<StoppedServer>,
    admin: String,
    room: i64,
    lesson: Profile,
    exam: Profile,
    free: Profile,
    teacher: LocalActor,
    console_id: i64,
    _dir: tempfile::TempDir,
    paths: AgentPaths,
    f: Arc<FakePlatform>,
    computer_id: u64,
}

impl World {
    fn server(&self) -> &TestServer {
        self.s.as_ref().unwrap()
    }
    async fn admin_client(&self) -> AdminClient<Channel> {
        AdminClient::new(self.server().grpc().await)
    }
    async fn stop_server(&mut self) {
        let s = self.s.take().unwrap();
        self.stopped = Some(s.stop().await);
    }
    async fn start_server(&mut self) {
        let st = self.stopped.take().unwrap();
        self.s = Some(st.start().await);
    }
}

async fn world() -> World {
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
    let console_id = AuthClient::new(s.grpc().await)
        .enroll_console(EnrollConsoleRequest {
            code: console_code,
            public_key: KeyPair::generate().public().as_bytes().to_vec(),
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
        _dir: dir,
        paths,
        f,
        computer_id,
    }
}

async fn started(w: &World) -> AgentHandle {
    let h = run(opts(&w.paths, &w.f)).await.unwrap();
    assert!(
        until(Duration::from_secs(5), || h.status().online
            && h.status().profile_name == "Lecție")
        .await,
        "agent never came online"
    );
    h
}

async fn offline(w: &mut World, h: &AgentHandle) {
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

fn set_profile(actor: &LocalActor, room: i64, profile: i64) -> LocalRequest {
    LocalRequest {
        id: "req-1".into(),
        actor: Some(actor.clone()),
        kind: Some(local_request::Kind::SetProfile(LocalSetProfile {
            room_id: room,
            profile_id: profile,
        })),
    }
}

fn refusal(r: &LocalReply) -> &str {
    match &r.kind {
        Some(local_reply::Kind::Refused(x)) => &x.code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn online_agent_refuses_with_online() {
    let w = world().await;
    let h = started(&w).await;
    let r = h.local_request(w.console_id, set_profile(&w.teacher, w.room, w.exam.id));
    assert_eq!(refusal(&r), "online");
    assert_eq!(r.request_id, "req-1");
    h.shutdown().await;
}

#[tokio::test]
async fn offline_agent_accepts_set_profile() {
    let mut w = world().await;
    let h = started(&w).await;
    offline(&mut w, &h).await;
    let status = LocalRequest {
        id: "s".into(),
        actor: Some(w.teacher.clone()),
        kind: Some(local_request::Kind::Status(StatusRequest {})),
    };
    match h.local_request(w.console_id, status).kind {
        Some(local_reply::Kind::Status(s)) => {
            assert_eq!(s.computer_id, w.computer_id);
            assert_eq!(s.room_id, w.room);
            assert_eq!(s.effective_profile_id, w.lesson.id);
            assert!(!s.local_override);
        }
        other => panic!("{other:?}"),
    }
    let r = h.local_request(w.console_id, set_profile(&w.teacher, w.room, w.exam.id));
    match r.kind {
        Some(local_reply::Kind::Status(s)) => {
            assert!(s.local_override);
            assert_eq!(s.effective_profile_id, w.exam.id);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(h.status().local_override, Some(w.exam.id));
    assert_eq!(h.status().profile_name, "Examen");
    let other = w.f.spawn("other.exe", r"C:\x\other.exe");
    assert!(until(Duration::from_secs(2), || !w.f.is_running(other)).await);
    let notes = w.f.spawn("notes.exe", r"C:\x\notes.exe");
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(w.f.is_running(notes));
    let local = load_local(&w.paths.local()).unwrap();
    assert_eq!(local.journal.len(), 1);
    assert_eq!(local.journal[0].action, "local.set_profile");
    assert_eq!(local.journal[0].console_id, w.console_id);
    assert_eq!(local.journal[0].login, "prof");
    assert!(local.journal[0].detail.contains("Examen"));
    h.shutdown().await;
}

#[tokio::test]
async fn free_profile_is_refused() {
    let mut w = world().await;
    let h = started(&w).await;
    offline(&mut w, &h).await;
    let r = h.local_request(w.console_id, set_profile(&w.teacher, w.room, w.free.id));
    assert_eq!(refusal(&r), "free_profile");
    assert_eq!(h.status().local_override, None);
    h.shutdown().await;
}

#[tokio::test]
async fn teacher_of_other_room_is_refused() {
    let mut w = world().await;
    let other_token = w.server().teacher_token("other", &[]).await;
    let other = AuthClient::new(w.server().grpc().await)
        .who_am_i(req(&other_token, Empty {}))
        .await
        .unwrap()
        .into_inner();
    let h = started(&w).await;
    offline(&mut w, &h).await;
    let actor = LocalActor {
        user_id: other.id,
        login: other.login,
    };
    let r = h.local_request(w.console_id, set_profile(&actor, w.room, w.exam.id));
    assert_eq!(refusal(&r), "unauthorized");
    let r = h.local_request(
        w.console_id,
        set_profile(&w.teacher, w.room + 100, w.exam.id),
    );
    assert_eq!(refusal(&r), "not_in_room");
    h.shutdown().await;
}

#[tokio::test]
async fn removed_teacher_is_refused_by_the_agent() {
    let mut w = world().await;
    let h = started(&w).await;
    let mut a = w.admin_client().await;
    a.set_user_rooms(req(
        &w.admin,
        SetUserRoomsRequest {
            user_id: w.teacher.user_id,
            room_ids: vec![],
        },
    ))
    .await
    .unwrap();
    // Let the trust update reach the agent before the server goes away.
    tokio::time::sleep(Duration::from_millis(500)).await;
    offline(&mut w, &h).await;
    let r = h.local_request(w.console_id, set_profile(&w.teacher, w.room, w.exam.id));
    assert_eq!(refusal(&r), "unauthorized");
    h.shutdown().await;
}

#[tokio::test]
async fn computer_with_exception_refuses() {
    let mut w = world().await;
    let h = started(&w).await;
    let mut a = w.admin_client().await;
    a.update_computer(req(
        &w.admin,
        UpdateComputerRequest {
            id: w.computer_id,
            exception_profile_id: Some(w.free.id),
            ..Default::default()
        },
    ))
    .await
    .unwrap();
    assert!(
        until(Duration::from_secs(3), || h.status().profile_name
            == "Liber")
        .await
    );
    offline(&mut w, &h).await;
    let r = h.local_request(w.console_id, set_profile(&w.teacher, w.room, w.exam.id));
    assert_eq!(refusal(&r), "has_exception");
    h.shutdown().await;
}

#[tokio::test]
async fn command_is_unsupported_and_not_journaled() {
    let mut w = world().await;
    let h = started(&w).await;
    offline(&mut w, &h).await;
    let r = h.local_request(
        w.console_id,
        LocalRequest {
            id: "c".into(),
            actor: Some(w.teacher.clone()),
            kind: Some(local_request::Kind::Command(CommandEnvelope {
                id: "11111111-1111-1111-1111-111111111111".into(),
                expires_at_unix: i64::MAX,
                command: Some(Command {
                    kind: Some(command::Kind::Message(MessageCmd {
                        title: "t".into(),
                        text: "x".into(),
                    })),
                }),
            })),
        },
    );
    match r.kind {
        Some(local_reply::Kind::CommandResult(c)) => {
            assert_eq!(c.status, ResultStatus::ResultUnsupported as i32);
        }
        other => panic!("{other:?}"),
    }
    let bad = h.local_request(
        w.console_id,
        LocalRequest {
            id: "b".into(),
            actor: None,
            kind: Some(local_request::Kind::Status(StatusRequest {})),
        },
    );
    assert_eq!(refusal(&bad), "bad_request");
    assert!(load_local(&w.paths.local()).is_none_or(|l| l.journal.is_empty()));
    h.shutdown().await;
}

#[tokio::test]
async fn override_and_journal_survive_restart() {
    let mut w = world().await;
    let h = started(&w).await;
    offline(&mut w, &h).await;
    h.local_request(w.console_id, set_profile(&w.teacher, w.room, w.exam.id));
    h.shutdown().await;
    let h = run(opts(&w.paths, &w.f)).await.unwrap();
    assert_eq!(h.status().profile_name, "Examen");
    assert_eq!(h.status().local_override, Some(w.exam.id));
    assert_eq!(load_local(&w.paths.local()).unwrap().journal.len(), 1);
    h.shutdown().await;
}

async fn local_rows(s: &TestServer) -> Vec<(String, String, Option<i64>)> {
    let db = s.open_db_read_only().await;
    sqlx::query_as(
        "SELECT action, target, actor_user_id FROM audit_log WHERE source = 'local' ORDER BY id",
    )
    .fetch_all(&db)
    .await
    .unwrap()
}

#[tokio::test]
async fn server_return_sends_journal_and_undoes_override() {
    let mut w = world().await;
    let h = started(&w).await;
    offline(&mut w, &h).await;
    h.local_request(w.console_id, set_profile(&w.teacher, w.room, w.exam.id));
    assert_eq!(h.status().profile_name, "Examen");
    w.start_server().await;
    assert!(
        until(Duration::from_secs(8), || h.status().online
            && h.status().profile_name == "Lecție"
            && h.status().local_override.is_none())
        .await,
        "{:?}",
        h.status()
    );
    let mut rows = Vec::new();
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        rows = local_rows(w.server()).await;
        if !rows.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0, "local.set_profile");
    assert_eq!(rows[0].1, format!("computer:{}", w.computer_id));
    assert_eq!(rows[0].2, Some(w.teacher.user_id));
    assert!(
        until(Duration::from_secs(3), || load_local(&w.paths.local())
            .is_some_and(|l| l.journal.is_empty()))
        .await
    );
    // game.exe is blocked again under Lecție.
    let game = w.f.spawn("game.exe", r"C:\g\game.exe");
    assert!(until(Duration::from_secs(2), || !w.f.is_running(game)).await);
    h.shutdown().await;
}

#[tokio::test]
async fn override_is_undone_when_server_returns_with_same_version() {
    let mut w = world().await;
    let h = started(&w).await;
    let version = h.status().state_version;
    offline(&mut w, &h).await;
    h.local_request(w.console_id, set_profile(&w.teacher, w.room, w.exam.id));
    w.start_server().await;
    assert!(
        until(Duration::from_secs(8), || h.status().online
            && h.status().profile_name == "Lecție")
        .await,
        "{:?}",
        h.status()
    );
    assert_eq!(h.status().state_version, version);
    // The engine enforces Lecție again: a program Examen would kill keeps running.
    let other = w.f.spawn("other.exe", r"C:\x\other.exe");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(w.f.is_running(other), "the engine still enforces Examen");
    h.shutdown().await;
}
