//! Local fallback mode in the backend (§11): enrolling this console, keeping
//! the offline cache current while the server is reachable, and, when it is
//! not, working directly with the computers on the school network until the
//! server answers again.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures_util::StreamExt;
use futures_util::stream;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::{
    CachedUser, CreateEnrollCodeRequest, EnrollCodeKind, EnrollConsoleRequest, Mode as ProfileMode,
    Profile, Snapshot, User, WatchEvent, watch_event,
};
use tokio::task::JoinHandle;

use super::errors::status_text;
use super::{ConnState, Core, Event, FallbackState, lock};
use crate::fallback::cache::{FallbackCache, Reach};
use crate::fallback::client::{LocalClient, PARALLEL, default_broadcast};
use crate::fallback::key;
use crate::settings::{Mode, Secret, fallback_paths};

/// The cache is written at most this often while online.
const SAVE_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// The refusal for a wrong local sign-in.
pub const BAD_LOCAL_LOGIN: &str = "Unknown login or wrong password. Only users who have signed in on this computer while the server was reachable can work locally.";

type ReachMap = Arc<Mutex<BTreeMap<u64, (Reach, Option<SocketAddr>)>>>;

struct ActiveRun {
    user: CachedUser,
    reach: ReachMap,
    task: JoinHandle<()>,
}

/// Fallback state of the backend.
#[derive(Default)]
pub(crate) struct Fb {
    cache: Option<FallbackCache>,
    /// The server could not be reached the last time we tried.
    down: bool,
    /// `Available` was announced.
    announced: bool,
    active: Option<ActiveRun>,
    dirty: bool,
    saved_at: Option<Instant>,
}

/// The remote server of the current mode (`None` in embedded mode).
pub(crate) fn remote_server(core: &Core) -> Option<String> {
    match &lock(&core.settings).mode {
        Some(Mode::Remote { server, .. }) => Some(server.clone()),
        _ => None,
    }
}

fn paths(core: &Core) -> (std::path::PathBuf, std::path::PathBuf) {
    fallback_paths(&core.settings_path)
}

fn save_now(core: &Core, fb: &mut Fb) {
    if let Some(c) = &fb.cache {
        if let Err(e) = c.save(&paths(core).1) {
            tracing::warn!("cannot save the fallback cache: {e}");
        }
        fb.dirty = false;
        fb.saved_at = Some(Instant::now());
    }
}

fn save_soon(core: &Core, fb: &mut Fb) {
    fb.dirty = true;
    if fb.saved_at.is_none_or(|t| t.elapsed() >= SAVE_EVERY) {
        save_now(core, fb);
    }
}

/// The user of the saved session, if the cache knows them (a saved token is
/// proof of an earlier sign-in on this computer).
fn session_user(core: &Core, cache: &FallbackCache) -> Option<CachedUser> {
    if let Some(u) = lock(&core.user).as_ref() {
        return cache.user(u.id);
    }
    let s = lock(&core.settings);
    s.token()?;
    cache.user_by_login(s.login.as_deref()?)
}

fn can_work_locally(core: &Core, fb: &Fb) -> bool {
    fb.down
        && fb
            .cache
            .as_ref()
            .is_some_and(|c| c.has_users() || session_user(core, c).is_some())
}

/// A session to `server` starts (or `None`: embedded): loads its cache.
pub(crate) fn session_started(core: &Core, server: Option<&str>) {
    let id = {
        let mut fb = lock(&core.fallback);
        stop_active(&mut fb);
        fb.cache = server.and_then(|s| FallbackCache::load(&paths(core).1, s));
        fb.down = false;
        fb.announced = false;
        fb.cache.as_ref().map(FallbackCache::console_id)
    };
    core.send_event(Event::FallbackEnabled(id));
    core.send_event(Event::Fallback(FallbackState::Off));
}

fn stop_active(fb: &mut Fb) -> bool {
    match fb.active.take() {
        Some(a) => {
            a.task.abort();
            true
        }
        None => false,
    }
}

