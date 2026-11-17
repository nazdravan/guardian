//! `sessions`: opaque tokens, stored as SHA-256 hex.

use guardian_proto::v1::Role;
use sqlx::Row;

use super::{Db, DbError};

/// The active user behind a valid session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionUser {
    /// User id.
    pub id: i64,
    /// Login.
    pub login: String,
    /// Role.
    pub role: Role,
}

/// Stores a session.
pub async fn create(
    db: &Db,
    token_hash: &str,
    user_id: i64,
    expires_at: i64,
) -> Result<(), DbError> {
    sqlx::query("INSERT INTO sessions(token_hash, user_id, expires_at) VALUES (?, ?, ?)")
        .bind(token_hash)
        .bind(user_id)
        .bind(expires_at)
        .execute(&db.write)
        .await?;
    Ok(())
}

/// The active user of an unexpired session.
pub async fn lookup(db: &Db, token_hash: &str, now: i64) -> Result<Option<SessionUser>, DbError> {
    let r = sqlx::query(
        "SELECT u.id, u.login, u.role FROM sessions s JOIN users u ON u.id = s.user_id \
         WHERE s.token_hash = ? AND s.expires_at > ? AND u.active = 1",
    )
    .bind(token_hash)
    .bind(now)
    .fetch_optional(&db.read)
    .await?;
    let Some(r) = r else { return Ok(None) };
    Ok(Some(SessionUser {
        id: r.try_get("id")?,
        login: r.try_get("login")?,
        role: if r.try_get::<&str, _>("role")? == "admin" {
            Role::Admin
        } else {
            Role::Teacher
        },
    }))
}

/// Deletes one session.
pub async fn delete(db: &Db, token_hash: &str) -> Result<(), DbError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(token_hash)
        .execute(&db.write)
        .await?;
    Ok(())
}

/// Deletes every session of `user_id` except `keep`.
pub async fn delete_for_user(db: &Db, user_id: i64, keep: Option<&str>) -> Result<(), DbError> {
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash != ?")
        .bind(user_id)
        .bind(keep.unwrap_or(""))
        .execute(&db.write)
        .await?;
    Ok(())
}
