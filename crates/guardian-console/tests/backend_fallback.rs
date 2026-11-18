#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
#[path = "../../guardian-agent/tests/common_local/mod.rs"]
mod common_local;

use std::net::SocketAddr;
use std::time::Duration;

use common::Console;
use common_local::*;
use guardian_agent::AgentHandle;
use guardian_console::backend::{Action, Backend, BackendOptions, ConnState, Event, FallbackState};
use guardian_console::settings::{Secret, fallback_paths};
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::{CreateEnrollCodeRequest, Empty, EnrollCodeKind};
use guardian_server::testing::{ADMIN_PASSWORD, TEACHER_PASSWORD, req};

struct Setup {
    w: World,
    agents: Vec<AgentHandle>,
    _extra: ExtraAgent,
    extra_f: std::sync::Arc<guardian_platform::fake::FakePlatform>,
    udp: Vec<SocketAddr>,
}

async fn setup() -> Setup {
    let w = world().await;
    let extra = extra_agent(&w).await;
    let a1 = started_with(&w, opts_with_listener(&w.paths, &w.f)).await;
    let a2 = started_with(&w, opts_with_listener(&extra.paths, &extra.f)).await;
    let udp = vec![a1.local_addrs().unwrap().1, a2.local_addrs().unwrap().1];
    let extra_f = extra.f.clone();
    Setup {
        w,
        agents: vec![a1, a2],
        _extra: extra,
        extra_f,
        udp,
    }
}

fn opts(udp: &[SocketAddr]) -> BackendOptions {
    BackendOptions {
        fallback_broadcast: Some(udp.to_vec()),
        fallback_sweep_every: Duration::from_millis(500),
    }
}

fn console(st: &Setup) -> Console {
    Console::at_with(tempfile::tempdir().unwrap(), opts(&st.udp))
}

async fn enabled(c: &Console, code: Option<String>) -> i64 {
    c.b.send(Action::EnableFallback { code });
    let evs = c
        .until(
            |e| matches!(e, Event::FallbackEnabled(Some(_)) | Event::Error(_)),
            "fallback enabled",
        )
        .await;
    match evs.last() {
        Some(Event::FallbackEnabled(Some(id))) => *id,
        other => panic!("{other:?}"),
    }
}

async fn available(c: &Console) {
    c.until(
        |e| matches!(e, Event::Fallback(FallbackState::Available)),
        "fallback available",
    )
    .await;
}

async fn active(c: &Console) {
    c.until(
        |e| matches!(e, Event::Fallback(FallbackState::Active { .. })),
        "fallback active",
    )
    .await;
}

async fn result(c: &Console) -> Result<String, String> {
    let evs = c
        .until(
            |e| matches!(e, Event::Done(_) | Event::Error(_)),
            "a result",
        )
        .await;
    match evs.last() {
        Some(Event::Done(m)) => Ok(m.clone()),
        Some(Event::Error(m)) => Err(m.clone()),
        _ => unreachable!(),
    }
}

async fn teacher_console(st: &Setup) -> Console {
    let c = console(st);
    c.login(st.w.server(), "prof", TEACHER_PASSWORD).await;
    let code = console_code(&st.w).await;
    enabled(&c, Some(code)).await;
    c
}

async fn console_code(w: &World) -> String {
    w.admin_client()
        .await
        .create_enroll_code(req(
            &w.admin,
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
        .code
}

async fn agents_offline(st: &mut Setup) {
    st.w.stop_server().await;
    for h in &st.agents {
        assert!(
            until(Duration::from_secs(5), || h
                .status()
                .offline_for
                .is_some_and(|d| d > OFFLINE_AFTER))
            .await
        );
    }
}

#[tokio::test]
async fn admin_enables_fallback_in_one_step() {
    let st = setup().await;
    let c = console(&st);
    c.login(st.w.server(), "admin", ADMIN_PASSWORD).await;
    let id = enabled(&c, None).await;
    let list = AdminClient::new(st.w.server().grpc().await)
        .list_consoles(req(&st.w.admin, Empty {}))
        .await
        .unwrap()
        .into_inner();
    assert!(list.consoles.iter().any(|x| x.id == id));
    let (key, cache) = fallback_paths(&c.path);
    assert!(key.exists() && cache.exists());
}

#[tokio::test]
async fn teacher_enables_with_a_code() {
    let st = setup().await;
    let c = teacher_console(&st).await;
    // A wrong code is refused with the server's words.
    c.b.send(Action::EnableFallback {
        code: Some("NOPE-NOPE".into()),
    });
    assert!(result(&c).await.is_err());
}

#[tokio::test]
async fn fallback_is_available_only_when_server_is_down() {
    let mut st = setup().await;
    let c = teacher_console(&st).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !c.seen
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, Event::Fallback(FallbackState::Available))),
        "available while online"
    );
    agents_offline(&mut st).await;
    available(&c).await;
}

