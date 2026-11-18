//! The live picture the console shows, built from `Watch` events: rooms, their
//! computers, profiles, and the texts derived from them. Pure logic.

use std::collections::BTreeMap;

use guardian_proto::v1::{Computer, Profile, Room, WatchEvent, watch_event::Event};

/// Everything the user may see right now.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Fleet {
    rooms: BTreeMap<i64, Room>,
    computers: BTreeMap<u64, Computer>,
    profiles: BTreeMap<i64, Profile>,
}

/// A room as shown.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomView {
    /// Room id.
    pub id: i64,
    /// Room name.
    pub name: String,
    /// The active profile, if known.
    pub profile: Option<Profile>,
    /// Its computers: online first, then by name.
    pub computers: Vec<ComputerView>,
    /// How many are online.
    pub online: usize,
}

/// A computer as shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerView {
    /// Computer id.
    pub id: u64,
    /// Name (hostname when unnamed).
    pub name: String,
    /// Connected now.
    pub online: bool,
    /// "● Online", "Offline · 5 min ago", …
    pub connection: String,
    /// Name of the effective profile.
    pub profile: Option<String>,
    /// Has its own exception profile.
    pub exception: bool,
    /// OS reported by the agent.
    pub os: String,
    /// Agent version.
    pub agent_version: String,
    /// Room id (0 = none).
    pub room_id: i64,
    /// Exception profile id (0 = none).
    pub exception_profile_id: i64,
}

fn key(s: &str) -> String {
    s.to_lowercase()
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// "● Online", "Never connected", "Offline · just now / N min / N h / N days ago".
pub fn connection_text(online: bool, last_seen_unix: i64, now_unix: i64) -> String {
    if online {
        return "● Online".into();
    }
    if last_seen_unix == 0 {
        return "Never connected".into();
    }
    let ago = now_unix.saturating_sub(last_seen_unix).max(0);
    if ago < 60 {
        "Offline · just now".into()
    } else if ago < 3600 {
        format!("Offline · {} min ago", ago / 60)
    } else if ago < 86400 {
        format!("Offline · {} h ago", ago / 3600)
    } else {
        let d = ago / 86400;
        format!("Offline · {d} day{} ago", if d == 1 { "" } else { "s" })
    }
}

impl Fleet {
    /// Applies one `Watch` event. A snapshot replaces everything.
    pub fn apply(&mut self, ev: WatchEvent) {
        match ev.event {
            Some(Event::Snapshot(s)) => {
                self.rooms = s.rooms.into_iter().map(|r| (r.id, r)).collect();
                self.computers = s.computers.into_iter().map(|c| (c.id, c)).collect();
                self.profiles = s.profiles.into_iter().map(|p| (p.id, p)).collect();
            }
            Some(Event::Computer(c)) => {
                self.computers.insert(c.id, c);
            }
            Some(Event::Room(r)) => {
                self.rooms.insert(r.id, r);
            }
            Some(Event::ComputerRemoved(id)) => {
                self.computers.remove(&id);
            }
            Some(Event::RoomRemoved(id)) => {
                self.rooms.remove(&id);
            }
            Some(Event::CommandResult(_)) | None => {}
        }
    }

    /// Replaces the profiles (after an admin edits them; `Watch` does not send
    /// profile changes).
    pub fn set_profiles(&mut self, ps: Vec<Profile>) {
        self.profiles = ps.into_iter().map(|p| (p.id, p)).collect();
    }

    /// One profile.
    pub fn profile(&self, id: i64) -> Option<&Profile> {
        self.profiles.get(&id)
    }

    /// All profiles, by name.
    pub fn profiles(&self) -> Vec<&Profile> {
        let mut v: Vec<&Profile> = self.profiles.values().collect();
        v.sort_by_key(|p| key(&p.name));
        v
    }

    /// (online, all) computers.
    pub fn totals(&self) -> (usize, usize) {
        (
            self.computers.values().filter(|c| c.online).count(),
            self.computers.len(),
        )
    }

    /// Whether any computer is online.
    pub fn any_online(&self) -> bool {
        self.computers.values().any(|c| c.online)
    }

    fn view(&self, c: &Computer, now: i64) -> ComputerView {
        ComputerView {
            id: c.id,
            name: if c.name.is_empty() {
                c.hostname.clone()
            } else {
                c.name.clone()
            },
            online: c.online,
            connection: connection_text(c.online, c.last_seen_unix, now),
            profile: self.profile(c.effective_profile_id).map(|p| p.name.clone()),
            exception: c.exception_profile_id != 0,
            os: c.os.clone(),
            agent_version: c.agent_version.clone(),
            room_id: c.room_id,
            exception_profile_id: c.exception_profile_id,
        }
    }

    fn views(&self, room_id: i64) -> Vec<ComputerView> {
        let now = now_unix();
        let mut v: Vec<ComputerView> = self
            .computers
            .values()
            .filter(|c| c.room_id == room_id)
            .map(|c| self.view(c, now))
            .collect();
        v.sort_by(|a, b| {
            b.online
                .cmp(&a.online)
                .then(key(&a.name).cmp(&key(&b.name)))
        });
        v
    }

    /// Rooms by name, each with its computers.
    pub fn rooms(&self) -> Vec<RoomView> {
        let mut v: Vec<RoomView> = self
            .rooms
            .values()
            .map(|r| {
                let computers = self.views(r.id);
                RoomView {
                    id: r.id,
                    name: r.name.clone(),
                    profile: self.profile(r.active_profile_id).cloned(),
                    online: computers.iter().filter(|c| c.online).count(),
                    computers,
                }
            })
            .collect();
        v.sort_by_key(|r| key(&r.name));
        v
    }

    /// Computers without a room (only admins see them).
    pub fn unassigned(&self) -> Vec<ComputerView> {
        self.views(0)
    }

    /// Every computer, by name (for the admin Computers tab).
    pub fn computers(&self) -> Vec<ComputerView> {
        let now = now_unix();
        let mut v: Vec<ComputerView> = self.computers.values().map(|c| self.view(c, now)).collect();
        v.sort_by_key(|c| key(&c.name));
        v
    }

    /// Room name by id.
    pub fn room_name(&self, id: i64) -> Option<&str> {
        self.rooms.get(&id).map(|r| r.name.as_str())
    }
}
