#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::control_client::ControlClient;
use guardian_proto::v1::watch_client::WatchClient;
use guardian_proto::v1::{
    Command, CommandResult, CreateEnrollCodeRequest, CreateRoomRequest, Empty, EnrollCodeKind,
    MessageCmd, ResultStatus, Room, SendCommandRequest, SetUserRoomsRequest, UpdateUserRequest,
    WatchEvent, command, send_command_request, watch_event,
};
use guardian_server::testing::{SimAgent, TestServer, req};
use tonic::transport::Channel;
use tonic::{Code, Streaming};

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
        let t = s.admin_token().await;
        Ctx {
            a: AdminClient::new(s.grpc().await),
            s,
            t,
        }
    }
    async fn room(&mut self, name: &str) -> Room {
        self.a
            .create_room(req(&self.t, CreateRoomRequest { name: name.into() }))
            .await
            .unwrap()
            .into_inner()
    }
    async fn code(&mut self, room_id: i64) -> String {
        self.a
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
            .code
    }
    async fn watch(&self, token: &str) -> Streaming<WatchEvent> {
        WatchClient::new(self.s.grpc().await)
            .watch(req(token, Empty {}))
            .await
            .unwrap()
            .into_inner()
    }
}

async fn next(
    st: &mut Streaming<WatchEvent>,
    t: Duration,
) -> Option<Result<watch_event::Event, tonic::Status>> {
    match tokio::time::timeout(t, st.message()).await {
        Ok(Ok(Some(e))) => e.event.map(Ok),
        Ok(Err(s)) => Some(Err(s)),
        _ => None,
    }
}

async fn snapshot(st: &mut Streaming<WatchEvent>) -> guardian_proto::v1::Snapshot {
    match next(st, WAIT).await {
        Some(Ok(watch_event::Event::Snapshot(s))) => s,
        other => panic!("expected snapshot, got {other:?}"),
    }
}

/// Next `Computer` event, skipping others.
async fn next_computer(
    st: &mut Streaming<WatchEvent>,
    t: Duration,
) -> Option<guardian_proto::v1::Computer> {
    let end = tokio::time::Instant::now() + t;
    loop {
        let left = end.saturating_duration_since(tokio::time::Instant::now());
        match next(st, left).await? {
            Ok(watch_event::Event::Computer(c)) => return Some(c),
            Ok(_) => {}
            Err(e) => panic!("{e}"),
        }
    }
}

#[tokio::test]
async fn snapshot_then_online_event() {
    let mut c = Ctx::new().await;
    let r1 = c.room("R1").await;
    let tt = c.s.teacher_token("ion", &[r1.id]).await;
    let mut w = c.watch(&tt).await;
    let snap = snapshot(&mut w).await;
    assert_eq!(snap.rooms.len(), 1);
    assert!(snap.computers.is_empty());
    assert_eq!(snap.profiles.len(), 3);
    let code = c.code(r1.id).await;
    let agent = SimAgent::enroll(&c.s, &code, &[]).await.unwrap();
    let comp = next_computer(&mut w, WAIT).await.unwrap();
    assert_eq!((comp.id, comp.online), (agent.id(), true));
    agent.close().await;
    let comp = next_computer(&mut w, WAIT).await.unwrap();
    assert!(!comp.online);
}

#[tokio::test]
async fn teacher_does_not_see_other_rooms() {
    let mut c = Ctx::new().await;
    let r1 = c.room("R1").await;
    let r2 = c.room("R2").await;
    let code2 = c.code(r2.id).await;
    let _a2 = SimAgent::enroll(&c.s, &code2, &[]).await.unwrap();
    let tt = c.s.teacher_token("ion", &[r1.id]).await;
    let mut w = c.watch(&tt).await;
    let snap = snapshot(&mut w).await;
    assert_eq!(snap.rooms.iter().map(|r| r.id).collect::<Vec<_>>(), [r1.id]);
    assert!(snap.computers.is_empty());
    let code2b = c.code(r2.id).await;
    let _b = SimAgent::enroll(&c.s, &code2b, &[]).await.unwrap();
    assert!(next_computer(&mut w, QUIET).await.is_none());
    // Admin sees everything, including roomless computers.
    let code0 = c.code(0).await;
    let _x = SimAgent::enroll(&c.s, &code0, &[]).await.unwrap();
    let mut wa = c.watch(&c.t).await;
    let snap = snapshot(&mut wa).await;
    assert_eq!(snap.rooms.len(), 2);
    assert_eq!(snap.computers.len(), 3);
}

