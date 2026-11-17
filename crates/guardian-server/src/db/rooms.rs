//! `rooms`.

use guardian_proto::v1 as pb;

use super::settings::bump_state_version;
use super::{Db, DbError};

/// Every room, by name.
pub async fn list(db: &Db) -> Result<Vec<pb::Room>, DbError> {
    let rows: Vec<(i64, String, i64)> =
        sqlx::query_as("SELECT id, name, active_profile_id FROM rooms ORDER BY name")
            .fetch_all(&db.read)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name, active_profile_id)| pb::Room {
            id,
            name,
            active_profile_id,
        })
        .collect())
}

/// One room.
pub async fn get(db: &Db, id: i64) -> Result<Option<pb::Room>, DbError> {
    let r: Option<(i64, String, i64)> =
        sqlx::query_as("SELECT id, name, active_profile_id FROM rooms WHERE id = ?")
            .bind(id)
            .fetch_optional(&db.read)
            .await?;
    Ok(r.map(|(id, name, active_profile_id)| pb::Room {
        id,
        name,
        active_profile_id,
    }))
}

/// A room by exact name.
pub async fn by_name(db: &Db, name: &str) -> Result<Option<pb::Room>, DbError> {
    let id: Option<i64> = sqlx::query_scalar("SELECT id FROM rooms WHERE name = ?")
        .bind(name)
        .fetch_optional(&db.read)
        .await?;
    match id {
        Some(id) => get(db, id).await,
        None => Ok(None),
    }
}

/// Creates a room whose active profile is the school default.
pub async fn create(db: &Db, name: &str) -> Result<pb::Room, DbError> {
    let id = sqlx::query(
        "INSERT INTO rooms(name, active_profile_id) \
         SELECT ?, default_profile_id FROM settings WHERE id = 1",
    )
    .bind(name)
    .execute(&db.write)
    .await?
    .last_insert_rowid();
    get(db, id)
        .await?
        .ok_or(DbError::Sqlx(sqlx::Error::RowNotFound))
}

/// Renames a room; `None` if it does not exist.
pub async fn rename(db: &Db, id: i64, name: &str) -> Result<Option<pb::Room>, DbError> {
    let n = sqlx::query("UPDATE rooms SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(&db.write)
        .await?
        .rows_affected();
    if n == 0 { Ok(None) } else { get(db, id).await }
}

/// Deletes a room (its computers lose their room); `false` if it did not exist.
pub async fn delete(db: &Db, id: i64) -> Result<bool, DbError> {
    let mut tx = db.write.begin().await?;
    let n = sqlx::query("DELETE FROM rooms WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n > 0 {
        bump_state_version(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(n > 0)
}

/// Sets the active profile; `None` if the room does not exist.
pub async fn set_active_profile(
    db: &Db,
    room_id: i64,
    profile_id: i64,
) -> Result<Option<pb::Room>, DbError> {
    let mut tx = db.write.begin().await?;
    let n = sqlx::query("UPDATE rooms SET active_profile_id = ? WHERE id = ?")
        .bind(profile_id)
        .bind(room_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Ok(None);
    }
    bump_state_version(&mut tx).await?;
    tx.commit().await?;
    get(db, room_id).await
}
