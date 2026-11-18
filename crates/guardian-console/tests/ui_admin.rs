#![cfg(feature = "testing")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod uikit;

use egui_kittest::kittest::Queryable;
use guardian_console::backend::{Action, Event};
use guardian_proto::v1::{EnrollCode, EnrollCodeKind, Mode as PMode, Role, Settings};
use uikit::*;

fn sent(fake: &guardian_console::testing::FakeBackend) -> Vec<Action> {
    fake.sent()
}

#[test]
fn rooms_add_sends_create() {
    let (mut h, fake) = signed_in(Role::Admin);
    open_tab(&mut h, "Rooms");
    type_into(&mut h, "New room", "Sala 2");
    h.get_by_label("Add room").click();
    h.run();
    assert!(
        sent(&fake)
            .iter()
            .any(|a| matches!(a, Action::CreateRoom { name } if name == "Sala 2")),
        "{:?}",
        sent(&fake)
    );
}

#[test]
fn delete_room_needs_confirmation() {
    let (mut h, fake) = signed_in(Role::Admin);
    open_tab(&mut h, "Rooms");
    h.get_by_label("Delete Sala 1").click();
    h.run();
    h.get_by_label_contains("move to no room");
    h.get_by_label("No").click();
    h.run();
    assert!(
        !sent(&fake)
            .iter()
            .any(|a| matches!(a, Action::DeleteRoom { .. }))
    );
    h.get_by_label("Delete Sala 1").click();
    h.run();
    h.get_by_label("Yes").click();
    h.run();
    assert!(
        sent(&fake)
            .iter()
            .any(|a| matches!(a, Action::DeleteRoom { id: 10 }))
    );
}

#[test]
fn profile_mode_and_apps_are_saved() {
    let (mut h, fake) = signed_in(Role::Admin);
    open_tab(&mut h, "Profiles");
    assert!(
        sent(&fake)
            .iter()
            .any(|a| matches!(a, Action::LoadProfiles))
    );
    fake.push(Event::Profiles(profiles()));
    h.run();
    pick(&mut h, "Profile to edit", "Examen");
    h.get_by_label_contains("Allow only the listed apps");
    type_into(&mut h, "Application name", "notepad.exe");
    h.get_by_label("Add application").click();
    h.run();
    h.get_by_label("notepad.exe");
    h.get_by_label("Save profile").click();
    h.run();
    assert!(
        sent(&fake).iter().any(|a| matches!(
            a,
            Action::SaveProfile(p) if p.id == 3
                && p.mode == PMode::Whitelist as i32
                && p.apps == ["notepad.exe"]
        )),
        "{:?}",
        sent(&fake)
    );
}

#[test]
fn computer_move_sends_update() {
    let (mut h, fake) = signed_in(Role::Admin);
    open_tab(&mut h, "Computers");
    pick(&mut h, "Room of pc-01", "— no room —");
    assert!(
        sent(&fake).iter().any(|a| matches!(
            a,
            Action::UpdateComputer {
                id: 1,
                name: None,
                room_id: Some(0),
                exception_profile_id: None
            }
        )),
        "{:?}",
        sent(&fake)
    );
    pick(&mut h, "Exception of pc-02", "Liber");
    assert!(sent(&fake).iter().any(|a| matches!(
        a,
        Action::UpdateComputer {
            id: 2,
            exception_profile_id: Some(1),
            ..
        }
    )));
}

#[test]
fn revoke_needs_confirmation() {
    let (mut h, fake) = signed_in(Role::Admin);
    open_tab(&mut h, "Computers");
    h.get_by_label("Revoke pc-01").click();
    h.run();
    h.get_by_label_contains("enrolled again with a new code");
    h.get_by_label("No").click();
    h.run();
    assert!(
        !sent(&fake)
            .iter()
            .any(|a| matches!(a, Action::RevokeComputer { .. }))
    );
    h.get_by_label("Revoke pc-01").click();
    h.run();
    h.get_by_label("Yes").click();
    h.run();
    assert!(
        sent(&fake)
            .iter()
            .any(|a| matches!(a, Action::RevokeComputer { id: 1 }))
    );
}

#[test]
fn add_user_sends_create_without_echoing_password() {
    let (mut h, fake) = signed_in(Role::Admin);
    open_tab(&mut h, "Users");
    assert!(sent(&fake).iter().any(|a| matches!(a, Action::LoadUsers)));
    type_into(&mut h, "New login", "ion");
    type_into(&mut h, "New password", "teacher-pass-1");
    h.get_by_label("Add user").click();
    h.run();
    assert!(sent(&fake).iter().any(|a| matches!(
        a,
        Action::CreateUser { login, password, role: Role::Teacher }
            if login == "ion" && password.expose() == "teacher-pass-1"
    )));
    assert!(h.query_by_value("teacher-pass-1").is_none());
    assert!(h.query_by_label_contains("teacher-pass-1").is_none());
}

#[test]
fn settings_save() {
    let (mut h, fake) = signed_in(Role::Admin);
    open_tab(&mut h, "Settings");
    assert!(
        sent(&fake)
            .iter()
            .any(|a| matches!(a, Action::LoadSettings))
    );
    fake.push(Event::Settings(Settings {
        default_profile_id: 2,
        retention_days: 90,
    }));
    h.run();
    pick(&mut h, "Default profile", "Examen");
    h.get_by_label("Save settings").click();
    h.run();
    assert!(sent(&fake).iter().any(|a| matches!(
        a,
        Action::SaveSettings(Settings {
            default_profile_id: 3,
            retention_days: 90
        })
    )));
}

#[test]
fn connection_info_shows_commands_after_new_code() {
    let (mut h, fake) = signed_in(Role::Admin);
    h.get_by_label("Connection info…").click();
    h.run();
    h.get_by_label("New code").click();
    h.run();
    assert!(sent(&fake).iter().any(|a| matches!(
        a,
        Action::CreateAgentCode {
            room_id: 10,
            max_uses: 50,
            ttl_seconds: 86400
        }
    )));
    fake.push(Event::Code(EnrollCode {
        code: "ABCD-1234".into(),
        expires_at_unix: 0,
        max_uses: 50,
        kind: EnrollCodeKind::EnrollAgent as i32,
        room_id: 10,
    }));
    h.run();
    // Windows: enroll … --code; Linux: install.sh <server> <code>.
    h.get_by_label_contains("enroll --server wss://srv.school:8443 --code ABCD-1234");
    h.get_by_label_contains("sudo ./install.sh wss://srv.school:8443 ABCD-1234");
}

#[test]
fn change_mode_returns_to_setup() {
    let (mut h, fake) = signed_in(Role::Admin);
    open_tab(&mut h, "Settings");
    h.get_by_label("Change mode…").click();
    h.run();
    h.get_by_label("Yes").click();
    h.run();
    assert!(sent(&fake).iter().any(|a| matches!(a, Action::ForgetMode)));
    h.get_by_label("Welcome to Guardian Console");
}
