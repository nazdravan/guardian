//! What the console keeps for fallback mode while the server is reachable
//! (`fallback.pb`): rooms, computers with their keys and LAN addresses,
//! profiles, and an argon2id verifier for every user who signed in here.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use guardian_proto::v1::{
    CachedComputer, CachedUser, Computer, ConsoleCache, LocalStatus, Profile, Role, Room, User,
    WatchEvent, watch_event::Event,
};
use prost::Message as _;

use super::FallbackError;
use crate::fleet::Fleet;
use crate::settings::{Secret, write_private};

/// What the console knows about one computer in fallback mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// Not asked yet.
    Unknown,
    /// Answered on the local network.
    Reachable(LocalStatus),
    /// No answer.
    Unreachable,
    /// The agent still has its server connection.
    StillOnline,
    /// The agent refused (its message).
    Refused(String),
}

/// The text shown for a computer in fallback mode.
pub fn note(r: &Reach) -> &str {
    match r {
        Reach::Unknown => "Checking…",
        Reach::Reachable(_) => "Reachable locally",
        Reach::Unreachable => "Not reachable",
        Reach::StillOnline => "Still connected to the server",
        Reach::Refused(m) => m,
    }
}

/// The offline cache of one console for one server.
#[derive(Clone, PartialEq)]
pub struct FallbackCache(ConsoleCache);

impl std::fmt::Debug for FallbackCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FallbackCache")
            .field("server", &self.0.server)
            .field("console_id", &self.0.console_id)
            .field("users", &self.0.users.len())
            .field("rooms", &self.0.rooms.len())
            .field("computers", &self.0.computers.len())
            .finish()
    }
}

fn hash(p: &str) -> Option<String> {
    Argon2::default()
        .hash_password(p.as_bytes())
        .ok()
        .map(|h| h.to_string())
}

fn check(p: &str, verifier: &str) -> bool {
    PasswordHash::new(verifier)
        .ok()
        .is_some_and(|h| Argon2::default().verify_password(p.as_bytes(), &h).is_ok())
}

fn cached_computer(c: &Computer) -> CachedComputer {
    CachedComputer {
        id: c.id,
        name: if c.name.is_empty() {
            c.hostname.clone()
        } else {
            c.name.clone()
        },
        room_id: c.room_id,
        public_key: c.public_key.clone(),
        lan_addresses: c.lan_addresses.clone(),
        exception_profile_id: c.exception_profile_id,
    }
}

impl FallbackCache {
    /// An empty cache for `server`, enrolled as `console_id`.
    pub fn new(server: &str, console_id: i64) -> FallbackCache {
        FallbackCache(ConsoleCache {
            server: server.to_string(),
            console_id,
            ..Default::default()
        })
    }

    /// The console id on the server.
    pub fn console_id(&self) -> i64 {
        self.0.console_id
    }

    /// Reads `path`; `None` when missing, unreadable or for another server.
    pub fn load(path: &Path, server: &str) -> Option<FallbackCache> {
        let bytes = std::fs::read(path).ok()?;
        let c = ConsoleCache::decode(bytes.as_slice()).ok()?;
        (c.server == server && c.console_id != 0).then_some(FallbackCache(c))
    }

