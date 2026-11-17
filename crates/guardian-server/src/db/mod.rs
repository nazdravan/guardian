//! SQLite access: one writer connection, a separate read-only pool.

pub mod codes;
pub mod computers;
pub mod consoles;
pub mod profiles;
pub mod rooms;
pub mod sessions;
pub mod settings;
pub mod users;

use std::path::Path;
use std::time::Duration;

use guardian_proto::v1::ResultStatus;
use sqlx::sqlite::{
    SqliteAutoVacuum, SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions,
    SqliteSynchronous,
};

/// A database failure.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The query or connection failed.
    #[error("database: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// A migration failed.
    #[error("database migration: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

impl From<DbError> for tonic::Status {
    fn from(e: DbError) -> Self {
        tracing::error!("{e}");
        tonic::Status::internal("internal database error")
    }
}

impl DbError {
    /// `true` for a UNIQUE constraint violation.
    pub fn is_unique_violation(&self) -> bool {
        matches!(self, DbError::Sqlx(e) if e.as_database_error().is_some_and(|d| d.is_unique_violation()))
    }
}

/// The server database.
#[derive(Debug, Clone)]
pub struct Db {
    /// Read-only pool (4 connections).
    pub read: SqlitePool,
    /// The single writer (1 connection); every write goes through it.
    pub write: SqlitePool,
}

fn options(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_millis(5000))
        .foreign_keys(true)
}

impl Db {
    /// Opens (creating if needed) the database at `path` and runs the migrations.
    pub async fn open(path: &Path) -> Result<Db, DbError> {
        let write = SqlitePoolOptions::new()
            .max_connections(1)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(60))
            .connect_with(
                options(path)
                    .create_if_missing(true)
                    .auto_vacuum(SqliteAutoVacuum::Incremental),
            )
            .await?;
        sqlx::migrate!("./migrations").run(&write).await?;
        let read = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options(path).read_only(true))
            .await?;
        Ok(Db { read, write })
    }

    /// Closes both pools (waits for connections to be returned).
    pub async fn close(&self) {
        self.read.close().await;
        self.write.close().await;
    }
}

/// Seconds since the Unix epoch.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// The `command_targets.status` text for a result status.
pub fn status_name(s: ResultStatus) -> &'static str {
    match s {
        ResultStatus::Unspecified | ResultStatus::ResultPending => "pending",
        ResultStatus::ResultDelivered => "delivered",
        ResultStatus::ResultOk => "ok",
        ResultStatus::ResultFailed => "failed",
        ResultStatus::ResultNoUser => "no_user",
        ResultStatus::ResultUnsupported => "unsupported",
        ResultStatus::ResultExpired => "expired",
    }
}

/// Inverse of [`status_name`]; unknown text maps to `Unspecified`.
pub fn status_from_name(s: &str) -> ResultStatus {
    match s {
        "pending" => ResultStatus::ResultPending,
        "delivered" => ResultStatus::ResultDelivered,
        "ok" => ResultStatus::ResultOk,
        "failed" => ResultStatus::ResultFailed,
        "no_user" => ResultStatus::ResultNoUser,
        "unsupported" => ResultStatus::ResultUnsupported,
        "expired" => ResultStatus::ResultExpired,
        _ => ResultStatus::Unspecified,
    }
}

/// `true` for statuses after which a target never changes again.
pub fn is_final(s: ResultStatus) -> bool {
    !matches!(
        s,
        ResultStatus::Unspecified | ResultStatus::ResultPending | ResultStatus::ResultDelivered
    )
}
