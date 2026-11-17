//! Passwords, session tokens, the authenticated caller, and rate limiting.

use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use axum::extract::ConnectInfo;
use base64::Engine;
use guardian_core::random::random_bytes;
use guardian_proto::v1::Role;
use sha2::{Digest, Sha256};
use tonic::Status;

use crate::db::{self, now_unix};
use crate::state::AppState;

/// Session lifetime: 7 days.
pub const SESSION_TTL_SECS: i64 = 7 * 24 * 60 * 60;
/// Minimum password length.
pub const MIN_PASSWORD_LEN: usize = 8;

/// A password could not be hashed.
#[derive(Debug, thiserror::Error)]
#[error("password hashing failed")]
pub struct AuthError;

/// Hashes `p` with argon2id (default parameters, random salt).
pub fn hash_password(p: &str) -> Result<String, AuthError> {
    Argon2::default()
        .hash_password(p.as_bytes())
        .map(|h| h.to_string())
        .map_err(|_| AuthError)
}

/// `true` if `p` matches the PHC string `hash`.
pub fn verify_password(p: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .ok()
        .is_some_and(|h| Argon2::default().verify_password(p.as_bytes(), &h).is_ok())
}

/// Runs argon2 off the async runtime.
pub async fn hash_password_async(p: String) -> Result<String, Status> {
    tokio::task::spawn_blocking(move || hash_password(&p))
        .await
        .map_err(|_| Status::internal("hashing task failed"))?
        .map_err(|_| Status::internal("password hashing failed"))
}

/// Verifies off the async runtime. With `hash = None` it still spends the same time
/// (against a dummy hash) so unknown logins cannot be told apart by timing.
pub async fn verify_password_async(p: String, hash: Option<String>) -> bool {
    static DUMMY: OnceLock<Option<String>> = OnceLock::new();
    tokio::task::spawn_blocking(move || match hash {
        Some(h) => verify_password(&p, &h),
        None => {
            if let Some(d) = DUMMY.get_or_init(|| hash_password("dummy-password").ok()) {
                let _ = verify_password(&p, d);
            }
            false
        }
    })
    .await
    .unwrap_or(false)
}

/// A new opaque session token (32 random bytes, base64url) and its SHA-256 hex.
pub fn new_session_token() -> (String, String) {
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes::<32>());
    let hash = token_hash(&token);
    (token, hash)
}

/// SHA-256 hex of a token, as stored in `sessions.token_hash`.
pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// The authenticated user behind a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// User id.
    pub user_id: i64,
    /// Login.
    pub login: String,
    /// Role.
    pub role: Role,
    /// Rooms the user may control (teachers).
    pub room_ids: BTreeSet<i64>,
    /// Hash of the session token used.
    pub session_hash: String,
}

impl Caller {
    /// `PERMISSION_DENIED` unless the caller is an admin.
    pub fn require_admin(&self) -> Result<(), Status> {
        if self.role == Role::Admin {
            Ok(())
        } else {
            Err(Status::permission_denied("admin role required"))
        }
    }

    /// Admins control every room; teachers only theirs.
    pub fn can_control_room(&self, room_id: i64) -> bool {
        self.role == Role::Admin || self.room_ids.contains(&room_id)
    }
}

/// The bearer token from `authorization: Bearer <token>`.
pub fn bearer<T>(req: &tonic::Request<T>) -> Option<&str> {
    req.metadata()
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// Loads the caller for `req` from the database, on every call (no cache), so a
/// disabled user or a removed room takes effect immediately.
pub async fn caller_from_request<T>(
    state: &AppState,
    req: &tonic::Request<T>,
) -> Result<Caller, Status> {
    let token = bearer(req).ok_or_else(|| Status::unauthenticated("missing token"))?;
    caller_from_token(state, token).await
}

/// Like [`caller_from_request`], from a raw token.
pub async fn caller_from_token(state: &AppState, token: &str) -> Result<Caller, Status> {
    let hash = token_hash(token);
    let user = db::sessions::lookup(&state.db, &hash, now_unix())
        .await?
        .ok_or_else(|| Status::unauthenticated("invalid or expired session"))?;
    let room_ids = db::users::rooms_of(&state.db, user.id).await?;
    Ok(Caller {
        user_id: user.id,
        login: user.login,
        role: user.role,
        room_ids,
        session_hash: hash,
    })
}

/// The client IP of a gRPC request.
pub fn client_ip<T>(req: &tonic::Request<T>) -> Option<IpAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip())
}

