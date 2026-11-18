#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use guardian_console::fallback::cache::{FallbackCache, Reach, note};
use guardian_console::fallback::key;
use guardian_console::fleet::Fleet;
use guardian_console::settings::{ConsoleSettings, Secret, fallback_paths};
use guardian_core::keys::KeyPair;
use guardian_proto::v1::{
    Computer, LocalStatus, Mode, Profile, Role, Room, Snapshot, User, WatchEvent, watch_event,
};

const SERVER: &str = "https://srv.school:8443";

fn user(id: i64, login: &str, role: Role, rooms: &[i64]) -> User {
    User {
        id,
        login: login.into(),
        role: role as i32,
        active: true,
        room_ids: rooms.to_vec(),
    }
}

fn pw(s: &str) -> Secret {
    Secret::new(s.to_string())
}

fn room(id: i64, name: &str) -> Room {
    Room {
        id,
        name: name.into(),
        active_profile_id: 2,
    }
}

fn computer(id: u64, room_id: i64, key: bool) -> Computer {
    Computer {
        id,
        name: format!("pc-{id}"),
        room_id,
        lan_addresses: vec![format!("10.0.0.{id}")],
        public_key: if key {
            KeyPair::generate().public().as_bytes().to_vec()
        } else {
            vec![]
        },
        ..Default::default()
    }
}

fn profiles() -> Vec<Profile> {
    vec![
        Profile {
            id: 1,
            name: "Liber".into(),
            mode: Mode::Free as i32,
            apps: vec![],
        },
        Profile {
            id: 2,
            name: "Lecție".into(),
            mode: Mode::Blacklist as i32,
            apps: vec![],
        },
        Profile {
            id: 3,
            name: "Examen".into(),
            mode: Mode::Whitelist as i32,
            apps: vec![],
        },
    ]
}

fn snapshot(rooms: Vec<Room>, computers: Vec<Computer>) -> WatchEvent {
    WatchEvent {
        event: Some(watch_event::Event::Snapshot(Snapshot {
            rooms,
            computers,
            profiles: profiles(),
        })),
    }
}

#[test]
fn verifier_accepts_right_password_only() {
    let mut c = FallbackCache::new(SERVER, 4);
    c.remember_login(&user(5, "prof", Role::Teacher, &[1]), &pw("secret-123"))
        .unwrap();
    let u = c.verify("prof", &pw("secret-123")).unwrap();
    assert_eq!(u.user_id, 5);
    assert_eq!(u.room_ids, vec![1]);
    assert!(u.verifier.starts_with("$argon2id$"));
    assert!(c.verify("PROF", &pw("secret-123")).is_some());
    assert!(c.verify("prof", &pw("secret-124")).is_none());
}

#[test]
fn unknown_login_is_none() {
    let mut c = FallbackCache::new(SERVER, 4);
    c.remember_login(&user(5, "prof", Role::Teacher, &[1]), &pw("secret-123"))
        .unwrap();
    assert!(c.verify("nobody", &pw("secret-123")).is_none());
}

#[test]
fn update_user_keeps_the_verifier() {
    let mut c = FallbackCache::new(SERVER, 4);
    c.remember_login(&user(5, "prof", Role::Teacher, &[1]), &pw("secret-123"))
        .unwrap();
    c.update_user(&user(5, "prof", Role::Teacher, &[1, 2]));
    let u = c.verify("prof", &pw("secret-123")).unwrap();
    assert_eq!(u.room_ids, vec![1, 2]);
    // A user who never typed a password here gets no verifier.
    c.update_user(&user(6, "other", Role::Teacher, &[1]));
    assert!(c.verify("other", &pw("")).is_none());
}

#[test]
fn cache_of_other_server_is_ignored() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("fallback.pb");
    FallbackCache::new(SERVER, 4).save(&p).unwrap();
    assert!(FallbackCache::load(&p, SERVER).is_some());
    assert!(FallbackCache::load(&p, "https://other:8443").is_none());
    assert!(FallbackCache::load(&d.path().join("missing.pb"), SERVER).is_none());
    std::fs::write(&p, b"\xff\xff\xff").unwrap();
    assert!(FallbackCache::load(&p, SERVER).is_none());
}

#[test]
fn merge_keeps_rooms_of_other_teachers() {
    let mut c = FallbackCache::new(SERVER, 4);
    c.merge_event(&snapshot(
        vec![room(1, "Sala 1")],
        vec![computer(10, 1, true)],
    ));
    c.merge_event(&snapshot(
        vec![room(2, "Sala 2")],
        vec![computer(20, 2, true)],
    ));
    assert_eq!(c.computers_in(1).len(), 1);
    assert_eq!(c.computers_in(2).len(), 1);
    let admin = cached(&mut c, user(1, "admin", Role::Admin, &[]));
    assert_eq!(c.rooms_for(&admin).len(), 2);
}

#[test]
fn removed_computer_is_forgotten() {
    let mut c = FallbackCache::new(SERVER, 4);
    c.merge_event(&snapshot(
        vec![room(1, "Sala 1"), room(2, "Sala 2")],
        vec![computer(10, 1, true), computer(11, 1, true)],
    ));
    c.merge_event(&WatchEvent {
        event: Some(watch_event::Event::ComputerRemoved(10)),
    });
    c.merge_event(&WatchEvent {
        event: Some(watch_event::Event::RoomRemoved(2)),
    });
    let ids: Vec<u64> = c.computers_in(1).iter().map(|x| x.id).collect();
    assert_eq!(ids, vec![11]);
    let admin = cached(&mut c, user(1, "admin", Role::Admin, &[]));
    assert_eq!(c.rooms_for(&admin).len(), 1);
}