/// Follows the connection state: announces fallback mode when the server is
/// down, and ends it as soon as the server answers.
pub(crate) fn on_connection(core: &Core, st: &ConnState) {
    let mut out = Vec::new();
    {
        let mut fb = lock(&core.fallback);
        match st {
            ConnState::Reconnecting { .. } => {
                fb.down = true;
                if fb.dirty {
                    save_now(core, &mut fb);
                }
                if fb.active.is_none() && !fb.announced && can_work_locally(core, &fb) {
                    fb.announced = true;
                    out.push(Event::Fallback(FallbackState::Available));
                }
            }
            ConnState::Online | ConnState::NeedsLogin { .. } => {
                fb.down = false;
                let was_active = stop_active(&mut fb);
                if was_active || fb.announced {
                    out.push(Event::Fallback(FallbackState::Off));
                }
                if was_active {
                    out.push(Event::Done(
                        "The server is back; fallback mode ended.".into(),
                    ));
                }
                fb.announced = false;
            }
            ConnState::Disconnected => {
                fb.down = false;
                fb.announced = false;
                if stop_active(&mut fb) {
                    out.push(Event::Fallback(FallbackState::Off));
                }
            }
            ConnState::Connecting => {}
        }
    }
    for e in out {
        core.send_event(e);
    }
}

/// A sign-in failed because the server cannot be reached.
pub(crate) fn server_unreachable(core: &Core) {
    on_connection(
        core,
        &ConnState::Reconnecting {
            attempt: 1,
            error: String::new(),
        },
    );
}

/// Keeps the cache current with a `Watch` event.
pub(crate) fn on_watch(core: &Core, ev: &WatchEvent) {
    let mut fb = lock(&core.fallback);
    if let Some(c) = fb.cache.as_mut() {
        c.merge_event(ev);
        save_soon(core, &mut fb);
    }
}

/// Keeps the cache's profiles current after an admin edit.
pub(crate) fn on_profiles(core: &Core, ps: &[Profile]) {
    let mut fb = lock(&core.fallback);
    if let Some(c) = fb.cache.as_mut() {
        c.set_profiles(ps);
        save_soon(core, &mut fb);
    }
}

/// Signed in (or the session resumed): role and rooms into the cache.
pub(crate) fn on_user(core: &Core, u: &User) {
    let mut fb = lock(&core.fallback);
    if let Some(c) = fb.cache.as_mut() {
        c.update_user(u);
        save_now(core, &mut fb);
    }
}

/// A successful online sign-in: a verifier for local sign-in later.
pub(crate) async fn remember_login(core: &Arc<Core>, u: &User, password: &Secret) {
    let cache = lock(&core.fallback).cache.clone();
    let Some(mut cache) = cache else { return };
    let (user, pw) = (u.clone(), password.clone());
    let done = tokio::task::spawn_blocking(move || {
        let r = cache.remember_login(&user, &pw);
        (cache, r)
    })
    .await;
    if let Ok((cache, Ok(()))) = done {
        let mut fb = lock(&core.fallback);
        fb.cache = Some(cache);
        save_now(core, &mut fb);
    }
}

/// The current fleet as a snapshot (the cache starts from it).
fn snapshot_of(core: &Core) -> WatchEvent {
    let f = lock(&core.fleet).clone();
    WatchEvent {
        event: Some(watch_event::Event::Snapshot(Snapshot {
            rooms: f.raw_rooms(),
            computers: f.raw_computers(),
            profiles: f.profiles().into_iter().cloned().collect(),
        })),
    }
}

/// `EnableFallback`: enrolls this console (creating the code for an admin).
pub(crate) async fn enable(core: Arc<Core>, code: Option<String>) {
    let Some(server) = remote_server(&core) else {
        core.emit(Event::Error(
            "Local fallback is only for a remote server.".into(),
        ));
        return;
    };
    let Some(ch) = core.channel() else {
        core.emit(Event::Error("Not connected to a server yet.".into()));
        return;
    };
    let code = match code {
        Some(c) => c.trim().to_string(),
        None => {
            let Some(a) = core.authed() else {
                core.emit(Event::Error("Sign in first.".into()));
                return;
            };
            match AdminClient::new(a)
                .create_enroll_code(CreateEnrollCodeRequest {
                    kind: EnrollCodeKind::EnrollConsole as i32,
                    room_id: 0,
                    max_uses: 1,
                    ttl_seconds: 900,
                })
                .await
            {
                Ok(c) => c.into_inner().code,
                Err(st) => {
                    core.emit(Event::Error(status_text(&st)));
                    return;
                }
            }
        }
    };
    let (key_path, _) = paths(&core);
    let key = match key::load_or_create(&key_path) {
        Ok(k) => k,
        Err(e) => {
            core.emit(Event::Error(format!("Cannot create the console key: {e}")));
            return;
        }
    };
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    let id = match AuthClient::new(ch)
        .enroll_console(EnrollConsoleRequest {
            code,
            public_key: key.public().as_bytes().to_vec(),
            name: if name.is_empty() {
                "console".into()
            } else {
                name
            },
        })
        .await
    {
        Ok(r) => r.into_inner().console_id,
        Err(st) => {
            core.emit(Event::Error(status_text(&st)));
            return;
        }
    };
    let mut cache = FallbackCache::new(&server, id);
    cache.merge_event(&snapshot_of(&core));
    let user = lock(&core.user).clone();
    let secret = lock(&core.login_secret).clone();
    if let Some(u) = &user {
        match secret {
            Some((uid, pw)) if uid == u.id => {
                let u2 = u.clone();
                cache = match tokio::task::spawn_blocking(move || {
                    let _ = cache.remember_login(&u2, &pw);
                    cache
                })
                .await
                {
                    Ok(c) => c,
                    Err(_) => FallbackCache::new(&server, id),
                };
            }
            _ => cache.update_user(u),
        }
    }
    {
        let mut fb = lock(&core.fallback);
        fb.cache = Some(cache);
        save_now(&core, &mut fb);
    }
    core.emit(Event::Done(
        "Local fallback is enabled on this computer.".into(),
    ));
    core.emit(Event::FallbackEnabled(Some(id)));
}