#[tokio::test]
async fn removed_from_room_stops_events_and_commands() {
    let mut c = Ctx::new().await;
    let r1 = c.room("R1").await;
    let tt = c.s.teacher_token("ion", &[r1.id]).await;
    let me =
        c.a.list_users(req(&c.t, Empty {}))
            .await
            .unwrap()
            .into_inner()
            .users
            .into_iter()
            .find(|u| u.login == "ion")
            .unwrap();
    let mut w = c.watch(&tt).await;
    snapshot(&mut w).await;
    c.a.set_user_rooms(req(
        &c.t,
        SetUserRoomsRequest {
            user_id: me.id,
            room_ids: vec![],
        },
    ))
    .await
    .unwrap();
    // The stream re-reads visibility and resends a (now empty) snapshot.
    let snap = snapshot(&mut w).await;
    assert!(snap.rooms.is_empty());
    let code = c.code(r1.id).await;
    let _agent = SimAgent::enroll(&c.s, &code, &["message"]).await.unwrap();
    assert!(next_computer(&mut w, QUIET).await.is_none());
    let e = ControlClient::new(c.s.grpc().await)
        .send_command(req(
            &tt,
            SendCommandRequest {
                target: Some(send_command_request::Target::RoomId(r1.id)),
                command: Some(Command {
                    kind: Some(command::Kind::Message(MessageCmd {
                        title: String::new(),
                        text: "x".into(),
                    })),
                }),
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::PermissionDenied);
}

#[tokio::test]
async fn command_results_are_streamed() {
    let mut c = Ctx::new().await;
    let r1 = c.room("R1").await;
    let code = c.code(r1.id).await;
    let mut agent = SimAgent::enroll(&c.s, &code, &["message"]).await.unwrap();
    let tt = c.s.teacher_token("ion", &[r1.id]).await;
    let mut w = c.watch(&tt).await;
    snapshot(&mut w).await;
    let id = ControlClient::new(c.s.grpc().await)
        .send_command(req(
            &tt,
            SendCommandRequest {
                target: Some(send_command_request::Target::RoomId(r1.id)),
                command: Some(Command {
                    kind: Some(command::Kind::Message(MessageCmd {
                        title: String::new(),
                        text: "x".into(),
                    })),
                }),
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .command_id;
    agent
        .send_result(CommandResult {
            command_id: id.clone(),
            status: ResultStatus::ResultOk as i32,
            detail: "ok".into(),
        })
        .await;
    let mut seen = Vec::new();
    while let Some(Ok(e)) = next(&mut w, WAIT).await {
        if let watch_event::Event::CommandResult(r) = e {
            assert_eq!(r.command_id, id);
            let st = ResultStatus::try_from(r.result.unwrap().status).unwrap();
            seen.push(st);
            if st == ResultStatus::ResultOk {
                break;
            }
        }
    }
    assert_eq!(
        seen,
        [ResultStatus::ResultDelivered, ResultStatus::ResultOk]
    );
}

#[tokio::test]
async fn disabled_user_stream_ends() {
    let mut c = Ctx::new().await;
    let r1 = c.room("R1").await;
    let tt = c.s.teacher_token("ion", &[r1.id]).await;
    let me =
        c.a.list_users(req(&c.t, Empty {}))
            .await
            .unwrap()
            .into_inner()
            .users
            .into_iter()
            .find(|u| u.login == "ion")
            .unwrap();
    let mut w = c.watch(&tt).await;
    snapshot(&mut w).await;
    c.a.update_user(req(
        &c.t,
        UpdateUserRequest {
            id: me.id,
            active: Some(false),
            ..Default::default()
        },
    ))
    .await
    .unwrap();
    match next(&mut w, WAIT).await {
        Some(Err(s)) => assert_eq!(s.code(), Code::Unauthenticated),
        other => panic!("expected Unauthenticated, got {other:?}"),
    }
}

#[tokio::test]
async fn room_changes_are_streamed() {
    let mut c = Ctx::new().await;
    let r1 = c.room("R1").await;
    let mut w = c.watch(&c.t).await;
    snapshot(&mut w).await;
    c.a.rename_room(req(
        &c.t,
        guardian_proto::v1::RenameRoomRequest {
            id: r1.id,
            name: "R1b".into(),
        },
    ))
    .await
    .unwrap();
    match next(&mut w, WAIT).await {
        Some(Ok(watch_event::Event::Room(r))) => assert_eq!(r.name, "R1b"),
        other => panic!("{other:?}"),
    }
    c.a.delete_room(req(&c.t, guardian_proto::v1::IdRequest { id: r1.id }))
        .await
        .unwrap();
    match next(&mut w, WAIT).await {
        Some(Ok(watch_event::Event::RoomRemoved(id))) => assert_eq!(id, r1.id),
        other => panic!("{other:?}"),
    }
}
