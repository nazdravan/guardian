#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A whole lesson, over TLS with a pinned self-signed certificate.

use std::collections::BTreeMap;
use std::time::Duration;

use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::control_client::ControlClient;
use guardian_proto::v1::watch_client::WatchClient;
use guardian_proto::v1::{
    Command, CommandResult, ComputerIdRequest, CreateEnrollCodeRequest, CreateRoomRequest, Empty,
    EnrollCodeKind, GetCommandRequest, ResultStatus, SendCommandRequest, ServerFrame,
    SetRoomProfileRequest, ShutdownCmd, WatchEvent, command, send_command_request, server_frame,
    watch_event,
};
use guardian_server::testing::{SimAgent, TestServer, req};
use tonic::Streaming;

const WAIT: Duration = Duration::from_secs(3);

async fn next_event(w: &mut Streaming<WatchEvent>) -> watch_event::Event {
    tokio::time::timeout(WAIT, w.message())
        .await
        .expect("watch event in time")
        .unwrap()
        .unwrap()
        .event
        .unwrap()
}

/// Reads events until `f` returns `Some`.
async fn until<T>(
    w: &mut Streaming<WatchEvent>,
    mut f: impl FnMut(watch_event::Event) -> Option<T>,
) -> T {
    loop {
        if let Some(t) = f(next_event(w).await) {
            return t;
        }
    }
}

async fn next_frame(a: &mut SimAgent) -> server_frame::Msg {
    a.next_frame(WAIT)
        .await
        .expect("frame in time")
        .msg
        .unwrap()
}

#[tokio::test]
async fn full_classroom_flow() {
    let s = TestServer::start_tls().await;
    assert!(s.url().starts_with("https://") && s.pin().is_some());

    // 1. Admin, room, teacher, room code for 3 computers.
    let admin = s.admin_token().await;
    let mut a = AdminClient::new(s.grpc().await);
    let room = a
        .create_room(req(
            &admin,
            CreateRoomRequest {
                name: "Sala 12".into(),
            },
        ))
        .await
        .unwrap()
        .into_inner();
    let ion = s.teacher_token("ion", &[room.id]).await;
    let profiles = a
        .list_profiles(req(&admin, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .profiles;
    let id_of = |n: &str| profiles.iter().find(|p| p.name == n).unwrap().id;
    let (lesson, exam) = (id_of("Lecție"), id_of("Examen"));
    let code = a
        .create_enroll_code(req(
            &admin,
            CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollAgent as i32,
                room_id: room.id,
                max_uses: 3,
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code;

    // 2. Three agents enroll into the room with "Lecție".
    let mut agents = Vec::new();
    for _ in 0..3 {
        let ag = SimAgent::enroll(&s, &code, &["message", "power.shutdown"])
            .await
            .unwrap();
        let st = ag.welcome().state.clone().unwrap();
        assert_eq!((st.room_id, st.effective_profile_id), (room.id, lesson));
        agents.push(ag);
    }
    assert!(
        SimAgent::enroll(&s, &code, &[]).await.is_err(),
        "a 4th use of a 3-use code fails"
    );

    // 3. The teacher watches: 3 computers online.
    let mut w = WatchClient::new(s.grpc().await)
        .watch(req(&ion, Empty {}))
        .await
        .unwrap()
        .into_inner();
    let snap = match next_event(&mut w).await {
        watch_event::Event::Snapshot(snap) => snap,
        e => panic!("{e:?}"),
    };
    assert_eq!(snap.computers.iter().filter(|c| c.online).count(), 3);

    // 4. Switch to "Examen": every agent gets the new state, Watch sees the room.
    let mut ctl = ControlClient::new(s.grpc().await);
    ctl.set_room_profile(req(
        &ion,
        SetRoomProfileRequest {
            room_id: room.id,
            profile_id: exam,
        },
    ))
    .await
    .unwrap();
    for ag in &mut agents {
        match next_frame(ag).await {
            server_frame::Msg::State(st) => assert_eq!(st.effective_profile_id, exam),
            m => panic!("{m:?}"),
        }
    }
    let r = until(&mut w, |e| match e {
        watch_event::Event::Room(r) => Some(r),
        _ => None,
    })
    .await;
    assert_eq!(r.active_profile_id, exam);

    // 5. Shutdown in 60 s: two say ok, one has nobody logged in.
    let cid = ctl
        .send_command(req(
            &ion,
            SendCommandRequest {
                target: Some(send_command_request::Target::RoomId(room.id)),
                command: Some(Command {
                    kind: Some(command::Kind::Shutdown(ShutdownCmd {
                        countdown_seconds: 60,
                    })),
                }),
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .command_id;
    for (i, ag) in agents.iter_mut().enumerate() {
        match next_frame(ag).await {
            server_frame::Msg::Command(c) => {
                assert_eq!(c.id, cid);
                assert!(matches!(
                    c.command.unwrap().kind,
                    Some(command::Kind::Shutdown(ShutdownCmd {
                        countdown_seconds: 60
                    }))
                ));
            }
            m => panic!("{m:?}"),
        }
        let status = if i < 2 {
            ResultStatus::ResultOk
        } else {
            ResultStatus::ResultNoUser
        };
        ag.send_result(CommandResult {
            command_id: cid.clone(),
            status: status as i32,
            detail: String::new(),
        })
        .await;
    }
    let mut final_results = BTreeMap::new();
    while final_results.len() < 3 {
        let (computer, st) = until(&mut w, |e| match e {
            watch_event::Event::CommandResult(r) if r.command_id == cid => {
                let res = r.result.unwrap();
                let st = ResultStatus::try_from(res.status).unwrap();
                (st != ResultStatus::ResultDelivered).then_some((res.computer_id, st))
            }
            _ => None,
        })
        .await;
        final_results.insert(computer, st);
    }
    s.handle.state().batcher.flush().await;
    let status = ctl
        .get_command(req(
            &ion,
            GetCommandRequest {
                command_id: cid.clone(),
            },
        ))
        .await
        .unwrap()
        .into_inner();
    let mut got: Vec<_> = status
        .results
        .iter()
        .map(|r| ResultStatus::try_from(r.status).unwrap())
        .collect();
    got.sort_by_key(|s| *s as i32);
    assert_eq!(
        got,
        [
            ResultStatus::ResultOk,
            ResultStatus::ResultOk,
            ResultStatus::ResultNoUser
        ]
    );

    // 6. The admin revokes one computer.
    let mut revoked = agents.remove(0);
    a.revoke_computer(req(&admin, ComputerIdRequest { id: revoked.id() }))
        .await
        .unwrap();
    match revoked.next_frame(WAIT).await {
        Some(ServerFrame {
            msg: Some(server_frame::Msg::Error(e)),
        }) => assert_eq!(e.code, "revoked"),
        f => panic!("{f:?}"),
    }
    let gone = until(&mut w, |e| match e {
        watch_event::Event::ComputerRemoved(id) => Some(id),
        _ => None,
    })
    .await;
    assert_eq!(gone, revoked.id());

    // 7. A clean disconnect shows up as offline within a second.
    let leaving = agents.remove(0);
    let leaving_id = leaving.id();
    let started = std::time::Instant::now();
    leaving.close().await;
    let c = until(&mut w, |e| match e {
        watch_event::Event::Computer(c) if c.id == leaving_id && !c.online => Some(c),
        _ => None,
    })
    .await;
    assert!(!c.online);
    assert!(
        started.elapsed() <= Duration::from_secs(1),
        "offline after {:?}",
        started.elapsed()
    );
}
