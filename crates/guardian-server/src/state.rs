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
