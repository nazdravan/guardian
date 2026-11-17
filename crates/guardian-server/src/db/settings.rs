//! The single `settings` row.

use guardian_proto::v1 as pb;
use sqlx::SqliteConnection;

use super::{Db, DbError};

/// Current settings.
pub async fn get(db: &Db) -> Result<pb::Settings, DbError> {
    let (default_profile_id, retention_days): (i64, i64) =
        sqlx::query_as("SELECT default_profile_id, retention_days FROM settings WHERE id = 1")
            .fetch_one(&db.read)
            .await?;
    Ok(pb::Settings {
        default_profile_id,
        retention_days: u32::try_from(retention_days).unwrap_or(90),
    })
}

/// Writes settings (the caller has validated them) and bumps the state version.
pub async fn update(db: &Db, s: &pb::Settings) -> Result<(), DbError> {
    let mut tx = db.write.begin().await?;
    sqlx::query("UPDATE settings SET default_profile_id = ?, retention_days = ? WHERE id = 1")
        .bind(s.default_profile_id)
        .bind(i64::from(s.retention_days))
        .execute(&mut *tx)
        .await?;
    bump_state_version(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Increments `state_version` inside the caller's transaction and returns the new value.
pub async fn bump_state_version(conn: &mut SqliteConnection) -> Result<u64, DbError> {
    let v: i64 = sqlx::query_scalar(
        "UPDATE settings SET state_version = state_version + 1 WHERE id = 1 RETURNING state_version",
    )
    .fetch_one(conn)
    .await?;
    Ok(u64::try_from(v).unwrap_or(0))
}

/// The current state version.
pub async fn state_version(db: &Db) -> Result<u64, DbError> {
    let v: i64 = sqlx::query_scalar("SELECT state_version FROM settings WHERE id = 1")
        .fetch_one(&db.read)
        .await?;
    Ok(u64::try_from(v).unwrap_or(0))
}