/// `ForgetMode`: the key and the cache go too.
pub(crate) fn forget(core: &Core) {
    {
        let mut fb = lock(&core.fallback);
        stop_active(&mut fb);
        *fb = Fb::default();
    }
    let (k, c) = paths(core);
    for p in [k, c] {
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("cannot remove {}: {e}", p.display()),
        }
    }
    core.emit(Event::FallbackEnabled(None));
    core.emit(Event::Fallback(FallbackState::Off));
}

fn client(core: &Core, cache: &FallbackCache) -> Option<LocalClient> {
    let k = key::load_or_create(&paths(core).0).ok()?;
    let port = lock(&core.settings).fallback_port();
    let broadcast = core
        .opts
        .fallback_broadcast
        .clone()
        .unwrap_or_else(|| default_broadcast(port));
    Some(LocalClient::new(k, cache.console_id(), port).with_broadcast(broadcast))
}

/// `FallbackLogin`: a local sign-in against the cached verifier.
pub(crate) async fn login(core: Arc<Core>, login: String, password: Secret) {
    let (cache, down) = {
        let fb = lock(&core.fallback);
        (fb.cache.clone(), fb.down)
    };
    let Some(cache) = cache else {
        core.emit(Event::Error(
            "Local fallback is not enabled on this computer.".into(),
        ));
        return;
    };
    if !down {
        core.emit(Event::Error(
            "The server is reachable; sign in normally.".into(),
        ));
        return;
    }
    let found = tokio::task::spawn_blocking(move || cache.verify(&login, &password))
        .await
        .ok()
        .flatten();
    match found {
        Some(u) => start(core, u),
        None => core.emit(Event::Error(BAD_LOCAL_LOGIN.into())),
    }
}

/// `EnterFallback`: the signed-in (or saved-session) user goes local.
pub(crate) fn enter(core: Arc<Core>) {
    let user = {
        let fb = lock(&core.fallback);
        if !fb.down {
            None
        } else {
            fb.cache.as_ref().and_then(|c| session_user(&core, c))
        }
    };
    match user {
        Some(u) => start(core, u),
        None => core.emit(Event::Error(
            "Fallback mode is available only while the server cannot be reached.".into(),
        )),
    }
}

/// `LeaveFallback`.
pub(crate) fn leave(core: &Core) {
    let next = {
        let mut fb = lock(&core.fallback);
        stop_active(&mut fb);
        if can_work_locally(core, &fb) {
            fb.announced = true;
            FallbackState::Available
        } else {
            fb.announced = false;
            FallbackState::Off
        }
    };
    core.emit(Event::Fallback(next));
}

/// Whether fallback mode is on (room switches go to the agents directly).
pub(crate) fn is_active(core: &Core) -> bool {
    lock(&core.fallback).active.is_some()
}

fn view(core: &Core, user: &CachedUser, reach: &ReachMap) -> Option<Event> {
    let fb = lock(&core.fallback);
    let cache = fb.cache.as_ref()?;
    let r: BTreeMap<u64, Reach> = lock(reach)
        .iter()
        .map(|(id, (x, _))| (*id, x.clone()))
        .collect();
    Some(Event::Fallback(FallbackState::Active {
        user: user.clone(),
        fleet: cache.fleet_for(user, &r),
    }))
}

