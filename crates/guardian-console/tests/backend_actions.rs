#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::{Duration, Instant};

use common::*;
use guardian_console::backend::{Action, Backend, Event};
use guardian_console::settings::Secret;
use guardian_proto::v1::{Mode, Profile, Role, Settings, server_frame};
use guardian_server::testing::{ADMIN_PASSWORD, SimAgent, TestServer};

async fn admin_console() -> (TestServer, String, Console) {
    let s = TestServer::start().await;
    let t = s.admin_token().await;
    let c = Console::new();
    c.login(&s, "admin", ADMIN_PASSWORD).await;
    c.fleet_where(|f| f.profiles().len() == 3).await;
    (s, t, c)
}

async fn done(c: &Console) -> String {
    let evs = c
        .until(
            |e| matches!(e, Event::Done(_) | Event::Error(_)),
            "the action's result",
        )
        .await;
    match evs.last() {
        Some(Event::Done(m)) => m.clone(),
        other => panic!("action failed: {other:?}"),
    }
}

fn profile_id(f: &guardian_console::fleet::Fleet, name: &str) -> i64 {
    f.profiles().iter().find(|p| p.name == name).unwrap().id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn room_lifecycle() {
    let (_s, _t, c) = admin_console().await;
    c.b.send(Action::CreateRoom {
        name: "Sala 1".into(),
    });
    done(&c).await;
    let f = c.fleet_where(|f| f.rooms().len() == 1).await;
    let id = f.rooms()[0].id;
    c.b.send(Action::RenameRoom {
        id,
        name: "Sala 2".into(),
    });
    done(&c).await;
    c.fleet_where(|f| f.rooms().first().is_some_and(|r| r.name == "Sala 2"))
        .await;
    c.b.send(Action::DeleteRoom { id });
    done(&c).await;
    c.fleet_where(|f| f.rooms().is_empty()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_room_profile_reaches_the_agent() {
    let (s, t, c) = admin_console().await;
    let (room, code) = room_with_code(&s, &t, "Sala 1").await;
    let mut agent = SimAgent::enroll(&s, &code, &[]).await.unwrap();
    let f = c.fleet_where(|f| f.totals() == (1, 1)).await;
    let exam = profile_id(&f, "Examen");
    c.b.send(Action::SetRoomProfile {
        room_id: room,
        profile_id: exam,
    });
    done(&c).await;
    c.fleet_where(|f| {
        f.rooms()
            .first()
            .and_then(|r| r.profile.as_ref())
            .is_some_and(|p| p.id == exam)
    })
    .await;
    let end = Instant::now() + WAIT;
    loop {
        assert!(Instant::now() < end, "the agent never got the Examen state");
        if let Some(fr) = agent.next_frame(Duration::from_millis(500)).await
            && let Some(server_frame::Msg::State(st)) = fr.msg
            && st.effective_profile_id == exam
        {
            break;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn profile_edit_refreshes_profiles() {
    let (_s, _t, c) = admin_console().await;
    c.b.send(Action::SaveProfile(Profile {
        id: 0,
        name: "Test".into(),
        mode: Mode::Blacklist as i32,
        apps: vec!["game.exe".into()],
    }));
    done(&c).await;
    let evs = c
        .until(
            |e| matches!(e, Event::Profiles(p) if p.iter().any(|p| p.name == "Test")),
            "the new profile",
        )
        .await;
    let Some(Event::Profiles(ps)) = evs.last() else {
        panic!()
    };
    let mut test = ps.iter().find(|p| p.name == "Test").unwrap().clone();
    test.apps.push("other.exe".into());
    c.b.send(Action::SaveProfile(test.clone()));
    done(&c).await;
    c.until(
        |e| matches!(e, Event::Profiles(p) if p.iter().any(|p| p.apps.len() == 2)),
        "the edited profile",
    )
    .await;
    c.b.send(Action::DeleteProfile { id: test.id });
    done(&c).await;
    c.until(
        |e| matches!(e, Event::Profiles(p) if !p.iter().any(|p| p.name == "Test")),
        "the profile gone",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computer_move_and_exception() {
    let (s, t, c) = admin_console().await;
    let (room, code) = room_with_code(&s, &t, "Sala 1").await;
    let agent = SimAgent::enroll(&s, &code, &[]).await.unwrap();
    let f = c.fleet_where(|f| f.totals() == (1, 1)).await;
    let free = profile_id(&f, "Liber");
    c.b.send(Action::UpdateComputer {
        id: agent.id(),
        name: Some("Catedra".into()),
        room_id: None,
        exception_profile_id: Some(free),
    });
    done(&c).await;
    let f = c
        .fleet_where(|f| f.computers().first().is_some_and(|c| c.exception))
        .await;
    assert_eq!(f.computers()[0].name, "Catedra");
    assert_eq!(f.computers()[0].room_id, room);
    c.b.send(Action::UpdateComputer {
        id: agent.id(),
        name: None,
        room_id: Some(0),
        exception_profile_id: Some(0),
    });
    done(&c).await;
    c.fleet_where(|f| f.unassigned().len() == 1 && !f.computers()[0].exception)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoke_removes_the_computer() {
    let (s, t, c) = admin_console().await;
    let (_room, code) = room_with_code(&s, &t, "Sala 1").await;
    let agent = SimAgent::enroll(&s, &code, &[]).await.unwrap();
    c.fleet_where(|f| f.totals() == (1, 1)).await;
    c.b.send(Action::RevokeComputer { id: agent.id() });
    done(&c).await;
    c.fleet_where(|f| f.totals() == (0, 0)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_lifecycle() {
    let (s, t, c) = admin_console().await;
    let (room, _code) = room_with_code(&s, &t, "Sala 1").await;
    c.b.send(Action::CreateUser {
        login: "ion".into(),
        password: Secret::new("teacher-pass-1"),
        role: Role::Teacher,
    });
    done(&c).await;
    let evs = c
        .until(
            |e| matches!(e, Event::Users(u) if u.iter().any(|u| u.login == "ion")),
            "the new user",
        )
        .await;
    let Some(Event::Users(us)) = evs.last() else {
        panic!()
    };
    let ion = us.iter().find(|u| u.login == "ion").unwrap().clone();
    c.b.send(Action::SetUserRooms {
        user_id: ion.id,
        room_ids: vec![room],
    });
    done(&c).await;
    c.b.send(Action::UpdateUser {
        id: ion.id,
        role: None,
        active: Some(false),
        new_password: None,
    });
    done(&c).await;
    c.until(
        |e| matches!(e, Event::Users(u) if u.iter().any(|u| u.login == "ion" && !u.active && u.room_ids == [room])),
        "ion disabled, with the room",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_code_is_returned() {
    let (s, t, c) = admin_console().await;
    let (room, _) = room_with_code(&s, &t, "Sala 1").await;
    c.b.send(Action::CreateAgentCode {
        room_id: room,
        max_uses: 50,
        ttl_seconds: 86400,
    });
    let evs = c
        .until(|e| matches!(e, Event::Code(_) | Event::Error(_)), "a code")
        .await;
    let Some(Event::Code(code)) = evs.last() else {
        panic!("{evs:?}")
    };
    assert!(!code.code.is_empty());
    assert_eq!((code.max_uses, code.room_id), (50, room));
    SimAgent::enroll(&s, &code.code, &[]).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_roundtrip() {
    let (_s, _t, c) = admin_console().await;
    c.b.send(Action::LoadSettings);
    let evs = c
        .until(|e| matches!(e, Event::Settings(_)), "settings")
        .await;
    let Some(Event::Settings(mut st)) = evs.last().cloned() else {
        panic!()
    };
    st.retention_days = 30;
    c.b.send(Action::SaveSettings(st));
    c.until(
        |e| {
            matches!(
                e,
                Event::Settings(Settings {
                    retention_days: 30,
                    ..
                })
            )
        },
        "saved settings",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn change_password_then_login_with_new() {
    let (s, _t, c) = admin_console().await;
    c.b.send(Action::ChangePassword {
        old: Secret::new(ADMIN_PASSWORD),
        new: Secret::new("brand-new-password"),
    });
    done(&c).await;
    let c2 = Console::new();
    c2.login(&s, "admin", "brand-new-password").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_denied_is_shown() {
    let s = TestServer::start().await;
    let t = s.admin_token().await;
    let (room, _) = room_with_code(&s, &t, "Sala 1").await;
    let _ = s.teacher_token("ion", &[]).await;
    let c = Console::new();
    c.login(&s, "ion", guardian_server::testing::TEACHER_PASSWORD)
        .await;
    c.b.send(Action::SetRoomProfile {
        room_id: room,
        profile_id: 1,
    });
    let evs = c
        .until(|e| matches!(e, Event::Error(_) | Event::Done(_)), "result")
        .await;
    assert!(
        matches!(evs.last(), Some(Event::Error(m)) if m == "You are not allowed to do that."),
        "{evs:?}"
    );
}