#[test]
fn computer_update_moves_it() {
    let mut c = FallbackCache::new(SERVER, 4);
    c.merge_event(&snapshot(
        vec![room(1, "A"), room(2, "B")],
        vec![computer(10, 1, true)],
    ));
    let mut moved = computer(10, 2, true);
    moved.lan_addresses = vec!["10.0.0.99".into()];
    c.merge_event(&WatchEvent {
        event: Some(watch_event::Event::Computer(moved)),
    });
    assert!(c.computers_in(1).is_empty());
    assert_eq!(c.computers_in(2)[0].lan_addresses, vec!["10.0.0.99"]);
}

#[test]
fn computers_without_key_are_skipped() {
    let mut c = FallbackCache::new(SERVER, 4);
    c.merge_event(&snapshot(
        vec![room(1, "Sala 1")],
        vec![computer(10, 1, false), computer(11, 1, true)],
    ));
    let ids: Vec<u64> = c.computers_in(1).iter().map(|x| x.id).collect();
    assert_eq!(ids, vec![11]);
}

fn cached(c: &mut FallbackCache, u: User) -> guardian_proto::v1::CachedUser {
    c.remember_login(&u, &pw("password-1")).unwrap();
    c.verify(&u.login, &pw("password-1")).unwrap()
}

#[test]
fn teacher_sees_only_their_rooms_admin_sees_all() {
    let mut c = FallbackCache::new(SERVER, 4);
    c.merge_event(&snapshot(
        vec![room(1, "Sala 1"), room(2, "Sala 2"), room(3, "Sala 3")],
        vec![],
    ));
    let t = cached(&mut c, user(5, "prof", Role::Teacher, &[2]));
    let names: Vec<String> = c.rooms_for(&t).into_iter().map(|r| r.name).collect();
    assert_eq!(names, vec!["Sala 2"]);
    let a = cached(&mut c, user(1, "admin", Role::Admin, &[]));
    assert_eq!(c.rooms_for(&a).len(), 3);
}

#[test]
fn fleet_for_marks_reachable_online_with_local_profile() {
    let mut c = FallbackCache::new(SERVER, 4);
    c.merge_event(&snapshot(
        vec![room(1, "Sala 1")],
        vec![computer(10, 1, true), computer(11, 1, true)],
    ));
    let t = cached(&mut c, user(5, "prof", Role::Teacher, &[1]));
    let mut reach = BTreeMap::new();
    reach.insert(
        10,
        Reach::Reachable(LocalStatus {
            computer_id: 10,
            room_id: 1,
            effective_profile_id: 3,
            local_override: true,
            ..Default::default()
        }),
    );
    reach.insert(11, Reach::Unreachable);
    let fleet: Fleet = c.fleet_for(&t, &reach);
    let rooms = fleet.rooms();
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].online, 1);
    let pc10 = rooms[0].computers.iter().find(|x| x.id == 10).unwrap();
    assert!(pc10.online);
    assert_eq!(pc10.profile.as_deref(), Some("Examen"));
    assert_eq!(pc10.connection, "Reachable locally");
    let pc11 = rooms[0].computers.iter().find(|x| x.id == 11).unwrap();
    assert_eq!(pc11.connection, "Not reachable");
}

#[test]
fn notes_replace_connection_text() {
    assert_eq!(note(&Reach::Unknown), "Checking…");
    assert_eq!(
        note(&Reach::Reachable(LocalStatus::default())),
        "Reachable locally"
    );
    assert_eq!(note(&Reach::Unreachable), "Not reachable");
    assert_eq!(note(&Reach::StillOnline), "Still connected to the server");
    assert_eq!(note(&Reach::Refused("No.".into())), "No.");
    let mut f = Fleet::from_parts(vec![room(1, "R")], vec![computer(10, 1, true)], profiles());
    let mut notes = BTreeMap::new();
    notes.insert(10, "Hello".to_string());
    f.set_notes(notes);
    assert_eq!(f.rooms()[0].computers[0].connection, "Hello");
}

#[test]
fn cache_round_trips_and_is_private() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("fallback.pb");
    let mut c = FallbackCache::new(SERVER, 4);
    c.merge_event(&snapshot(
        vec![room(1, "Sala 1")],
        vec![computer(10, 1, true)],
    ));
    c.remember_login(&user(5, "prof", Role::Teacher, &[1]), &pw("secret-123"))
        .unwrap();
    c.save(&p).unwrap();
    let back = FallbackCache::load(&p, SERVER).unwrap();
    assert_eq!(back, c);
    assert_eq!(back.console_id(), 4);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn key_is_created_once() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("console.key");
    let a = key::load_or_create(&p).unwrap();
    let b = key::load_or_create(&p).unwrap();
    assert_eq!(a.public(), b.public());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn fallback_files_sit_next_to_the_settings() {
    let (k, c) = fallback_paths(std::path::Path::new("/x/guardian/console.toml"));
    assert_eq!(k, std::path::Path::new("/x/guardian/console.key"));
    assert_eq!(c, std::path::Path::new("/x/guardian/fallback.pb"));
    let mut s = ConsoleSettings::default();
    assert_eq!(s.fallback_port(), 11500);
    s.fallback_port = Some(11600);
    assert_eq!(s.fallback_port(), 11600);
}