/// Per-IP limiter for **failed** authentication attempts (wrong password, wrong or
/// used-up code). Successful attempts are free, so a whole room enrolling — or
/// every teacher logging in — through one NAT or proxy address is never blocked,
/// while guessing is limited to `per_minute` failures per IP per minute.
pub struct RateLimiter {
    per_minute: usize,
    failures: std::sync::Mutex<
        std::collections::HashMap<IpAddr, std::collections::VecDeque<std::time::Instant>>,
    >,
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field("per_minute", &self.per_minute)
            .finish_non_exhaustive()
    }
}

const WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

impl RateLimiter {
    /// Allows `per_minute` failures per IP per minute (at least 1).
    pub fn new(per_minute: u32) -> RateLimiter {
        RateLimiter {
            per_minute: usize::try_from(per_minute.max(1)).unwrap_or(usize::MAX),
            failures: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn with<R>(
        &self,
        f: impl FnOnce(
            &mut std::collections::HashMap<IpAddr, std::collections::VecDeque<std::time::Instant>>,
        ) -> R,
    ) -> R {
        let mut g = match self.failures.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        f(&mut g)
    }

    /// `RESOURCE_EXHAUSTED` while `ip` has `per_minute` failures in the last minute.
    pub fn check(&self, ip: IpAddr) -> Result<(), Status> {
        let now = std::time::Instant::now();
        let blocked = self.with(|m| {
            let Some(q) = m.get_mut(&ip) else {
                return false;
            };
            while q.front().is_some_and(|t| now.duration_since(*t) >= WINDOW) {
                q.pop_front();
            }
            if q.is_empty() {
                m.remove(&ip);
                return false;
            }
            q.len() >= self.per_minute
        });
        if blocked {
            Err(Status::resource_exhausted(
                "too many failed attempts, try again later",
            ))
        } else {
            Ok(())
        }
    }

    /// Records a failed attempt from `ip`.
    pub fn failed(&self, ip: IpAddr) {
        let now = std::time::Instant::now();
        self.with(|m| {
            if m.len() > 10_000 {
                m.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) < WINDOW));
            }
            let q = m.entry(ip).or_default();
            q.push_back(now);
            while q.len() > self.per_minute {
                q.pop_front();
            }
        });
    }

    /// [`RateLimiter::check`] for the peer of a request (unknown peers share one bucket).
    pub fn check_request<T>(&self, req: &tonic::Request<T>) -> Result<(), Status> {
        self.check(request_ip(req))
    }
}

/// The peer IP of a request, or `0.0.0.0` when unknown.
pub fn request_ip<T>(req: &tonic::Request<T>) -> IpAddr {
    client_ip(req).unwrap_or(IpAddr::from([0, 0, 0, 0]))
}

/// Logins: `[a-z0-9._-]`, 3–32 characters.
pub fn validate_login(login: &str) -> Result<(), Status> {
    let ok = (3..=32).contains(&login.len())
        && login
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(Status::invalid_argument(
            "login must be 3-32 characters of a-z, 0-9, '.', '_' or '-'",
        ))
    }
}

/// Passwords: at least [`MIN_PASSWORD_LEN`] characters.
pub fn validate_password(p: &str) -> Result<(), Status> {
    if p.chars().count() >= MIN_PASSWORD_LEN {
        Ok(())
    } else {
        Err(Status::invalid_argument(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )))
    }
}
