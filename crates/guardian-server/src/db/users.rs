//! `users` and `user_rooms`.

use std::collections::BTreeSet;

use guardian_proto::v1 as pb;
use guardian_proto::v1::Role;
use sqlx::{Row, SqliteConnection};

use super::{Db, DbError, now_unix};

/// A user row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRow {
    /// Id.
    pub id: i64,
    /// Login.
    pub login: String,
    /// argon2id PHC string.
    pub password_hash: String,
    /// Role.
    pub role: Role,
    /// Whether the user may log in.
    pub active: bool,
}

/// Role → `users.role` text.
pub fn role_name(r: Role) -> &'static str {
    match r {
        Role::Admin => "admin",
        Role::Teacher | Role::Unspecified => "teacher",
    }
}

fn role_from(s: &str) -> Role {
    if s == "admin" {
        Role::Admin
    } else {
        Role::Teacher
    }
}

fn row(r: &sqlx::sqlite::SqliteRow) -> Result<UserRow, sqlx::Error> {
    Ok(UserRow {
        id: r.try_get("id")?,
        login: r.try_get("login")?,
        password_hash: r.try_get("password_hash")?,
        role: role_from(r.try_get::<&str, _>("role")?),
        active: r.try_get::<i64, _>("active")? != 0,
    })
}

/// Number of users.
pub async fn count(conn: &mut SqliteConnection) -> Result<i64, DbError> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(conn)
        .await?)
}

/// Inserts a user and returns its id.
pub async fn insert(
    conn: &mut SqliteConnection,
    login: &str,
    password_hash: &str,
    role: Role,
) -> Result<i64, DbError> {
    let id = sqlx::query(
        "INSERT INTO users(login, password_hash, role, active, created_at) VALUES (?, ?, ?, 1, ?)",
    )
    .bind(login)
    .bind(password_hash)
    .bind(role_name(role))
    .bind(now_unix())
    .execute(conn)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// The user with `login`.
pub async fn by_login(db: &Db, login: &str) -> Result<Option<UserRow>, DbError> {
    let r = sqlx::query("SELECT id, login, password_hash, role, active FROM users WHERE login = ?")
        .bind(login)
        .fetch_optional(&db.read)
        .await?;
    Ok(r.as_ref().map(row).transpose()?)
}

/// The user with `id`.
pub async fn get(db: &Db, id: i64) -> Result<Option<UserRow>, DbError> {
    let r = sqlx::query("SELECT id, login, password_hash, role, active FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(&db.read)
        .await?;
    Ok(r.as_ref().map(row).transpose()?)
}

/// Every user, by id.
pub async fn list(db: &Db) -> Result<Vec<UserRow>, DbError> {
    let rows = sqlx::query("SELECT id, login, password_hash, role, active FROM users ORDER BY id")
        .fetch_all(&db.read)
        .await?;
    Ok(rows.iter().map(row).collect::<Result<_, _>>()?)
}

/// Rooms assigned to a user.
pub async fn rooms_of(db: &Db, user_id: i64) -> Result<BTreeSet<i64>, DbError> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT room_id FROM user_rooms WHERE user_id = ?")
        .bind(user_id)
        .fetch_all(&db.read)
        .await?;
    Ok(ids.into_iter().collect())
}

/// Replaces a user's password hash.
pub async fn set_password(db: &Db, user_id: i64, hash: &str) -> Result<(), DbError> {
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash)
        .bind(user_id)
        .execute(&db.write)
        .await?;
    Ok(())
}

/// The protobuf form.
pub fn to_proto(u: &UserRow, rooms: &BTreeSet<i64>) -> pb::User {
    pb::User {
        id: u.id,
        login: u.login.clone(),
        role: u.role as i32,
        active: u.active,
        room_ids: rooms.iter().copied().collect(),
    }
}

/// Why an update was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// Applied.
    Updated,
    /// No such user.
    NotFound,
    /// It would leave no active admin.
    LastAdmin,
}

/// Applies role / active / password changes in one transaction, refusing to remove
/// the last active admin. Disabling or a new password deletes all sessions.
pub async fn update(
    db: &Db,
    id: i64,
    role: Option<Role>,
    active: Option<bool>,
    password_hash: Option<&str>,
) -> Result<UpdateOutcome, DbError> {
    let mut tx = db.write.begin().await?;
    let cur: Option<(String, i64)> = sqlx::query_as("SELECT role, active FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((cur_role, cur_active)) = cur else {
        return Ok(UpdateOutcome::NotFound);
    };
    let was_admin = cur_role == "admin" && cur_active != 0;
    let new_role: String = role.map_or(cur_role.clone(), |r| role_name(r).to_string());
    let new_active = active.unwrap_or(cur_active != 0);
    let stays_admin = new_role == "admin" && new_active;
    if was_admin && !stays_admin {
        let admins: i64 =
            sqlx::query_scalar("SELECT count(*) FROM users WHERE role = 'admin' AND active = 1")
                .fetch_one(&mut *tx)
                .await?;
        if admins <= 1 {
            return Ok(UpdateOutcome::LastAdmin);
        }
    }
    sqlx::query("UPDATE users SET role = ?, active = ? WHERE id = ?")
        .bind(&new_role)
        .bind(i64::from(new_active))
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if let Some(h) = password_hash {
        sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
            .bind(h)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if !new_active || password_hash.is_some() {
        sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(UpdateOutcome::Updated)
}

/// Replaces a user's rooms; `Ok(false)` (and nothing changed) if a room does not exist.
pub async fn set_rooms(db: &Db, user_id: i64, rooms: &[i64]) -> Result<bool, DbError> {
    let mut tx = db.write.begin().await?;
    for r in rooms {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM rooms WHERE id = ?")
            .bind(r)
            .fetch_one(&mut *tx)
            .await?;
        if n == 0 {
            return Ok(false);
        }
    }
    sqlx::query("DELETE FROM user_rooms WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for r in rooms {
        sqlx::query("INSERT OR IGNORE INTO user_rooms(user_id, room_id) VALUES (?, ?)")
            .bind(user_id)
            .bind(r)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// All `(user_id, room_id)` assignments.
pub async fn all_rooms(db: &Db) -> Result<Vec<(i64, i64)>, DbError> {
    Ok(sqlx::query_as("SELECT user_id, room_id FROM user_rooms")
        .fetch_all(&db.read)
        .await?)
}
