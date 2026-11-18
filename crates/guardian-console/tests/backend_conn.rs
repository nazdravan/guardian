#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::*;
use guardian_console::backend::{Action, Backend, ConnState, Event};
use guardian_console::settings::{ConsoleSettings, Secret};
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::{CreateUserRequest, Role, UpdateUserRequest};
use guardian_server::testing::{ADMIN_PASSWORD, SimAgent, TestServer, req};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn login_then_snapshot() {
    let s = TestServer::start().await;
    let _ = s.admin_token().await;
    let c = Console::new();
    let user = c.login(&s, "admin", ADMIN_PASSWORD).await;
    assert_eq!(user.role, Role::Admin as i32);
    let fleet = c.fleet_where(|f| f.profiles().len() == 3).await;
    let names: Vec<&str> = fleet.profiles().iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Examen", "Lecție", "Liber"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_password_is_an_error() {
    let s = TestServer::start().await;
    let _ = s.admin_token().await;
    let c = Console::new();
    c.connect(&s).await;
    c.b.send(Action::Login {
        login: "admin".into(),
        password: Secret::new("wrong-password"),
    });
    let evs = c.until(|e| matches!(e, Event::Error(_)), "an error").await;
    let Some(Event::Error(msg)) = evs.last() else {
        panic!()
    };
    assert!(!msg.contains("session has ended"), "{msg}");
    assert!(!evs.iter().any(|e| matches!(e, Event::LoggedIn(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_token_skips_login() {
    let s = TestServer::start().await;
    let _ = s.admin_token().await;
    let c = Console::new();
    c.login(&s, "admin", ADMIN_PASSWORD).await;
    let path = c.path.clone();
    let dir = c.into_dir();
    let saved = ConsoleSettings::load(&path).unwrap();
    assert!(saved.token().is_some());
    let c2 = Console::at(dir);
    c2.b.send(Action::Connect {
        server: s.url(),
        cert_sha256: None,
    });
    c2.until(
        |e| matches!(e, Event::LoggedIn(_)),
        "logged in from the saved token",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_coming_online_updates_the_fleet() {
    let s = TestServer::start().await;
    let t = s.admin_token().await;
    let c = Console::new();
    c.login(&s, "admin", ADMIN_PASSWORD).await;
    let (room, code) = room_with_code(&s, &t, "Sala 1").await;
    let agent = SimAgent::enroll(&s, &code, &[]).await.unwrap();
    let f = c
        .fleet_where(|f| f.rooms().iter().any(|r| r.id == room && r.online == 1))
        .await;
    assert_eq!(f.rooms()[0].name, "Sala 1");
    drop(agent);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_session_asks_for_login_again() {
    let s = TestServer::start().await;
    let t = s.admin_token().await;
    let mut a = AdminClient::new(s.grpc().await);
    let prof = a
        .create_user(req(
            &t,
            CreateUserRequest {
                login: "prof".into(),
                password: "prof-password-1".into(),
                role: Role::Admin as i32,
            },
        ))
        .await
        .unwrap()
        .into_inner();
    let c = Console::new();
    c.login(&s, "prof", "prof-password-1").await;
    c.fleet_where(|_| true).await;
    a.update_user(req(
        &t,
        UpdateUserRequest {
            id: prof.id,
            active: Some(false),
            ..Default::default()
        },
    ))
    .await
    .unwrap();
    let evs = c
        .until(
            |e| matches!(e, Event::Connection(ConnState::NeedsLogin { .. })),
            "asked to sign in again",
        )
        .await;
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::LoggedOut { reason: Some(_) })),
        "{evs:?}"
    );
    assert!(ConsoleSettings::load(&c.path).unwrap().token().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_restart_reconnects_and_replaces_the_snapshot() {
    let s = TestServer::start().await;
    let t = s.admin_token().await;
    let (_room, code) = room_with_code(&s, &t, "Sala 1").await;
    let agent = SimAgent::enroll(&s, &code, &[]).await.unwrap();
    let c = Console::new();
    c.login(&s, "admin", ADMIN_PASSWORD).await;
    c.fleet_where(|f| f.totals() == (1, 1)).await;
    drop(agent);
    let s = s.restart_same_port().await;
    c.until(
        |e| matches!(e, Event::Connection(ConnState::Reconnecting { .. })),
        "reconnecting",
    )
    .await;
    c.until(
        |e| matches!(e, Event::Connection(ConnState::Online)),
        "back online",
    )
    .await;
    // The fresh snapshot: the agent is gone, so it is offline now.
    c.fleet_where(|f| f.totals() == (0, 1)).await;
    drop(s);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logout_clears_the_token() {
    let s = TestServer::start().await;
    let _ = s.admin_token().await;
    let c = Console::new();
    c.login(&s, "admin", ADMIN_PASSWORD).await;
    c.b.send(Action::Logout);
    c.until(
        |e| matches!(e, Event::LoggedOut { reason: None }),
        "logged out",
    )
    .await;
    assert!(ConsoleSettings::load(&c.path).unwrap().token().is_none());
    let _ = Duration::ZERO;
}
