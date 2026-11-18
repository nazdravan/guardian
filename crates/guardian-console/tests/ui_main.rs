#![cfg(feature = "testing")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui::accesskit::Role as A11y;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use guardian_console::backend::{Action, ConnState, Event};
use guardian_console::embedded::EmbeddedState;
use guardian_console::fleet::Fleet;
use guardian_console::settings::{ConsoleSettings, Mode};
use guardian_console::testing::FakeBackend;
use guardian_console::ui::ConsoleApp;
use guardian_proto::v1::{
    Computer, Mode as PMode, Profile, Role, Room, Snapshot, User, WatchEvent, watch_event,
};

type App = ConsoleApp<FakeBackend>;

fn harness(settings: ConsoleSettings) -> (Harness<'static, App>, FakeBackend) {
    let fake = FakeBackend::default();
    let app = ConsoleApp::new(fake.clone(), settings);
    let h = Harness::builder()
        .with_size((1100.0, 720.0))
        .build_ui_state(|ui, app: &mut App| app.show(ui), app);
    (h, fake)
}

fn remote() -> ConsoleSettings {
    let mut s = ConsoleSettings::default();
    s.mode = Some(Mode::Remote {
        server: "https://srv.school:8443".into(),
        cert_sha256: None,
    });
    s
}

fn user(role: Role) -> User {
    User {
        id: 1,
        login: "prof".into(),
        role: role as i32,
        active: true,
        room_ids: vec![10],
    }
}

fn fleet() -> Fleet {
    let mut f = Fleet::default();
    f.apply(WatchEvent {
        event: Some(watch_event::Event::Snapshot(Snapshot {
            rooms: vec![Room {
                id: 10,
                name: "Sala 1".into(),
                active_profile_id: 2,
            }],
            computers: vec![
                Computer {
                    id: 1,
                    name: "pc-01".into(),
                    room_id: 10,
                    online: true,
                    effective_profile_id: 2,
                    ..Default::default()
                },
                Computer {
                    id: 2,
                    name: "pc-02".into(),
                    room_id: 10,
                    online: false,
                    last_seen_unix: 1,
                    effective_profile_id: 2,
                    ..Default::default()
                },
            ],
            profiles: vec![
                Profile {
                    id: 1,
                    name: "Liber".into(),
                    mode: PMode::Free as i32,
                    apps: vec![],
                },
                Profile {
                    id: 2,
                    name: "Lecție".into(),
                    mode: PMode::Blacklist as i32,
                    apps: vec![],
                },
                Profile {
                    id: 3,
                    name: "Examen".into(),
                    mode: PMode::Whitelist as i32,
                    apps: vec![],
                },
            ],
        })),
    });
    f
}

fn signed_in(role: Role) -> (Harness<'static, App>, FakeBackend) {
    let (mut h, fake) = harness(remote());
    fake.push(Event::Connection(ConnState::Online));
    fake.push(Event::LoggedIn(user(role)));
    fake.push(Event::Fleet(fleet()));
    h.run();
    (h, fake)
}

fn type_into(h: &mut Harness<'static, App>, label: &str, text: &str) {
    h.get_by_label(label).focus();
    h.run();
    h.get_by_label(label).type_text(text);
    h.run();
}

fn pick_profile(h: &mut Harness<'static, App>, name: &str) {
    h.get_by_role_and_label(A11y::ComboBox, "Profile").click();
    h.run();
    h.get_by_role(A11y::ListBox).get_by_label(name).click();
    h.run();
}

#[test]
fn setup_remote_sends_connect() {
    let (mut h, fake) = harness(ConsoleSettings::default());
    h.get_by_label("Connect to a server").click();
    h.run();
    type_into(&mut h, "Server address", "https://srv.school");
    h.get_by_label("Connect").click();
    h.run();
    assert!(
        fake.sent().iter().any(|a| matches!(
            a,
            Action::Connect { server, cert_sha256: None } if server == "https://srv.school:8443"
        )),
        "{:?}",
        fake.sent()
    );
}

#[test]
fn setup_rejects_http() {
    let (mut h, fake) = harness(ConsoleSettings::default());
    h.get_by_label("Connect to a server").click();
    h.run();
    type_into(&mut h, "Server address", "http://x");
    h.run();
    h.get_by_label("Connect").click();
    h.run();
    h.get_by_label_contains("Use https://");
    assert!(fake.sent().is_empty(), "{:?}", fake.sent());
}

#[test]
fn setup_embedded_sends_start() {
    let (mut h, fake) = harness(ConsoleSettings::default());
    h.get_by_label("Run the server on this computer").click();
    h.run();
    h.get_by_label("Start the server").click();
    h.run();
    assert!(fake.sent().iter().any(|a| matches!(
        a,
        Action::StartEmbedded {
            data_dir: None,
            port: 8443
        }
    )));
}

#[test]
fn first_admin_needs_matching_passwords() {
    let mut s = ConsoleSettings::default();
    s.mode = Some(Mode::Embedded { port: 8443 });
    let (mut h, fake) = harness(s);
    fake.push(Event::Connection(ConnState::NeedsLogin { can_init: true }));
    h.run();
    h.get_by_label_contains("Create the administrator account");
    type_into(&mut h, "Login", "admin");
    type_into(&mut h, "Password", "password-1");
    type_into(&mut h, "Repeat the password", "password-2");
    h.get_by_label("Create account").click();
    h.run();
    h.get_by_label_contains("do not match");
    assert!(fake.sent().is_empty());
}

