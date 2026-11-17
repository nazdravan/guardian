#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use guardian_core::keys::KeyPair;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::control_client::ControlClient;
use guardian_proto::v1::{
    AgentState, CreateEnrollCodeRequest, CreateRoomRequest, Empty, EnrollCodeKind,
    EnrollConsoleRequest, IdRequest, Profile, Room, ServerFrame, SetRoomProfileRequest,
    UpdateComputerRequest, Welcome, server_frame,
};
use guardian_server::testing::{SimAgent, TestServer, req};
use tonic::Code;
use tonic::transport::Channel;

const WAIT: Duration = Duration::from_secs(2);
const QUIET: Duration = Duration::from_millis(500);

struct Ctx {
    s: TestServer,
    a: AdminClient<Channel>,
    t: String,
}

impl Ctx {
    async fn new() -> Ctx {
        let s = TestServer::start().await;
        let a = AdminClient::new(s.grpc().await);
        let t = s.admin_token().await;
        Ctx { s, a, t }
    }
    async fn room(&mut self, name: &str) -> Room {
        self.a
            .create_room(req(&self.t, CreateRoomRequest { name: name.into() }))
            .await
            .unwrap()
            .into_inner()
    }
    async fn agent(&mut self, room_id: i64) -> SimAgent {
        let code = self
            .a
            .create_enroll_code(req(
                &self.t,
                CreateEnrollCodeRequest {
                    kind: EnrollCodeKind::EnrollAgent as i32,
                    room_id,
                    max_uses: 1,
                    ttl_seconds: 0,
                },
            ))
            .await
            .unwrap()
            .into_inner()
            .code;
        SimAgent::enroll(&self.s, &code, &[]).await.unwrap()
    }
    async fn profile(&mut self, name: &str) -> Profile {
        self.a
            .list_profiles(req(&self.t, Empty {}))
            .await
            .unwrap()
            .into_inner()
            .profiles
            .into_iter()
            .find(|p| p.name == name)
            .unwrap()
    }
    async fn control(&self) -> ControlClient<Channel> {
        ControlClient::new(self.s.grpc().await)
    }
}

async fn next_state(agent: &mut SimAgent) -> AgentState {
    match agent.next_frame(WAIT).await {
        Some(ServerFrame {
            msg: Some(server_frame::Msg::State(st)),
        }) => st,
        other => panic!("expected State, got {other:?}"),
    }
}

async fn next_trust(agent: &mut SimAgent) -> Welcome {
    match agent.next_frame(WAIT).await {
        Some(ServerFrame {
            msg: Some(server_frame::Msg::TrustUpdate(w)),
        }) => w,
        other => panic!("expected TrustUpdate, got {other:?}"),
    }
}

