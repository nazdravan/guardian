#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::*;
use guardian_console::backend::{Action, Backend, ConnState, Event};
use guardian_console::embedded::EmbeddedState;
use guardian_console::embedded::parse_whoami_sid;
use guardian_console::firewall::firewall_args;
use guardian_console::instance::{InstanceError, InstanceLock};
use guardian_console::settings::{ConsoleSettings, Mode, Secret};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start(c: &Console, data: &std::path::Path, port: u16) {
    c.b.send(Action::StartEmbedded {
        data_dir: Some(data.to_path_buf()),
        port,
    });
}

async fn running_pin(c: &Console) -> String {
    let evs = c
        .until(
            |e| {
                matches!(
                    e,
                    Event::Embedded(EmbeddedState::Running { .. } | EmbeddedState::Failed { .. })
                )
            },
            "the embedded server",
        )
        .await;
    match evs.last() {
        Some(Event::Embedded(EmbeddedState::Running { cert_sha256, .. })) => cert_sha256.clone(),
        other => panic!("not running: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedded_first_run_creates_admin_and_lists_profiles() {
    let data = tempfile::tempdir().unwrap();
    let c = Console::new();
    start(&c, data.path(), free_port());
    let pin = running_pin(&c).await;
    assert_eq!(pin.len(), 64);
    assert!(pin.chars().all(|ch| ch.is_ascii_hexdigit()));
    c.until(
        |e| {
            matches!(
                e,
                Event::Connection(ConnState::NeedsLogin { can_init: true })
            )
        },
        "the first-admin prompt",
    )
    .await;
    c.b.send(Action::Init {
        bootstrap_code: String::new(),
        login: "admin".into(),
        password: Secret::new("admin-password-1"),
    });
    c.until(|e| matches!(e, Event::LoggedIn(_)), "logged in")
        .await;
    let f = c.fleet_where(|f| f.profiles().len() == 3).await;
    assert_eq!(f.profiles()[1].name, "Lecție");
    let s = ConsoleSettings::load(&c.path).unwrap();
    assert!(matches!(s.mode, Some(Mode::Embedded { .. })));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn embedded_second_run_keeps_data_and_pin() {
    let data = tempfile::tempdir().unwrap();
    let port = free_port();
    let c = Console::new();
    start(&c, data.path(), port);
    let pin1 = running_pin(&c).await;
    c.until(
        |e| matches!(e, Event::Connection(ConnState::NeedsLogin { .. })),
        "sign-in",
    )
    .await;
    c.b.send(Action::Init {
        bootstrap_code: String::new(),
        login: "admin".into(),
        password: Secret::new("admin-password-1"),
    });
    c.until(|e| matches!(e, Event::LoggedIn(_)), "logged in")
        .await;
    let dir = c.into_dir();
    let c = Console::at(dir);
    start(&c, data.path(), port);
    assert_eq!(running_pin(&c).await, pin1);
    // The saved session resumes; no first-admin prompt.
    let evs = c
        .until(|e| matches!(e, Event::LoggedIn(_)), "resumed session")
        .await;
    assert!(!evs.iter().any(|e| matches!(
        e,
        Event::Connection(ConnState::NeedsLogin { can_init: true })
    )));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn port_in_use_is_reported_and_never_running() {
    let data = tempfile::tempdir().unwrap();
    let busy = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let port = busy.local_addr().unwrap().port();
    let c = Console::new();
    start(&c, data.path(), port);
    let evs = c
        .until(
            |e| matches!(e, Event::Embedded(EmbeddedState::Failed { .. })),
            "the failure",
        )
        .await;
    let Some(Event::Embedded(EmbeddedState::Failed { reason })) = evs.last() else {
        panic!()
    };
    assert!(reason.contains("already in use"), "{reason}");
    assert!(
        !evs.iter()
            .any(|e| matches!(e, Event::Embedded(EmbeddedState::Running { .. })))
    );
    drop(busy);
}

#[test]
fn second_instance_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let first = InstanceLock::acquire(d.path()).unwrap();
    assert!(matches!(
        InstanceLock::acquire(d.path()),
        Err(InstanceError::AlreadyRunning)
    ));
    drop(first);
    assert!(InstanceLock::acquire(d.path()).is_ok());
}

#[test]
fn firewall_args_are_exact() {
    assert_eq!(
        firewall_args(8443),
        [
            "advfirewall",
            "firewall",
            "add",
            "rule",
            "name=Guardian Console server",
            "dir=in",
            "action=allow",
            "protocol=TCP",
            "localport=8443",
            "profile=any",
            "remoteip=localsubnet",
        ]
    );
}

#[test]
fn whoami_sid_is_parsed() {
    assert_eq!(
        parse_whoami_sid("\"pc\\elev\",\"S-1-5-21-1-2-3-1001\"\r\n").as_deref(),
        Some("S-1-5-21-1-2-3-1001")
    );
    assert_eq!(parse_whoami_sid("garbage"), None);
}
