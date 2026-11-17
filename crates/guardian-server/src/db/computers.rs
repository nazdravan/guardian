//! `computers`: enrolled agents.

use guardian_core::keys::PublicKey;
use guardian_core::policy::{ProfileId, effective_profile_id};
use guardian_proto::v1 as pb;
use sqlx::Row;

use super::{Db, DbError, now_unix};

/// A computer row, with its room's active profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerRow {
    /// Id (also the agent's identity in the handshake).
    pub id: u64,
    /// ed25519 public key.
    pub public_key: Vec<u8>,
    /// Admin-given name.
    pub name: String,
    /// Reported hostname.
    pub hostname: String,
    /// `windows` / `linux`.
    pub os: String,
    /// Reported agent version.
    pub agent_version: String,
    /// Room, if any.
    pub room_id: Option<i64>,
    /// Per-computer exception profile.
    pub exception_profile_id: Option<i64>,
    /// Reported LAN addresses.
    pub lan_addresses: Vec<String>,
    /// Last time the agent was seen.
    pub last_seen: Option<i64>,
    /// Active profile of the room.
    pub room_profile_id: Option<i64>,
}

impl ComputerRow {
    /// Exception, then room profile, then the school default.
    pub fn effective_profile(&self, school_default: i64) -> i64 {
        effective_profile_id(
            self.exception_profile_id.map(ProfileId),
            self.room_profile_id.map(ProfileId),
            ProfileId(school_default),
        )
        .0
    }

    /// The protobuf form.
    pub fn to_proto(&self, online: bool, school_default: i64) -> pb::Computer {
        pb::Computer {
            id: self.id,
            name: self.name.clone(),
            hostname: self.hostname.clone(),
            os: self.os.clone(),
            agent_version: self.agent_version.clone(),
            room_id: self.room_id.unwrap_or(0),
            exception_profile_id: self.exception_profile_id.unwrap_or(0),
            lan_addresses: self.lan_addresses.clone(),
            online,
            last_seen_unix: self.last_seen.unwrap_or(0),
            effective_profile_id: self.effective_profile(school_default),
        }
    }
}

const SELECT: &str = "SELECT c.id, c.public_key, c.name, c.hostname, c.os, c.agent_version, \
     c.room_id, c.exception_profile_id, c.lan_addresses, c.last_seen, r.active_profile_id AS room_profile_id \
     FROM computers c LEFT JOIN rooms r ON r.id = c.room_id";

fn row(r: &sqlx::sqlite::SqliteRow) -> Result<ComputerRow, sqlx::Error> {
    let lan: String = r.try_get("lan_addresses")?;
    Ok(ComputerRow {
        id: u64::try_from(r.try_get::<i64, _>("id")?).unwrap_or(0),
        public_key: r.try_get("public_key")?,
        name: r.try_get("name")?,
        hostname: r.try_get("hostname")?,
        os: r.try_get("os")?,
        agent_version: r.try_get("agent_version")?,
        room_id: r.try_get("room_id")?,
        exception_profile_id: r.try_get("exception_profile_id")?,
        lan_addresses: serde_json::from_str(&lan).unwrap_or_default(),
        last_seen: r.try_get("last_seen")?,
        room_profile_id: r.try_get("room_profile_id")?,
    })
}

fn id_i64(id: u64) -> i64 {
    i64::try_from(id).unwrap_or(i64::MAX)
}

fn clean_info(info: &pb::SystemInfo) -> (String, String, String, String) {
    let cut = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    let lan: Vec<String> = info
        .lan_addresses
        .iter()
        .take(16)
        .map(|a| cut(a, 64))
        .collect();
    (
        cut(&info.hostname, 255),
        cut(&info.os, 32),
        cut(&info.agent_version, 64),
        serde_json::to_string(&lan).unwrap_or_else(|_| "[]".into()),
    )
}