/// Skips trust updates (room assignment pushes one) until a State arrives.
async fn next_state_skipping_trust(agent: &mut SimAgent) -> AgentState {
    loop {
        match agent.next_frame(WAIT).await {
            Some(ServerFrame {
                msg: Some(server_frame::Msg::State(st)),
            }) => return st,
            Some(ServerFrame {
                msg: Some(server_frame::Msg::TrustUpdate(_)),
            }) => {}
            other => panic!("expected State, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn teacher_switching_room_profile_pushes_state_to_room_only() {
    let mut c = Ctx::new().await;
    let r1 = c.room("R1").await;
    let r2 = c.room("R2").await;
    let mut a1 = c.agent(r1.id).await;
    let mut a2 = c.agent(r2.id).await;
    let v0 = a1.welcome().state.clone().unwrap().version;
    let tt = c.s.teacher_token("ion", &[r1.id]).await;
    // Assigning rooms pushes trust to everyone; drain it.
    next_trust(&mut a1).await;
    next_trust(&mut a2).await;
    let exam = c.profile("Examen").await;
    let room = c
        .control()
        .await
        .set_room_profile(req(
            &tt,
            SetRoomProfileRequest {
                room_id: r1.id,
                profile_id: exam.id,
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(room.active_profile_id, exam.id);
    let st = next_state(&mut a1).await;
    assert_eq!(st.effective_profile_id, exam.id);
    assert!(st.version > v0);
    assert!(
        a2.next_frame(QUIET).await.is_none(),
        "other room gets nothing"
    );
}

#[tokio::test]
async fn teacher_cannot_switch_other_room() {
    let mut c = Ctx::new().await;
    let r1 = c.room("R1").await;
    let r2 = c.room("R2").await;
    let mut a2 = c.agent(r2.id).await;
    let tt = c.s.teacher_token("ion", &[r1.id]).await;
    next_trust(&mut a2).await;
    let exam = c.profile("Examen").await;
    let mut ctl = c.control().await;
    let e = ctl
        .set_room_profile(req(
            &tt,
            SetRoomProfileRequest {
                room_id: r2.id,
                profile_id: exam.id,
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::PermissionDenied);
    assert!(a2.next_frame(QUIET).await.is_none());
    let e = ctl
        .set_room_profile(req(
            &c.t,
            SetRoomProfileRequest {
                room_id: 999,
                profile_id: exam.id,
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::NotFound);
    let e = ctl
        .set_room_profile(req(
            &c.t,
            SetRoomProfileRequest {
                room_id: r2.id,
                profile_id: 999,
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::NotFound);
}

#[tokio::test]
async fn exception_overrides_room_profile() {
    let mut c = Ctx::new().await;
    let r1 = c.room("R1").await;
    let mut a1 = c.agent(r1.id).await;
    let free = c.profile("Liber").await;
    c.a.update_computer(req(
        &c.t,
        UpdateComputerRequest {
            id: a1.id(),
            exception_profile_id: Some(free.id),
            ..Default::default()
        },
    ))
    .await
    .unwrap();
    assert_eq!(next_state(&mut a1).await.effective_profile_id, free.id);
    c.a.update_computer(req(
        &c.t,
        UpdateComputerRequest {
            id: a1.id(),
            exception_profile_id: Some(0),
            ..Default::default()
        },
    ))
    .await
    .unwrap();
    assert_eq!(
        next_state(&mut a1).await.effective_profile_id,
        r1.active_profile_id
    );
}

#[tokio::test]
async fn editing_profile_apps_pushes_to_everyone_online() {
    let mut c = Ctx::new().await;
    let r1 = c.room("R1").await;
    let mut a1 = c.agent(r1.id).await;
    let mut a2 = c.agent(0).await;
    let mut lesson = c.profile("Lecție").await;
    lesson.apps = vec!["chrome.exe".into()];
    c.a.update_profile(req(&c.t, lesson.clone())).await.unwrap();
    for a in [&mut a1, &mut a2] {
        let st = next_state(a).await;
        let p = st.profiles.iter().find(|p| p.id == lesson.id).unwrap();
        assert_eq!(p.apps, ["chrome.exe"]);
    }
}

#[tokio::test]
async fn moving_computer_between_rooms_pushes_new_room_state() {
    let mut c = Ctx::new().await;
    let r1 = c.room("R1").await;
    let r2 = c.room("R2").await;
    let exam = c.profile("Examen").await;
    let mut ctl = c.control().await;
    ctl.set_room_profile(req(
        &c.t,
        SetRoomProfileRequest {
            room_id: r2.id,
            profile_id: exam.id,
        },
    ))
    .await
    .unwrap();
    let mut a1 = c.agent(r1.id).await;
    c.a.update_computer(req(
        &c.t,
        UpdateComputerRequest {
            id: a1.id(),
            room_id: Some(r2.id),
            ..Default::default()
        },
    ))
    .await
    .unwrap();
    let st = next_state_skipping_trust(&mut a1).await;
    assert_eq!((st.room_id, st.effective_profile_id), (r2.id, exam.id));
}

#[tokio::test]
async fn revoking_console_sends_trust_update() {
    let mut c = Ctx::new().await;
    let mut a1 = c.agent(0).await;
    let code =
        c.a.create_enroll_code(req(
            &c.t,
            CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollConsole as i32,
                ..Default::default()
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code;
    let console = AuthClient::new(c.s.grpc().await)
        .enroll_console(EnrollConsoleRequest {
            code,
            public_key: KeyPair::generate().public().as_bytes().to_vec(),
            name: "L".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(next_trust(&mut a1).await.consoles.len(), 1);
    c.a.revoke_console(req(
        &c.t,
        IdRequest {
            id: console.console_id,
        },
    ))
    .await
    .unwrap();
    let w = next_trust(&mut a1).await;
    assert!(w.consoles.is_empty());
    assert!(w.state.is_none() && w.pending.is_empty());
}
