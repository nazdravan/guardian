//! State shared by every request handler.

use std::sync::Mutex;

use crate::auth::RateLimiter;
use crate::batch::Batcher;
use crate::config::ServerConfig;
use crate::db::Db;

/// Shared server state (behind an `Arc`).
#[derive(Debug)]
pub struct AppState {
    /// The database.
    pub db: Db,
    /// Batched writer for results and audit entries.
    pub batcher: Batcher,
    /// Per-IP limiter for authentication attempts.
    pub limiter: RateLimiter,
    /// The one-time bootstrap code, while no user exists.
    pub bootstrap: Mutex<Option<String>>,
    /// The configuration the server was started with.
    pub config: ServerConfig,
}

impl AppState {
    /// The current bootstrap code, if any.
    pub fn bootstrap_code(&self) -> Option<String> {
        self.bootstrap.lock().ok().and_then(|g| g.clone())
    }
}

/// Which agents a policy change affects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every connected agent.
    All,
    /// Agents in one room.
    Room(i64),
    /// One agent.
    Computer(u64),
}

impl AppState {
    /// Pushes new state to the agents in `scope` (wired to the hub in Task 12).
    pub async fn policy_changed(&self, _scope: Scope) {}

    /// Pushes trusted consoles and users to every agent (wired in Task 12).
    pub async fn trust_changed(&self) {}

    /// A user's rooms, role or active flag changed (wired in Task 14).
    pub async fn user_rooms_changed(&self, _user_id: i64) {}

    /// Whether an agent is connected (the hub, from Task 11).
    pub fn is_online(&self, _id: u64) -> bool {
        false
    }

    /// A computer's row changed (Watch event, Task 14).
    pub fn computer_changed(&self, _id: u64) {}

    /// A room was renamed or switched profile (Watch event, Task 14).
    pub fn room_changed(&self, _room_id: i64) {}

    /// A room was deleted (Watch event, Task 14).
    pub fn room_removed(&self, _room_id: i64) {}

    /// A computer was revoked (wired in Task 11).
    pub async fn computer_revoked(&self, _id: u64) {}

    /// Queues an audit entry for an action through the server.
    pub fn audit(&self, actor: Option<i64>, action: &str, target: String, detail: String) {
        self.batcher.audit(crate::batch::AuditEntry {
            actor_user_id: actor,
            action: action.into(),
            target,
            detail,
            source: crate::batch::AuditSource::Server,
        });
    }
}
