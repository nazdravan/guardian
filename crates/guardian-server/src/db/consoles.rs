//! `consoles`: teacher consoles trusted for local (offline) mode.

use guardian_core::keys::PublicKey;
use guardian_proto::v1 as pb;

use super::{Db, DbError, now_unix};

/// Registers a console and returns its id.
pub async fn insert(db: &Db, key: &PublicKey, name: &str) -> Result<i64, DbError> {
    Ok(
        sqlx::query("INSERT INTO consoles(public_key, name, created_at) VALUES (?, ?, ?)")
            .bind(key.as_bytes().to_vec())
            .bind(name)
            .bind(now_unix())
            .execute(&db.write)
            .await?
            .last_insert_rowid(),
    )
}

/// Consoles that are not revoked.
pub async fn list(db: &Db) -> Result<Vec<pb::Console>, DbError> {
    let rows: Vec<(i64, String, Vec<u8>, i64)> = sqlx::query_as(
        "SELECT id, name, public_key, created_at FROM consoles WHERE revoked_at IS NULL ORDER BY id",
    )
    .fetch_all(&db.read)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, public_key, created_at_unix)| pb::Console {
            id,
            name,
            public_key,
            created_at_unix,
        })
        .collect())
}

/// Revokes a console; `false` if it does not exist or is already revoked.
pub async fn revoke(db: &Db, id: i64) -> Result<bool, DbError> {
    let n = sqlx::query("UPDATE consoles SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(now_unix())
        .bind(id)
        .execute(&db.write)
        .await?
        .rows_affected();
    Ok(n > 0)
}

/// `(id, key)` of every non-revoked console.
pub async fn trusted(db: &Db) -> Result<Vec<(i64, PublicKey)>, DbError> {
    let rows: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT id, public_key FROM consoles WHERE revoked_at IS NULL ORDER BY id")
            .fetch_all(&db.read)
            .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, k)| PublicKey::from_bytes(&k).ok().map(|k| (id, k)))
        .collect())
}
