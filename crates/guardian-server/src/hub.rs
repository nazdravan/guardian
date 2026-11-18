//! The hub: which agents are connected, and the event bus for `Watch`.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use guardian_proto::v1::{AgentState, AuthorizedUser, ServerFrame, TrustedConsole};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::db::{self, Db, DbError};

/// Something a `Watch` stream may care about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubEvent {
    /// A computer connected, disconnected or was edited.
    ComputerChanged(u64),
    /// A computer was revoked.
    ComputerRemoved(u64),
    /// A room was renamed or switched profile.
    RoomChanged(i64),
    /// A room was deleted.
    RoomRemoved(i64),
    /// A command target changed status.
    CommandResult {
        /// Command.
        command_id: Uuid,
        /// Target.
        computer_id: u64,
        /// New status.
        status: guardian_proto::v1::ResultStatus,
        /// Agent-supplied detail.
        detail: String,
    },
    /// A user's rooms, role or active flag changed.
    UserRoomsChanged(i64),
}

/// Identifies one registration, so a replaced connection cannot unregister its successor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionToken(u64);

struct Conn {
    token: u64,
    caps: BTreeSet<String>,
    tx: mpsc::Sender<ServerFrame>,
}

/// Connected agents. Dropping a connection's sender closes its socket.
pub struct Hub {
    conns: Mutex<HashMap<u64, Conn>>,
    next: AtomicU64,
    events: broadcast::Sender<HubEvent>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub")
            .field("online", &self.online_ids().len())
            .finish()
    }
}

impl Default for Hub {
    fn default() -> Self {
        Hub::new()
    }
}

impl Hub {
    /// An empty hub.
    pub fn new() -> Hub {
        Hub {
            conns: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            events: broadcast::channel(1024).0,
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut HashMap<u64, Conn>) -> R) -> R {
        let mut g = match self.conns.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        f(&mut g)
    }

    /// Registers a connection, replacing (and thereby closing) any previous one for `id`.
    pub fn register(
        &self,
        id: u64,
        caps: BTreeSet<String>,
        tx: mpsc::Sender<ServerFrame>,
    ) -> ConnectionToken {
        let token = self.next.fetch_add(1, Ordering::Relaxed);
        let old = self.with(|m| m.insert(id, Conn { token, caps, tx }));
        if old.is_some() {
            tracing::info!("computer {id} reconnected; closing the previous connection");
        }
        ConnectionToken(token)
    }

    /// Removes the connection if it is still the current one; returns whether it was.
    pub fn unregister(&self, id: u64, token: ConnectionToken) -> bool {
        self.with(|m| match m.get(&id) {
            Some(c) if c.token == token.0 => {
                m.remove(&id);
                true
            }
            _ => false,
        })
    }

    /// Whether `id` is connected.
    pub fn is_online(&self, id: u64) -> bool {
        self.with(|m| m.contains_key(&id))
    }

    /// Queues a frame; `false` if offline or the queue is full.
    pub fn send(&self, id: u64, frame: ServerFrame) -> bool {
        self.with(|m| m.get(&id).map(|c| c.tx.try_send(frame).is_ok()))
            .unwrap_or(false)
    }

    /// Closes the connection after already-queued frames are written.
    pub fn disconnect(&self, id: u64) {
        self.with(|m| m.remove(&id));
    }

    /// Closes every connection (shutdown).
    pub fn disconnect_all(&self) {
        self.with(HashMap::clear);
    }

    /// Feature names the connected agent announced.
    pub fn capabilities(&self, id: u64) -> Option<BTreeSet<String>> {
        self.with(|m| m.get(&id).map(|c| c.caps.clone()))
    }

    /// Ids of connected agents.
    pub fn online_ids(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.with(|m| m.keys().copied().collect());
        v.sort_unstable();
        v
    }

    /// Subscribes to hub events.
    pub fn events(&self) -> broadcast::Receiver<HubEvent> {
        self.events.subscribe()
    }

    /// Publishes an event (ignored when nobody listens).
    pub fn emit(&self, e: HubEvent) {
        let _ = self.events.send(e);
    }
}

/// The state an agent enforces: its effective profile plus every profile (so local
/// mode can switch between them), its room and the state version.
///
/// The version is read *before* the data: a concurrent change then yields data at
/// least as new as the version, and that change's own push carries a higher
/// version, so an agent that applies only newer versions always converges.
pub async fn build_agent_state(db: &Db, computer_id: u64) -> Result<AgentState, DbError> {
    let version = db::settings::state_version(db).await?;
    let row = db::computers::get(db, computer_id)
        .await?
        .ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))?;
    let settings = db::settings::get(db).await?;
    let profiles = db::profiles::list(db).await?;
    Ok(AgentState {
        version,
        effective_profile_id: row.effective_profile(settings.default_profile_id),
        profiles: profiles.iter().map(|p| p.to_proto()).collect(),
        room_id: row.room_id.unwrap_or(0),
        exception_profile_id: row.exception_profile_id.unwrap_or(0),
    })
}

/// Consoles and users the agent trusts in local mode: non-revoked consoles, active
/// admins, and the active teachers of the computer's room.
pub async fn build_trust(
    db: &Db,
    computer_id: u64,
) -> Result<(Vec<TrustedConsole>, Vec<AuthorizedUser>), DbError> {
    let consoles = db::consoles::trusted(db)
        .await?
        .into_iter()
        .map(|(id, k)| TrustedConsole {
            console_id: id,
            public_key: k.as_bytes().to_vec(),
        })
        .collect();
    let room = db::computers::get(db, computer_id)
        .await?
        .and_then(|c| c.room_id);
    let users: Vec<(i64, String)> = sqlx::query_as(
        "SELECT u.id, u.login FROM users u WHERE u.active = 1 AND (u.role = 'admin' OR \
         EXISTS (SELECT 1 FROM user_rooms ur WHERE ur.user_id = u.id AND ur.room_id = ?)) ORDER BY u.id",
    )
    .bind(room.unwrap_or(-1))
    .fetch_all(&db.read)
    .await?;
    Ok((
        consoles,
        users
            .into_iter()
            .map(|(user_id, login)| AuthorizedUser { user_id, login })
            .collect(),
    ))
}