fn start(core: Arc<Core>, user: CachedUser) {
    let Some(cache) = lock(&core.fallback).cache.clone() else {
        return;
    };
    let Some(client) = client(&core, &cache) else {
        core.emit(Event::Error("Cannot read the console key.".into()));
        return;
    };
    let reach: ReachMap = Arc::new(Mutex::new(BTreeMap::new()));
    let task = tokio::spawn({
        let (core, user, reach) = (core.clone(), user.clone(), reach.clone());
        async move {
            loop {
                let computers = match lock(&core.fallback).cache.as_ref() {
                    Some(c) => c.computers_for(&user),
                    None => return,
                };
                let known: BTreeMap<u64, SocketAddr> = lock(&reach)
                    .iter()
                    .filter_map(|(id, (_, a))| a.map(|a| (*id, a)))
                    .collect();
                let out = client.sweep(&computers, &user, &known).await;
                {
                    let mut r = lock(&reach);
                    for (id, (x, a)) in out {
                        let keep = r.get(&id).and_then(|(_, old)| *old);
                        r.insert(id, (x, a.or(keep)));
                    }
                }
                if let Some(e) = view(&core, &user, &reach) {
                    core.emit(e);
                }
                tokio::time::sleep(core.opts.fallback_sweep_every).await;
            }
        }
    });
    {
        let mut fb = lock(&core.fallback);
        stop_active(&mut fb);
        fb.announced = false;
        fb.active = Some(ActiveRun {
            user: user.clone(),
            reach: reach.clone(),
            task,
        });
    }
    tracing::info!("fallback mode: working locally as {}", user.login);
    if let Some(e) = view(&core, &user, &reach) {
        core.emit(e);
    }
}

/// `SetRoomProfile` in fallback mode: straight to each computer of the room.
pub(crate) async fn set_room_profile(core: Arc<Core>, room_id: i64, profile_id: i64) {
    let (cache, user, reach) = {
        let fb = lock(&core.fallback);
        match (&fb.cache, &fb.active) {
            (Some(c), Some(a)) => (c.clone(), a.user.clone(), a.reach.clone()),
            _ => return,
        }
    };
    let Some(profile) = cache
        .profiles()
        .iter()
        .find(|p| p.id == profile_id)
        .cloned()
    else {
        core.emit(Event::Error("This profile is not known.".into()));
        return;
    };
    if profile.mode == ProfileMode::Free as i32 {
        core.emit(Event::Error(
            "A free profile cannot be set in fallback mode.".into(),
        ));
        return;
    }
    let Some(room) = cache.rooms_for(&user).into_iter().find(|r| r.id == room_id) else {
        core.emit(Event::Error("You may not control this room.".into()));
        return;
    };
    let Some(client) = client(&core, &cache) else {
        core.emit(Event::Error("Cannot read the console key.".into()));
        return;
    };
    let computers = cache.computers_in(room_id);
    let total = computers.len();
    let mut known: BTreeMap<u64, SocketAddr> = lock(&reach)
        .iter()
        .filter_map(|(id, (_, a))| a.map(|a| (*id, a)))
        .collect();
    // Computers not located yet are looked for first (cached address, then
    // discovery), so a switch right after entering fallback mode reaches them.
    let unknown: Vec<_> = computers
        .iter()
        .filter(|c| !known.contains_key(&c.id))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        for (id, (_, a)) in client.sweep(&unknown, &user, &known).await {
            if let Some(a) = a {
                known.insert(id, a);
            }
        }
    }
    let results: Vec<(u64, Reach, Option<SocketAddr>)> = stream::iter(computers)
        .map(|c| {
            let (client, user, hint) = (client.clone(), user.clone(), known.get(&c.id).copied());
            async move {
                match hint {
                    Some(a) => {
                        let r = client.set_profile(a, &c, &user, room_id, profile_id).await;
                        (c.id, r, Some(a))
                    }
                    None => (c.id, Reach::Unreachable, None),
                }
            }
        })
        .buffer_unordered(PARALLEL)
        .collect()
        .await;
    let applied = results
        .iter()
        .filter(|(_, r, _)| matches!(r, Reach::Reachable(_)))
        .count();
    let unreachable = results
        .iter()
        .filter(|(_, r, _)| matches!(r, Reach::Unreachable))
        .count();
    {
        let mut r = lock(&reach);
        for (id, x, a) in results {
            r.insert(id, (x, a));
        }
    }
    let mut msg = format!(
        "{} applied on {applied} of {total} computers in {}.",
        profile.name, room.name
    );
    if unreachable > 0 {
        msg.push_str(&format!(" {unreachable} could not be reached."));
    }
    if let Some(e) = view(&core, &user, &reach) {
        core.emit(e);
    }
    core.emit(Event::Done(msg));
}
