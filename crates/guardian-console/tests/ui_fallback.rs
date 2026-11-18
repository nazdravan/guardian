#![cfg(feature = "testing")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod uikit;

use std::collections::BTreeMap;

use egui::accesskit::Role as A11y;
use egui_kittest::kittest::Queryable;
use guardian_console::backend::{Action, ConnState, Event, FallbackState};
use guardian_console::settings::{ConsoleSettings, Mode};
use guardian_proto::v1::{CachedUser, Role};
use uikit::*;

fn cached(role: Role) -> CachedUser {
    CachedUser {
        user_id: 1,
        login: "prof".into(),
        role: role as i32,
        room_ids: vec![10],
        verifier: String::new(),
    }
}

fn active(role: Role) -> Event {
    let mut f = fleet();
    let mut notes = BTreeMap::new();
    notes.insert(1, "Reachable locally".to_string());
    notes.insert(2, "Not reachable".to_string());
    f.set_notes(notes);
    Event::Fallback(FallbackState::Active {
        user: cached(role),
        fleet: f,
    })
}

fn reconnecting() -> Event {
    Event::Connection(ConnState::Reconnecting {
        attempt: 2,
        error: "Cannot reach the server".into(),
    })
}

#[test]
fn fallback_window_enables_with_code() {
    let (mut h, fake) = signed_in(Role::Teacher);
    h.get_by_label("Local fallback…").click();
    h.run();
    h.get_by_label_contains("Not enabled on this computer.");
    assert!(h.query_by_label("Enable (create a code)").is_none());
    type_into(&mut h, "Console code", "ABCD-EFGH");
    h.get_by_label("Enable").click();
    h.run();
    assert!(
        fake.sent().iter().any(|a| matches!(
            a,
            Action::EnableFallback { code: Some(c) } if c == "ABCD-EFGH"
        )),
        "{:?}",
        fake.sent()
    );
}

#[test]
fn admin_sees_create_a_code() {
    let (mut h, fake) = signed_in(Role::Admin);
    h.get_by_label("Local fallback…").click();
    h.run();
    h.get_by_label("Enable (create a code)").click();
    h.run();
    assert!(
        fake.sent()
            .iter()
            .any(|a| matches!(a, Action::EnableFallback { code: None }))
    );
    fake.push(Event::FallbackEnabled(Some(3)));
    h.run();
    h.get_by_label_contains("Enabled (console #3).");
}

#[test]
fn sign_in_screen_offers_fallback_when_available() {
    let (mut h, fake) = harness(remote());
    fake.push(reconnecting());
    h.run();
    assert!(h.query_by_label("Work locally (fallback mode)…").is_none());
    fake.push(Event::Fallback(FallbackState::Available));
    h.run();
    h.get_by_label("Work locally (fallback mode)…");
}

#[test]
fn local_sign_in_sends_fallback_login() {
    let (mut h, fake) = harness(remote());
    fake.push(reconnecting());
    fake.push(Event::Fallback(FallbackState::Available));
    h.run();
    h.get_by_label("Work locally (fallback mode)…").click();
    h.run();
    h.get_by_label_contains("Sign in to fallback mode");
    type_into(&mut h, "Login", "prof");
    type_into(&mut h, "Password", "secret-123");
    h.get_by_label("Sign in").click();
    h.run();
    assert!(
        fake.sent().iter().any(|a| matches!(
            a,
            Action::FallbackLogin { login, password } if login == "prof" && password.expose() == "secret-123"
        )),
        "{:?}",
        fake.sent()
    );
}

#[test]
fn reconnecting_banner_offers_enter() {
    let (mut h, fake) = signed_in(Role::Teacher);
    fake.push(reconnecting());
    fake.push(Event::Fallback(FallbackState::Available));
    h.run();
    h.get_by_label("Work locally (fallback mode)").click();
    h.run();
    assert!(
        fake.sent()
            .iter()
            .any(|a| matches!(a, Action::EnterFallback))
    );
}

#[test]
fn active_fallback_shows_banner_and_hides_admin_tabs() {
    let (mut h, fake) = signed_in(Role::Admin);
    h.get_by_label("Rooms");
    fake.push(reconnecting());
    fake.push(active(Role::Admin));
    h.run();
    h.get_by_label_contains("Fallback mode: the server cannot be reached.");
    assert!(h.query_by_label("Rooms").is_none());
    assert!(h.query_by_label("Connection info…").is_none());
    assert!(h.query_by_label("Change password…").is_none());
    h.get_by_label("Signed in locally as prof");
}

#[test]
fn active_fallback_works_from_the_sign_in_screen() {
    let (mut h, fake) = harness(remote());
    fake.push(reconnecting());
    fake.push(active(Role::Teacher));
    h.run();
    h.get_by_label_contains("Fallback mode: the server cannot be reached.");
    h.get_by_label("Sala 1");
}

#[test]
fn free_profile_is_not_offered_in_fallback() {
    let (mut h, fake) = harness(remote());
    fake.push(reconnecting());
    fake.push(active(Role::Teacher));
    h.run();
    h.get_by_role_and_label(A11y::ComboBox, "Profile").click();
    h.run();
    let list = h.get_by_role(A11y::ListBox);
    assert!(list.query_by_label("Liber").is_none());
    list.get_by_label("Examen");
}

#[test]
fn switch_in_fallback_sends_set_room_profile() {
    let (mut h, fake) = harness(remote());
    fake.push(reconnecting());
    fake.push(active(Role::Teacher));
    h.run();
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
fn notes_are_shown_per_computer() {
    let (mut h, fake) = harness(remote());
    fake.push(reconnecting());
    fake.push(active(Role::Teacher));
    h.run();
    h.get_by_label("Not reachable");
    h.get_by_label("Reachable locally");
}

#[test]
fn leave_button_sends_leave() {
    let (mut h, fake) = harness(remote());
    fake.push(reconnecting());
    fake.push(active(Role::Teacher));
    h.run();
    h.get_by_label("Leave fallback mode").click();
    h.run();
    assert!(
        fake.sent()
            .iter()
            .any(|a| matches!(a, Action::LeaveFallback))
    );
}

#[test]
fn embedded_mode_has_no_fallback_button() {
    let mut s = ConsoleSettings::default();
    s.mode = Some(Mode::Embedded { port: 8443 });
    let (mut h, fake) = harness(s);
    fake.push(Event::Connection(ConnState::Online));
    fake.push(Event::LoggedIn(user(Role::Admin)));
    fake.push(Event::Fleet(fleet()));
    h.run();
    assert!(h.query_by_label("Local fallback…").is_none());
}