#[test]
fn first_admin_sends_init() {
    let mut s = ConsoleSettings::default();
    s.mode = Some(Mode::Embedded { port: 8443 });
    let (mut h, fake) = harness(s);
    fake.push(Event::Connection(ConnState::NeedsLogin { can_init: true }));
    h.run();
    type_into(&mut h, "Login", "admin");
    type_into(&mut h, "Password", "password-1");
    type_into(&mut h, "Repeat the password", "password-1");
    h.get_by_label("Create account").click();
    h.run();
    assert!(fake.sent().iter().any(|a| matches!(
        a,
        Action::Init { bootstrap_code, login, password }
            if bootstrap_code.is_empty() && login == "admin" && password.expose() == "password-1"
    )));
}

#[test]
fn sign_in_sends_login() {
    let (mut h, fake) = harness(remote());
    fake.push(Event::Connection(ConnState::NeedsLogin { can_init: false }));
    h.run();
    type_into(&mut h, "Login", "prof");
    type_into(&mut h, "Password", "secret-pass");
    h.run();
    h.get_by_label("Sign in").click();
    h.run();
    assert!(fake.sent().iter().any(|a| matches!(
        a,
        Action::Login { login, password } if login == "prof" && password.expose() == "secret-pass"
    )));
}

#[test]
fn rooms_show_counts_and_computers() {
    let (h, _fake) = signed_in(Role::Admin);
    h.get_by_label("Sala 1");
    h.get_by_label("1/2 online");
    h.get_by_label("pc-01");
    h.get_by_label("Online");
    h.get_by_label_contains("Offline ·");
}

#[test]
fn switching_profile_sends_action() {
    let (mut h, fake) = signed_in(Role::Teacher);
    pick_profile(&mut h, "Examen");
    assert!(fake.sent().iter().any(|a| matches!(
        a,
        Action::SetRoomProfile {
            room_id: 10,
            profile_id: 3
        }
    )));
}

#[test]
fn free_profile_needs_confirmation() {
    let (mut h, fake) = signed_in(Role::Teacher);
    pick_profile(&mut h, "Liber");
    h.get_by_label_contains("will run anything");
    assert!(fake.sent().is_empty());
    h.get_by_label("No").click();
    h.run();
    assert!(fake.sent().is_empty());
    assert!(h.query_by_label_contains("will run anything").is_none());
    pick_profile(&mut h, "Liber");
    h.get_by_label("Yes").click();
    h.run();
    assert!(fake.sent().iter().any(|a| matches!(
        a,
        Action::SetRoomProfile {
            room_id: 10,
            profile_id: 1
        }
    )));
}

#[test]
fn failed_embedded_is_red_with_reason() {
    let mut s = ConsoleSettings::default();
    s.mode = Some(Mode::Embedded { port: 8443 });
    let (mut h, fake) = harness(s);
    fake.push(Event::Embedded(EmbeddedState::Failed {
        reason: "Port 8443 is already in use — another Guardian server".into(),
    }));
    h.run();
    h.get_by_label_contains("Server is not running");
    h.get_by_label_contains("Port 8443 is already in use");
    assert!(h.query_by_label_contains("Server running").is_none());
}

#[test]
fn error_event_shows_banner_and_ui_survives() {
    let (mut h, fake) = signed_in(Role::Admin);
    fake.push(Event::Error("boom".into()));
    h.run();
    h.get_by_label("boom");
    h.run();
    h.get_by_label("Dismiss").click();
    h.run();
    assert!(h.query_by_label("boom").is_none());
}

#[test]
fn teacher_sees_only_room_controls() {
    let (h, _fake) = signed_in(Role::Teacher);
    for tab in ["Rooms", "Profiles", "Computers", "Users", "Settings"] {
        assert!(
            h.query_by_label(tab).is_none(),
            "{tab} visible to a teacher"
        );
    }
    h.get_by_label("Sala 1");
}

#[test]
fn admin_sees_the_admin_tabs() {
    let (h, _fake) = signed_in(Role::Admin);
    for tab in ["Rooms", "Profiles", "Computers", "Users", "Settings"] {
        h.get_by_label(tab);
    }
}

#[test]
fn logged_out_shows_reason() {
    let (mut h, fake) = signed_in(Role::Admin);
    fake.push(Event::LoggedOut {
        reason: Some("Your session has ended. Sign in again.".into()),
    });
    fake.push(Event::Connection(ConnState::NeedsLogin { can_init: false }));
    h.run();
    h.get_by_label("Your session has ended. Sign in again.");
    h.get_by_label("Sign in");
}

#[test]
fn reconnecting_is_shown_with_the_last_state() {
    let (mut h, fake) = signed_in(Role::Admin);
    fake.push(Event::Connection(ConnState::Reconnecting {
        attempt: 2,
        error: "Cannot reach the server".into(),
    }));
    h.run();
    h.get_by_label_contains("Reconnecting… (attempt 2)");
    h.get_by_label("Sala 1");
}
