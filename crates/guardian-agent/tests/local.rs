#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common_local;

use std::time::{Duration, Instant};

use common_local::*;
use guardian_agent::run;
use guardian_agent::store::load_local;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::{
    Command, CommandEnvelope, Empty, LocalActor, LocalReply, LocalRequest, LocalSetProfile,
    MessageCmd, ResultStatus, SetUserRoomsRequest, StatusRequest, UpdateComputerRequest, command,
    local_reply, local_request,
};
use guardian_server::testing::{TestServer, req};

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

#[tokio::test]
async fn late_local_switch_does_not_beat_the_server() {
    let mut w = world().await;
    let h = std::sync::Arc::new(started(&w).await);
    for round in 0..10 {
        offline(&mut w, &h).await;
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let hammer = {
            let (h, stop, teacher, room, exam) = (
                h.clone(),
                stop.clone(),
                w.teacher.clone(),
                w.room,
                w.exam.id,
            );
            tokio::task::spawn_blocking(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    h.local_request(1, set_profile(&teacher, room, exam));
                }
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        w.start_server().await;
        assert!(
            until(Duration::from_secs(8), || h.status().online).await,
            "round {round}: never online"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        hammer.await.unwrap();
        // The engine enforces Lecție: a program Examen would kill keeps running.
        let other = w.f.spawn("other.exe", r"C:\x\other.exe");
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(
            w.f.is_running(other),
            "round {round}: the late local switch beat the server"
        );
        w.f.remove(other);
    }
    std::sync::Arc::try_unwrap(h).unwrap().shutdown().await;
}