    /// Writes `path` atomically (0600).
    pub fn save(&self, path: &Path) -> Result<(), FallbackError> {
        write_private(path, &self.0.encode_to_vec()).map_err(|source| FallbackError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// After an online sign-in: the user's role, rooms and a new verifier.
    pub fn remember_login(&mut self, user: &User, password: &Secret) -> Result<(), FallbackError> {
        let verifier = hash(password.expose()).ok_or(FallbackError::Hash)?;
        self.update_user(user);
        if let Some(u) = self.0.users.iter_mut().find(|u| u.user_id == user.id) {
            u.verifier = verifier;
        }
        Ok(())
    }

    /// Role and rooms from the server (a session resumed from its token: no
    /// password, so the verifier stays).
    pub fn update_user(&mut self, user: &User) {
        let role = Role::try_from(user.role).unwrap_or(Role::Unspecified);
        match self.0.users.iter_mut().find(|u| u.user_id == user.id) {
            Some(u) => {
                u.login = user.login.clone();
                u.role = role as i32;
                u.room_ids = user.room_ids.clone();
            }
            None => self.0.users.push(CachedUser {
                user_id: user.id,
                login: user.login.clone(),
                role: role as i32,
                room_ids: user.room_ids.clone(),
                verifier: String::new(),
            }),
        }
    }

    /// The user whose password this is; an unknown login costs the same time.
    pub fn verify(&self, login: &str, password: &Secret) -> Option<CachedUser> {
        let found = self
            .0
            .users
            .iter()
            .find(|u| u.login.eq_ignore_ascii_case(login) && !u.verifier.is_empty());
        match found {
            Some(u) => check(password.expose(), &u.verifier).then(|| u.clone()),
            None => {
                static DUMMY: OnceLock<Option<String>> = OnceLock::new();
                if let Some(d) = DUMMY.get_or_init(|| hash("dummy-password")) {
                    let _ = check(password.expose(), d);
                }
                None
            }
        }
    }

    /// Whether any user can sign in locally.
    pub fn has_users(&self) -> bool {
        self.0.users.iter().any(|u| !u.verifier.is_empty())
    }

    /// Takes in one `Watch` event. Rooms and computers are added or updated but
    /// never dropped because one user does not see them (another teacher may);
    /// removals from the server drop them. Computers without a key are skipped.
    pub fn merge_event(&mut self, ev: &WatchEvent) {
        match &ev.event {
            Some(Event::Snapshot(s)) => {
                for r in &s.rooms {
                    self.upsert_room(r);
                }
                for c in &s.computers {
                    self.upsert_computer(c);
                }
                if !s.profiles.is_empty() {
                    self.0.profiles = s.profiles.clone();
                }
            }
            Some(Event::Computer(c)) => self.upsert_computer(c),
            Some(Event::Room(r)) => self.upsert_room(r),
            Some(Event::ComputerRemoved(id)) => self.0.computers.retain(|c| c.id != *id),
            Some(Event::RoomRemoved(id)) => self.0.rooms.retain(|r| r.id != *id),
            Some(Event::CommandResult(_)) | None => {}
        }
    }

    /// Replaces the profiles (after an admin edits them).
    pub fn set_profiles(&mut self, ps: &[Profile]) {
        self.0.profiles = ps.to_vec();
    }

    fn upsert_room(&mut self, r: &Room) {
        match self.0.rooms.iter_mut().find(|x| x.id == r.id) {
            Some(x) => *x = r.clone(),
            None => self.0.rooms.push(r.clone()),
        }
    }

    fn upsert_computer(&mut self, c: &Computer) {
        if c.public_key.len() != 32 {
            return;
        }
        let cc = cached_computer(c);
        match self.0.computers.iter_mut().find(|x| x.id == c.id) {
            Some(x) => *x = cc,
            None => self.0.computers.push(cc),
        }
    }

    /// The rooms `user` may control (an admin: all), by name.
    pub fn rooms_for(&self, user: &CachedUser) -> Vec<Room> {
        let admin = user.role == Role::Admin as i32;
        let mut v: Vec<Room> = self
            .0
            .rooms
            .iter()
            .filter(|r| admin || user.room_ids.contains(&r.id))
            .cloned()
            .collect();
        v.sort_by_key(|r| r.name.to_lowercase());
        v
    }

    /// The cached computers of a room.
    pub fn computers_in(&self, room_id: i64) -> Vec<CachedComputer> {
        self.0
            .computers
            .iter()
            .filter(|c| c.room_id == room_id)
            .cloned()
            .collect()
    }

    /// The cached computers of every room `user` may control.
    pub fn computers_for(&self, user: &CachedUser) -> Vec<CachedComputer> {
        self.rooms_for(user)
            .iter()
            .flat_map(|r| self.computers_in(r.id))
            .collect()
    }

    /// All synced profiles.
    pub fn profiles(&self) -> &[Profile] {
        &self.0.profiles
    }

    /// One room.
    pub fn room(&self, id: i64) -> Option<&Room> {
        self.0.rooms.iter().find(|r| r.id == id)
    }

    /// The picture shown in fallback mode: `user`'s rooms, their computers
    /// online when they answered locally, with what they answered. A room shows
    /// the profile most of its reachable computers apply.
    pub fn fleet_for(&self, user: &CachedUser, reach: &BTreeMap<u64, Reach>) -> Fleet {
        let mut rooms = self.rooms_for(user);
        let mut computers = Vec::new();
        let mut notes = BTreeMap::new();
        for r in &mut rooms {
            let mut votes: BTreeMap<i64, usize> = BTreeMap::new();
            for c in self.computers_in(r.id) {
                let state = reach.get(&c.id).cloned().unwrap_or(Reach::Unknown);
                let (online, profile) = match &state {
                    Reach::Reachable(s) => (true, s.effective_profile_id),
                    _ => (false, 0),
                };
                if online && c.exception_profile_id == 0 {
                    *votes.entry(profile).or_default() += 1;
                }
                notes.insert(c.id, note(&state).to_string());
                computers.push(Computer {
                    id: c.id,
                    name: c.name.clone(),
                    room_id: c.room_id,
                    exception_profile_id: c.exception_profile_id,
                    lan_addresses: c.lan_addresses.clone(),
                    online,
                    effective_profile_id: profile,
                    public_key: c.public_key.clone(),
                    ..Default::default()
                });
            }
            if let Some((p, _)) = votes.into_iter().max_by_key(|(_, n)| *n) {
                r.active_profile_id = p;
            }
        }
        let mut f = Fleet::from_parts(rooms, computers, self.0.profiles.clone());
        f.set_notes(notes);
        f
    }
}
