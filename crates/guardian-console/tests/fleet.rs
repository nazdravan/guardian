#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_console::fleet::{Fleet, connection_text};
use guardian_proto::v1::{Computer, Mode, Profile, Room, Snapshot, WatchEvent, watch_event::Event};

fn profile(id: i64, name: &str, mode: Mode) -> Profile {
    Profile {
        id,
        name: name.into(),
        mode: mode as i32,
        apps: vec![],
    }
}

fn room(id: i64, name: &str, profile: i64) -> Room {
    Room {
        id,
        name: name.into(),
        active_profile_id: profile,
    }
}

fn pc(id: u64, name: &str, room: i64, online: bool) -> Computer {
    Computer {
        id,
        name: name.into(),
        room_id: room,
        online,
        effective_profile_id: 2,
        ..Default::default()
    }
}

fn ev(e: Event) -> WatchEvent {
    WatchEvent { event: Some(e) }
}

fn snapshot(rooms: Vec<Room>, computers: Vec<Computer>) -> WatchEvent {
    ev(Event::Snapshot(Snapshot {
        rooms,
        computers,
        profiles: vec![
            profile(1, "Liber", Mode::Free),
            profile(2, "Lecție", Mode::Blacklist),
            profile(3, "Examen", Mode::Whitelist),
        ],
    }))
}

#[test]
fn snapshot_replaces_everything() {
    let mut f = Fleet::default();
    f.apply(snapshot(
        vec![room(10, "A", 2)],
        vec![pc(1, "pc1", 10, true), pc(2, "pc2", 10, false)],
    ));
    assert_eq!(f.totals(), (1, 2));
    f.apply(snapshot(
        vec![room(10, "A", 2)],
        vec![pc(1, "pc1", 10, true)],
    ));
    assert_eq!(f.totals(), (1, 1));
    assert_eq!(f.rooms()[0].computers.len(), 1);
}

#[test]
fn computer_update_upserts() {
    let mut f = Fleet::default();
    f.apply(snapshot(
        vec![room(10, "A", 2)],
        vec![pc(1, "pc1", 10, false)],
    ));
    f.apply(ev(Event::Computer(pc(1, "pc1", 10, true))));
    f.apply(ev(Event::Computer(pc(2, "pc2", 10, true))));
    assert_eq!(f.totals(), (2, 2));
}

#[test]
fn removals_remove() {
    let mut f = Fleet::default();
    f.apply(snapshot(
        vec![room(10, "A", 2), room(11, "B", 2)],
        vec![pc(1, "pc1", 10, true)],
    ));
    f.apply(ev(Event::ComputerRemoved(1)));
    f.apply(ev(Event::RoomRemoved(11)));
    assert_eq!(f.totals(), (0, 0));
    assert_eq!(f.rooms().len(), 1);
}

#[test]
fn rooms_sorted_with_online_count() {
    let mut f = Fleet::default();
    f.apply(snapshot(
        vec![room(10, "b", 2), room(11, "A", 2)],
        vec![
            pc(1, "z", 10, false),
            pc(2, "y", 10, true),
            pc(3, "x", 10, true),
        ],
    ));
    let rooms = f.rooms();
    assert_eq!(
        rooms.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        ["A", "b"]
    );
    assert_eq!(rooms[1].online, 2);
    // Online first, then by name.
    assert_eq!(
        rooms[1]
            .computers
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["x", "y", "z"]
    );
}

#[test]
fn room_profile_is_resolved() {
    let mut f = Fleet::default();
    f.apply(snapshot(vec![room(10, "A", 3)], vec![]));
    assert_eq!(f.rooms()[0].profile.as_ref().unwrap().name, "Examen");
    f.apply(ev(Event::Room(room(10, "A", 1))));
    assert_eq!(f.rooms()[0].profile.as_ref().unwrap().name, "Liber");
}

#[test]
fn exception_is_flagged() {
    let mut f = Fleet::default();
    let mut c = pc(1, "teacher", 10, true);
    c.exception_profile_id = 1;
    c.effective_profile_id = 1;
    f.apply(snapshot(
        vec![room(10, "A", 2)],
        vec![c, pc(2, "s", 10, true)],
    ));
    let r = &f.rooms()[0];
    let t = r.computers.iter().find(|c| c.name == "teacher").unwrap();
    assert!(t.exception);
    assert_eq!(t.profile.as_deref(), Some("Liber"));
    let s = r.computers.iter().find(|c| c.name == "s").unwrap();
    assert!(!s.exception);
    assert_eq!(s.profile.as_deref(), Some("Lecție"));
}

#[test]
fn unassigned_lists_room_zero() {
    let mut f = Fleet::default();
    f.apply(snapshot(
        vec![room(10, "A", 2)],
        vec![pc(1, "new", 0, true), pc(2, "in", 10, true)],
    ));
    let u = f.unassigned();
    assert_eq!(u.len(), 1);
    assert_eq!(u[0].name, "new");
}

#[test]
fn connection_texts() {
    let now = 1_000_000;
    assert_eq!(connection_text(true, now - 5000, now), "● Online");
    assert_eq!(connection_text(false, 0, now), "Never connected");
    assert_eq!(connection_text(false, now - 30, now), "Offline · just now");
    assert_eq!(
        connection_text(false, now - 5 * 60, now),
        "Offline · 5 min ago"
    );
    assert_eq!(
        connection_text(false, now - 3 * 3600, now),
        "Offline · 3 h ago"
    );
    assert_eq!(
        connection_text(false, now - 2 * 86400, now),
        "Offline · 2 days ago"
    );
}

#[test]
fn diacritics_survive() {
    let mut f = Fleet::default();
    f.apply(snapshot(vec![room(10, "Sala 1", 2)], vec![]));
    assert_eq!(f.rooms()[0].profile.as_ref().unwrap().name, "Lecție");
    assert_eq!(f.profile(2).unwrap().name, "Lecție");
    assert_eq!(
        f.profiles()
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["Examen", "Lecție", "Liber"]
    );
}

#[test]
fn set_profiles_replaces_the_list() {
    let mut f = Fleet::default();
    f.apply(snapshot(vec![], vec![]));
    f.set_profiles(vec![profile(7, "Test", Mode::Blacklist)]);
    assert_eq!(f.profiles().len(), 1);
}
