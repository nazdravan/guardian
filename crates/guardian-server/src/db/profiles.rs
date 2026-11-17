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