#[tokio::test]
async fn signed_in_user_enters_without_password() {
    let mut st = setup().await;
    let c = teacher_console(&st).await;
    agents_offline(&mut st).await;
    available(&c).await;
    c.b.send(Action::EnterFallback);
    active(&c).await;
    c.b.send(Action::SetRoomProfile {
        room_id: st.w.room,
        profile_id: st.w.exam.id,
    });
    assert_eq!(
        result(&c).await.unwrap(),
        "Examen applied on 2 of 2 computers in Sala 1."
    );
    for h in &st.agents {
        assert_eq!(h.status().profile_name, "Examen");
    }
}

#[tokio::test]
async fn teacher_signs_in_locally_and_switches_room() {
    let mut st = setup().await;
    let c = teacher_console(&st).await;
    c.b.send(Action::Logout);
    c.until(|e| matches!(e, Event::LoggedOut { .. }), "signed out")
        .await;
    let dir = c.into_dir();
    agents_offline(&mut st).await;
    // The console starts while the server is down (the window sends Connect).
    let c = Console::at_with(dir, opts(&st.udp));
    let settings = guardian_console::settings::ConsoleSettings::load(&c.path).unwrap();
    let Some(guardian_console::settings::Mode::Remote {
        server,
        cert_sha256,
    }) = settings.mode
    else {
        panic!("not remote");
    };
    c.b.send(Action::Connect {
        server,
        cert_sha256,
    });
    available(&c).await;
    c.b.send(Action::FallbackLogin {
        login: "prof".into(),
        password: Secret::new(TEACHER_PASSWORD),
    });
    active(&c).await;
    c.b.send(Action::SetRoomProfile {
        room_id: st.w.room,
        profile_id: st.w.exam.id,
    });
    assert_eq!(
        result(&c).await.unwrap(),
        "Examen applied on 2 of 2 computers in Sala 1."
    );
    let other = st.extra_f.spawn("other.exe", r"C:\x\other.exe");
    assert!(until(Duration::from_secs(2), || !st.extra_f.is_running(other)).await);
}

#[tokio::test]
async fn wrong_password_is_refused_locally() {
    let mut st = setup().await;
    let c = teacher_console(&st).await;
    agents_offline(&mut st).await;
    available(&c).await;
    c.b.send(Action::FallbackLogin {
        login: "prof".into(),
        password: Secret::new("wrong-password"),
    });
    assert_eq!(
        result(&c).await.unwrap_err(),
        "Unknown login or wrong password. Only users who have signed in on this computer while the server was reachable can work locally."
    );
}

#[tokio::test]
async fn free_is_refused_before_the_network() {
    let mut st = setup().await;
    let c = teacher_console(&st).await;
    agents_offline(&mut st).await;
    available(&c).await;
    c.b.send(Action::EnterFallback);
    active(&c).await;
    c.b.send(Action::SetRoomProfile {
        room_id: st.w.room,
        profile_id: st.w.free.id,
    });
    assert_eq!(
        result(&c).await.unwrap_err(),
        "A free profile cannot be set in fallback mode."
    );
    for h in &st.agents {
        assert_eq!(h.status().profile_name, "Lecție");
    }
}

#[tokio::test]
async fn fallback_ends_when_server_returns() {
    let mut st = setup().await;
    let c = teacher_console(&st).await;
    agents_offline(&mut st).await;
    available(&c).await;
    c.b.send(Action::EnterFallback);
    active(&c).await;
    c.b.send(Action::SetRoomProfile {
        room_id: st.w.room,
        profile_id: st.w.exam.id,
    });
    result(&c).await.unwrap();
    st.w.start_server().await;
    c.until(
        |e| matches!(e, Event::Fallback(FallbackState::Off)),
        "fallback off",
    )
    .await;
    assert!(
        c.seen
            .lock()
            .unwrap()
            .iter()
            .rev()
            .any(|e| matches!(e, Event::Connection(ConnState::Online))),
        "fallback ended before the server was online"
    );
    for h in &st.agents {
        assert!(
            until(Duration::from_secs(8), || h.status().online
                && h.status().profile_name == "Lecție")
            .await
        );
    }
}

