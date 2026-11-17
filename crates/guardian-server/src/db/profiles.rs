//! `profiles` and `profile_apps`.

use super::{Db, DbError};

/// Names of the profiles created on first start.
pub const DEFAULT_FREE: &str = "Liber";
/// The school default.
pub const DEFAULT_LESSON: &str = "Lecție";
/// Exam profile.
pub const DEFAULT_EXAM: &str = "Examen";

/// Creates the three default profiles and the settings row, once.
pub async fn seed_defaults(db: &Db) -> Result<(), DbError> {
    let mut tx = db.write.begin().await?;
    let seeded: i64 = sqlx::query_scalar("SELECT count(*) FROM settings")
        .fetch_one(&mut *tx)
        .await?;
    if seeded > 0 {
        return Ok(());
    }
    let mut lesson = 0;
    for (name, mode) in [
        (DEFAULT_FREE, "free"),
        (DEFAULT_LESSON, "blacklist"),
        (DEFAULT_EXAM, "whitelist"),
    ] {
        let id = sqlx::query("INSERT INTO profiles(name, mode) VALUES (?, ?)")
            .bind(name)
            .bind(mode)
            .execute(&mut *tx)
            .await?
            .last_insert_rowid();
        if name == DEFAULT_LESSON {
            lesson = id;
        }
    }
    sqlx::query("INSERT INTO settings(id, default_profile_id) VALUES (1, ?)")
        .bind(lesson)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

use std::collections::BTreeSet;

use guardian_core::policy::{AppName, Mode, Profile, ProfileId};
use sqlx::{Row, SqliteConnection};

use super::settings::bump_state_version;

/// `profiles.mode` text.
pub fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Free => "free",
        Mode::Blacklist => "blacklist",
        Mode::Whitelist => "whitelist",
    }
}

fn mode_from(s: &str) -> Mode {
    match s {
        "free" => Mode::Free,
        "whitelist" => Mode::Whitelist,
        _ => Mode::Blacklist,
    }
}

/// Every profile with its apps, by id.
pub async fn list(db: &Db) -> Result<Vec<Profile>, DbError> {
    let rows = sqlx::query("SELECT id, name, mode FROM profiles ORDER BY id")
        .fetch_all(&db.read)
        .await?;
    let apps: Vec<(i64, String)> =
        sqlx::query_as("SELECT profile_id, app FROM profile_apps ORDER BY profile_id, app")
            .fetch_all(&db.read)
            .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let id: i64 = r.try_get("id")?;
        out.push(Profile {
            id: ProfileId(id),
            name: r.try_get("name")?,
            mode: mode_from(r.try_get::<&str, _>("mode")?),
            apps: apps
                .iter()
                .filter(|(p, _)| *p == id)
                .filter_map(|(_, a)| AppName::parse(a).ok())
                .collect(),
        });
    }
    Ok(out)
}

/// One profile.
pub async fn get(db: &Db, id: ProfileId) -> Result<Option<Profile>, DbError> {
    let Some(r) = sqlx::query("SELECT id, name, mode FROM profiles WHERE id = ?")
        .bind(id.0)
        .fetch_optional(&db.read)
        .await?
    else {
        return Ok(None);
    };
    let apps: Vec<String> =
        sqlx::query_scalar("SELECT app FROM profile_apps WHERE profile_id = ? ORDER BY app")
            .bind(id.0)
            .fetch_all(&db.read)
            .await?;
    Ok(Some(Profile {
        id,
        name: r.try_get("name")?,
        mode: mode_from(r.try_get::<&str, _>("mode")?),
        apps: apps.iter().filter_map(|a| AppName::parse(a).ok()).collect(),
    }))
}

/// `true` if a profile with `id` exists.
pub async fn exists(db: &Db, id: i64) -> Result<bool, DbError> {
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM profiles WHERE id = ?")
        .bind(id)
        .fetch_one(&db.read)
        .await?;
    Ok(n > 0)
}

async fn write_apps(
    conn: &mut SqliteConnection,
    id: i64,
    apps: &BTreeSet<AppName>,
) -> Result<(), DbError> {
    sqlx::query("DELETE FROM profile_apps WHERE profile_id = ?")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    for a in apps {
        sqlx::query("INSERT INTO profile_apps(profile_id, app) VALUES (?, ?)")
            .bind(id)
            .bind(a.as_str())
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Inserts a profile (its `id` is ignored) and returns it with the new id.
pub async fn create(db: &Db, p: &Profile) -> Result<Profile, DbError> {
    let mut tx = db.write.begin().await?;
    let id = sqlx::query("INSERT INTO profiles(name, mode) VALUES (?, ?)")
        .bind(&p.name)
        .bind(mode_name(p.mode))
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
    write_apps(&mut tx, id, &p.apps).await?;
    tx.commit().await?;
    Ok(Profile {
        id: ProfileId(id),
        ..p.clone()
    })
}

/// Replaces name, mode and apps; `false` if the profile does not exist.
pub async fn update(db: &Db, p: &Profile) -> Result<bool, DbError> {
    let mut tx = db.write.begin().await?;
    let n = sqlx::query("UPDATE profiles SET name = ?, mode = ? WHERE id = ?")
        .bind(&p.name)
        .bind(mode_name(p.mode))
        .bind(p.id.0)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n == 0 {
        return Ok(false);
    }
    write_apps(&mut tx, p.id.0, &p.apps).await?;
    bump_state_version(&mut tx).await?;
    tx.commit().await?;
    Ok(true)
}

/// Why a profile could not be deleted.
#[derive(Debug)]
pub enum DeleteProfile {
    /// Deleted.
    Deleted,
    /// No such profile.
    NotFound,
    /// Still referenced; the text says where.
    InUse(String),
}

/// Deletes a profile unless a room, a computer exception or the school default uses it.
pub async fn delete(db: &Db, id: ProfileId) -> Result<DeleteProfile, DbError> {
    let mut tx = db.write.begin().await?;
    let exists: i64 = sqlx::query_scalar("SELECT count(*) FROM profiles WHERE id = ?")
        .bind(id.0)
        .fetch_one(&mut *tx)
        .await?;
    if exists == 0 {
        return Ok(DeleteProfile::NotFound);
    }
    let mut uses = Vec::new();
    let rooms: Vec<String> =
        sqlx::query_scalar("SELECT name FROM rooms WHERE active_profile_id = ? ORDER BY name")
            .bind(id.0)
            .fetch_all(&mut *tx)
            .await?;
    for r in rooms {
        uses.push(format!("used by room \"{r}\""));
    }
    let exceptions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM computers WHERE exception_profile_id = ?")
            .bind(id.0)
            .fetch_one(&mut *tx)
            .await?;
    if exceptions > 0 {
        let noun = if exceptions == 1 {
            "computer"
        } else {
            "computers"
        };
        uses.push(format!("used as an exception by {exceptions} {noun}"));
    }
    let default: i64 = sqlx::query_scalar("SELECT default_profile_id FROM settings WHERE id = 1")
        .fetch_one(&mut *tx)
        .await?;
    if default == id.0 {
        uses.push("is the school default".into());
    }
    if !uses.is_empty() {
        return Ok(DeleteProfile::InUse(uses.join("; ")));
    }
    sqlx::query("DELETE FROM profiles WHERE id = ?")
        .bind(id.0)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(DeleteProfile::Deleted)
}
