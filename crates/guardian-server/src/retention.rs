//! Daily pruning of old rows.

use crate::db::{Db, DbError};

/// Rows deleted by one run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RetentionReport {
    /// Finished commands older than the retention period.
    pub commands: u64,
    /// Audit entries older than the retention period.
    pub audit: u64,
    /// Expired or used-up enrollment codes.
    pub codes: u64,
    /// Expired sessions.
    pub sessions: u64,
}

/// Deletes finished commands and audit entries older than `settings.retention_days`,
/// expired or used-up codes and expired sessions, then reclaims free pages.
/// Commands with a target still `pending`/`delivered` are kept.
pub async fn run_once(db: &Db, now: i64) -> Result<RetentionReport, DbError> {
    let days: i64 = sqlx::query_scalar("SELECT retention_days FROM settings WHERE id = 1")
        .fetch_optional(&db.read)
        .await?
        .unwrap_or(90);
    let cutoff = now - days * 86_400;
    let mut tx = db.write.begin().await?;
    let commands = sqlx::query(
        "DELETE FROM commands WHERE created_at < ? AND NOT EXISTS (SELECT 1 FROM command_targets t \
         WHERE t.command_id = commands.id AND t.status IN ('pending', 'delivered'))",
    )
    .bind(cutoff)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let audit = sqlx::query("DELETE FROM audit_log WHERE at < ?")
        .bind(cutoff)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let codes = sqlx::query("DELETE FROM enroll_codes WHERE expires_at <= ? OR uses_left <= 0")
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let sessions = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(now)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    sqlx::query("PRAGMA incremental_vacuum")
        .execute(&db.write)
        .await?;
    Ok(RetentionReport {
        commands,
        audit,
        codes,
        sessions,
    })
}