#[tokio::test]
async fn embedded_mode_has_no_fallback() {
    let c = Console::new();
    let data = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    c.b.send(Action::StartEmbedded {
        data_dir: Some(data.path().to_path_buf()),
        port,
    });
    c.until(
        |e| matches!(e, Event::Connection(ConnState::NeedsLogin { .. })),
        "the embedded sign-in",
    )
    .await;
    c.b.send(Action::EnableFallback { code: None });
    assert_eq!(
        result(&c).await.unwrap_err(),
        "Local fallback is only for a remote server."
    );
}

#[tokio::test]
async fn forget_mode_deletes_key_and_cache() {
    let st = setup().await;
    let c = teacher_console(&st).await;
    let (key, cache) = fallback_paths(&c.path);
    assert!(key.exists() && cache.exists());
    c.b.send(Action::ForgetMode);
    c.until(
        |e| matches!(e, Event::FallbackEnabled(None)),
        "fallback forgotten",
    )
    .await;
    assert!(!key.exists() && !cache.exists());
}

#[tokio::test]
async fn fallback_from_sign_in_screen_ends_when_server_returns() {
    let mut st = setup().await;
    let c = teacher_console(&st).await;
    c.b.send(Action::Logout);
    c.until(
        |e| matches!(e, Event::Connection(ConnState::NeedsLogin { .. })),
        "the sign-in screen",
    )
    .await;
    agents_offline(&mut st).await;
    // Sitting on the sign-in screen, the teacher tries to sign in.
    c.b.send(Action::Login {
        login: "prof".into(),
        password: Secret::new(TEACHER_PASSWORD),
    });
    available(&c).await;
    c.b.send(Action::FallbackLogin {
        login: "prof".into(),
        password: Secret::new(TEACHER_PASSWORD),
    });
    active(&c).await;
    st.w.start_server().await;
    c.until_within(
        Duration::from_secs(45),
        |e| matches!(e, Event::Fallback(FallbackState::Off)),
        "fallback off",
    )
    .await;
}

#[tokio::test]
async fn late_switch_result_does_not_reenter_fallback() {
    let mut st = setup().await;
    let c = teacher_console(&st).await;
    agents_offline(&mut st).await;
    // One computer is gone: switching the room waits for its timeouts.
    st.agents.pop().unwrap().shutdown().await;
    available(&c).await;
    c.b.send(Action::EnterFallback);
    active(&c).await;
    c.b.send(Action::SetRoomProfile {
        room_id: st.w.room,
        profile_id: st.w.exam.id,
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    c.b.send(Action::LeaveFallback);
    c.until(
        |e| matches!(e, Event::Fallback(FallbackState::Available)),
        "left fallback",
    )
    .await;
    let after = c
        .until(
            |e| matches!(e, Event::Done(_) | Event::Error(_)),
            "the switch result",
        )
        .await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let mut late = after;
    late.extend(c.drain());
    assert!(
        !late
            .iter()
            .any(|e| matches!(e, Event::Fallback(FallbackState::Active { .. }))),
        "fallback mode came back after leaving it: {late:?}"
    );
}

#[tokio::test]
async fn fallback_can_be_enabled_again_after_revoke() {
    let st = setup().await;
    let c = teacher_console(&st).await;
    let first = match c.seen.lock().unwrap().iter().rev().find_map(|e| match e {
        Event::FallbackEnabled(Some(id)) => Some(*id),
        _ => None,
    }) {
        Some(id) => id,
        None => panic!("not enabled"),
    };
    AdminClient::new(st.w.server().grpc().await)
        .revoke_console(req(
            &st.w.admin,
            guardian_proto::v1::IdRequest { id: first },
        ))
        .await
        .unwrap();
    let code = console_code(&st.w).await;
    let second = enabled(&c, Some(code)).await;
    assert_ne!(first, second);
}