/// Registers a computer and returns its id.
pub async fn insert(
    db: &Db,
    key: &PublicKey,
    info: &pb::SystemInfo,
    room_id: Option<i64>,
) -> Result<u64, DbError> {
    let (hostname, os, version, lan) = clean_info(info);
    let id = sqlx::query(
        "INSERT INTO computers(public_key, name, hostname, os, agent_version, room_id, lan_addresses, enrolled_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(key.as_bytes().to_vec())
    .bind(&hostname)
    .bind(&hostname)
    .bind(os)
    .bind(version)
    .bind(room_id)
    .bind(lan)
    .bind(now_unix())
    .execute(&db.write)
    .await?
    .last_insert_rowid();
    Ok(u64::try_from(id).unwrap_or(0))
}

/// One computer.
pub async fn get(db: &Db, id: u64) -> Result<Option<ComputerRow>, DbError> {
    let q = format!("{SELECT} WHERE c.id = ?");
    let r = sqlx::query(sqlx::AssertSqlSafe(q))
        .bind(id_i64(id))
        .fetch_optional(&db.read)
        .await?;
    Ok(r.as_ref().map(row).transpose()?)
}

/// The id registered for a public key.
pub async fn by_key(db: &Db, key: &PublicKey) -> Result<Option<u64>, DbError> {
    let id: Option<i64> = sqlx::query_scalar("SELECT id FROM computers WHERE public_key = ?")
        .bind(key.as_bytes().to_vec())
        .fetch_optional(&db.read)
        .await?;
    Ok(id.and_then(|i| u64::try_from(i).ok()))
}

/// Every computer, by id.
pub async fn list(db: &Db) -> Result<Vec<ComputerRow>, DbError> {
    let q = format!("{SELECT} ORDER BY c.id");
    let rows = sqlx::query(sqlx::AssertSqlSafe(q))
        .fetch_all(&db.read)
        .await?;
    Ok(rows.iter().map(row).collect::<Result<_, _>>()?)
}

/// Ids of the computers in a room.
pub async fn in_room(db: &Db, room_id: i64) -> Result<Vec<u64>, DbError> {
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM computers WHERE room_id = ? ORDER BY id")
            .bind(room_id)
            .fetch_all(&db.read)
            .await?;
    Ok(ids
        .into_iter()
        .filter_map(|i| u64::try_from(i).ok())
        .collect())
}

/// A change to apply; `Some(None)` clears a reference.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ComputerUpdate {
    /// New name.
    pub name: Option<String>,
    /// New room (`Some(None)` = no room).
    pub room_id: Option<Option<i64>>,
    /// New exception (`Some(None)` = none).
    pub exception_profile_id: Option<Option<i64>>,
}

/// Applies an update (bumping the state version when policy is affected);
/// `false` if the computer does not exist.
pub async fn update(db: &Db, id: u64, u: &ComputerUpdate) -> Result<bool, DbError> {
    let mut tx = db.write.begin().await?;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM computers WHERE id = ?")
        .bind(id_i64(id))
        .fetch_one(&mut *tx)
        .await?;
    if n == 0 {
        return Ok(false);
    }
    if let Some(name) = &u.name {
        sqlx::query("UPDATE computers SET name = ? WHERE id = ?")
            .bind(name)
            .bind(id_i64(id))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(room) = u.room_id {
        sqlx::query("UPDATE computers SET room_id = ? WHERE id = ?")
            .bind(room)
            .bind(id_i64(id))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(exc) = u.exception_profile_id {
        sqlx::query("UPDATE computers SET exception_profile_id = ? WHERE id = ?")
            .bind(exc)
            .bind(id_i64(id))
            .execute(&mut *tx)
            .await?;
    }
    if u.room_id.is_some() || u.exception_profile_id.is_some() {
        super::settings::bump_state_version(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// Deletes a computer; `false` if it did not exist.
pub async fn delete(db: &Db, id: u64) -> Result<bool, DbError> {
    let n = sqlx::query("DELETE FROM computers WHERE id = ?")
        .bind(id_i64(id))
        .execute(&db.write)
        .await?
        .rows_affected();
    Ok(n > 0)
}

/// Records when the agent was last seen.
pub async fn touch_last_seen(db: &Db, id: u64, at: i64) -> Result<(), DbError> {
    sqlx::query("UPDATE computers SET last_seen = ? WHERE id = ?")
        .bind(at)
        .bind(id_i64(id))
        .execute(&db.write)
        .await?;
    Ok(())
}

/// Stores what the agent reported about itself (and `last_seen`).
pub async fn set_system_info(db: &Db, id: u64, info: &pb::SystemInfo) -> Result<(), DbError> {
    let (hostname, os, version, lan) = clean_info(info);
    sqlx::query(
        "UPDATE computers SET hostname = ?, os = ?, agent_version = ?, lan_addresses = ?, last_seen = ? \
         WHERE id = ?",
    )
    .bind(hostname)
    .bind(os)
    .bind(version)
    .bind(lan)
    .bind(now_unix())
    .bind(id_i64(id))
    .execute(&db.write)
    .await?;
    Ok(())
}
