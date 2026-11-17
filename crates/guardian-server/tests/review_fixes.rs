#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Regression tests for the final-review findings.

use std::time::Duration;

use guardian_core::keys::KeyPair;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::control_client::ControlClient;
use guardian_proto::v1::{
    Command, ComputerIds, CreateEnrollCodeRequest, CreateRoomRequest, Empty, EnrollCodeKind,
    LoginRequest, MessageCmd, SendCommandRequest, SetRoomProfileRequest, command,
    send_command_request, server_frame,
};
use guardian_server::ServerConfig;
use guardian_server::testing::{SimAgent, TestServer, req};

/// Finding 2: a command queued on a connection that dies before the agent read it
/// is delivered again on reconnect.
#[tokio::test]
async fn command_on_dead_connection_is_resent_on_reconnect() {
    let s = TestServer::start().await;
    let t = s.admin_token().await;
    let mut a = AdminClient::new(s.grpc().await);
    let code = a
        .create_enroll_code(req(
            &t,
            CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollAgent as i32,
                ..Default::default()
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code;
    let old = SimAgent::enroll(&s, &code, &["message"]).await.unwrap();
    let (id, key) = (old.id(), KeyPair::from_seed(&old.key().seed()));
    old.go_silent(); // half-open: the socket is registered, nothing is read
    let cid = ControlClient::new(s.grpc().await)
        .send_command(req(
            &t,
            SendCommandRequest {
                target: Some(send_command_request::Target::Computers(ComputerIds {
                    ids: vec![id],
                })),
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
    let fresh = SimAgent::connect(&s, id, &key, &["message"]).await.unwrap();
    assert_eq!(
        fresh
            .welcome()
            .pending
            .iter()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>(),
        [cid]
    );
}

/// Finding 3: a whole room enrolling through one NAT address is not rate limited;
/// only failed attempts count.
#[tokio::test]
async fn room_behind_one_ip_can_enroll_and_teachers_can_log_in() {
    let s = TestServer::start_with(|c: &mut ServerConfig| c.auth_rate_per_minute = 10).await;
    let t = s.admin_token().await;
    let mut a = AdminClient::new(s.grpc().await);
    let room = a
        .create_room(req(&t, CreateRoomRequest { name: "R".into() }))
        .await
        .unwrap()
        .into_inner();
    let code = a
        .create_enroll_code(req(
            &t,
            CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollAgent as i32,
                room_id: room.id,
                max_uses: 30,
                ttl_seconds: 0,
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code;
    let mut tasks = Vec::new();
    for _ in 0..30 {
        let (url_s, code) = (&s, code.clone());
        tasks.push(async move { SimAgent::enroll(url_s, &code, &[]).await });
    }
    let results = futures_util::future::join_all(tasks).await;
    let failed: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    assert!(failed.is_empty(), "{failed:?}");
    let mut auth = AuthClient::new(s.grpc().await);
    for _ in 0..15 {
        auth.login(LoginRequest {
            login: "admin".into(),
            password: "admin-password".into(),
        })
        .await
        .unwrap();
    }
}

/// Finding 1: agents that reconnect while the profile keeps changing converge on
/// the room's real profile (the agent ignores a State that is not newer).
#[tokio::test]
async fn welcome_and_pushes_converge_under_concurrent_profile_changes() {
    let s = TestServer::start().await;
    let t = s.admin_token().await;
    let mut a = AdminClient::new(s.grpc().await);
    let room = a
        .create_room(req(&t, CreateRoomRequest { name: "R".into() }))
        .await
        .unwrap()
        .into_inner();
    let profiles = a
        .list_profiles(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .profiles;
    let ids: Vec<i64> = profiles.iter().map(|p| p.id).collect();
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
    let first = SimAgent::enroll(&s, &code, &[]).await.unwrap();
    let (id, key) = (first.id(), KeyPair::from_seed(&first.key().seed()));
    first.close().await;
    let flipper = {
        let ch = s.grpc().await;
        let t = t.clone();
        let ids = ids.clone();
        tokio::spawn(async move {
            let mut ctl = ControlClient::new(ch);
            for i in 0..60 {
                ctl.set_room_profile(req(
                    &t,
                    SetRoomProfileRequest {
                        room_id: room.id,
                        profile_id: ids[i % 3],
                    },
                ))
                .await
                .unwrap();
            }
        })
    };
    for _ in 0..20 {
        let mut ag = SimAgent::connect(&s, id, &key, &[]).await.unwrap();
        let st = ag.welcome().state.clone().unwrap();
        let (mut ver, mut prof) = (st.version, st.effective_profile_id);
        // Drain pushes for a moment, applying only newer versions.
        while let Some(f) = ag.next_frame(Duration::from_millis(30)).await {
            if let Some(server_frame::Msg::State(st)) = f.msg
                && st.version > ver
            {
                (ver, prof) = (st.version, st.effective_profile_id);
            }
        }
        if flipper.is_finished() {
            // After the last change, what the agent applied must equal the room.
            while let Some(f) = ag.next_frame(Duration::from_millis(200)).await {
                if let Some(server_frame::Msg::State(st)) = f.msg
                    && st.version > ver
                {
                    (ver, prof) = (st.version, st.effective_profile_id);
                }
            }
            let actual = a
                .list_rooms(req(&t, Empty {}))
                .await
                .unwrap()
                .into_inner()
                .rooms[0]
                .active_profile_id;
            assert_eq!(
                prof, actual,
                "agent applied a stale profile (version {ver})"
            );
        }
        ag.close().await;
    }
    flipper.await.unwrap();
    let mut ag = SimAgent::connect(&s, id, &key, &[]).await.unwrap();
    let st = ag.welcome().state.clone().unwrap();
    let actual = a
        .list_rooms(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .rooms[0]
        .active_profile_id;
    assert_eq!(st.effective_profile_id, actual);
    let _ = ag.next_frame(Duration::from_millis(10)).await;
}
