#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};

use guardian_console::connect_info::{
    AgentTarget, agent_target, lan_fallback, linux_command, no_agent_warning, windows_command,
};
use guardian_console::embedded::EmbeddedState;
use guardian_console::settings::Mode;

fn pin() -> String {
    "ab".repeat(32)
}

#[test]
fn remote_target() {
    let t = agent_target(
        &Mode::Remote {
            server: "https://srv.school:8443".into(),
            cert_sha256: Some(pin()),
        },
        "PROF-PC",
        None,
    );
    assert_eq!(
        t,
        AgentTarget {
            server: "wss://srv.school:8443".into(),
            cert_sha256: Some(pin())
        }
    );
}

#[test]
fn embedded_target() {
    let t = agent_target(&Mode::Embedded { port: 8443 }, "PROF-PC", None);
    assert_eq!(t.server, "wss://PROF-PC:8443");
    assert_eq!(t.cert_sha256, None);
    let t = agent_target(&Mode::Embedded { port: 8443 }, "PROF-PC", Some(9443));
    assert_eq!(t.server, "wss://PROF-PC:9443");
}

#[test]
fn windows_command_with_and_without_pin() {
    let mut t = AgentTarget {
        server: "wss://srv:8443".into(),
        cert_sha256: Some(pin()),
    };
    assert_eq!(
        windows_command(&t, "ABCD-1234"),
        format!(
            "guardian-agent.exe enroll --server wss://srv:8443 --cert-sha256 {} --code ABCD-1234\nguardian-agent.exe service install",
            pin()
        )
    );
    t.cert_sha256 = None;
    assert_eq!(
        windows_command(&t, "C"),
        "guardian-agent.exe enroll --server wss://srv:8443 --code C\nguardian-agent.exe service install"
    );
}

#[test]
fn linux_command_order() {
    let t = AgentTarget {
        server: "wss://srv:8443".into(),
        cert_sha256: Some(pin()),
    };
    assert_eq!(
        linux_command(&t, "C"),
        format!("sudo ./install.sh wss://srv:8443 C {}", pin())
    );
    let t = AgentTarget {
        cert_sha256: None,
        ..t
    };
    assert_eq!(linux_command(&t, "C"), "sudo ./install.sh wss://srv:8443 C");
}

#[test]
fn lan_fallback_filters_public_and_v6() {
    let addrs = [
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)),
        IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
        IpAddr::V4(Ipv4Addr::new(172, 20, 1, 1)),
        IpAddr::V4(Ipv4Addr::new(172, 40, 1, 1)),
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ];
    assert_eq!(
        lan_fallback(&addrs, 8443),
        [
            "wss://192.168.1.20:8443",
            "wss://10.0.0.5:8443",
            "wss://172.20.1.1:8443"
        ]
    );
}

#[test]
fn no_agent_warning_after_two_minutes() {
    let now = Instant::now() + Duration::from_secs(1000);
    let running = |secs: u64| EmbeddedState::Running {
        port: 8443,
        cert_sha256: pin(),
        started_at: now - Duration::from_secs(secs),
    };
    assert!(!no_agent_warning(&running(119), false, now));
    assert!(no_agent_warning(&running(120), false, now));
    assert!(!no_agent_warning(&running(500), true, now));
    assert!(!no_agent_warning(
        &EmbeddedState::Failed { reason: "x".into() },
        false,
        now
    ));
}
