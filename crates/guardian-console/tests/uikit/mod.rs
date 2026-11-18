//! Shared helpers for the UI tests.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui::accesskit::Role as A11y;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use guardian_console::backend::{ConnState, Event};
use guardian_console::fleet::Fleet;
use guardian_console::settings::{ConsoleSettings, Mode};
use guardian_console::testing::FakeBackend;
use guardian_console::ui::ConsoleApp;
use guardian_proto::v1::{
    Computer, Mode as PMode, Profile, Role, Room, Snapshot, User, WatchEvent, watch_event,
};

pub type App = ConsoleApp<FakeBackend>;

pub fn harness(settings: ConsoleSettings) -> (Harness<'static, App>, FakeBackend) {
    let fake = FakeBackend::default();
    let app = ConsoleApp::new(fake.clone(), settings);
    let h = Harness::builder()
        .with_size((1100.0, 720.0))
        .build_ui_state(|ui, app: &mut App| app.show(ui), app);
    (h, fake)
}

pub fn remote() -> ConsoleSettings {
    let mut s = ConsoleSettings::default();
    s.mode = Some(Mode::Remote {
        server: "https://srv.school:8443".into(),
        cert_sha256: None,
    });
    s
}

pub fn user(role: Role) -> User {
    User {
        id: 1,
        login: "prof".into(),
        role: role as i32,
        active: true,
        room_ids: vec![10],
    }
}

pub fn fleet() -> Fleet {
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

pub fn signed_in(role: Role) -> (Harness<'static, App>, FakeBackend) {
    let (mut h, fake) = harness(remote());
    fake.push(Event::Connection(ConnState::Online));
    fake.push(Event::LoggedIn(user(role)));
    fake.push(Event::Fleet(fleet()));
    h.run();
    (h, fake)
}

pub fn type_into(h: &mut Harness<'static, App>, label: &str, text: &str) {
    h.get_by_label(label).focus();
    h.run();
    h.get_by_label(label).type_text(text);
    h.run();
}

pub fn pick_profile(h: &mut Harness<'static, App>, name: &str) {
    h.get_by_role_and_label(A11y::ComboBox, "Profile").click();
    h.run();
    h.get_by_role(A11y::ListBox).get_by_label(name).click();
    h.run();
}

/// Opens `tab` (admin).
pub fn open_tab(h: &mut Harness<'static, App>, tab: &str) {
    h.get_by_label(tab).click();
    h.run();
}

/// Picks `item` in the combo box labelled `label`.
pub fn pick(h: &mut Harness<'static, App>, label: &str, item: &str) {
    h.get_by_role_and_label(A11y::ComboBox, label).click();
    h.run();
    h.get_by_role(A11y::ListBox).get_by_label(item).click();
    h.run();
}

/// The three default profiles.
pub fn profiles() -> Vec<Profile> {
    fleet().profiles().into_iter().cloned().collect()
}
