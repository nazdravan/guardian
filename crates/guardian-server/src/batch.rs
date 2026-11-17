//! Batched writes for high-volume rows (command results, audit entries).
//!
//! Items are queued in memory and written every 500 ms or every 100 items,
//! one transaction per batch, so a room of agents reporting at once costs one
//! fsync instead of one per row.

use std::time::Duration;

use guardian_proto::v1::ResultStatus;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::db::{Db, now_unix, status_name};

/// Where an audited action happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditSource {
    /// Through the server.
    Server,
    /// In local (offline) mode, reported later by a console.
    Local,
}

/// One audit log row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    /// The acting user, if any.
    pub actor_user_id: Option<i64>,
    /// What was done (e.g. `room.set_profile`).
    pub action: String,
    /// What it was done to (e.g. `room:3`).
    pub target: String,
    /// Free text.
    pub detail: String,
    /// Where it happened.
    pub source: AuditSource,
}

enum Item {
    Result {
        command_id: Uuid,
        computer_id: u64,
        status: ResultStatus,
        detail: String,
    },
    Audit(AuditEntry, i64),
    Flush(oneshot::Sender<()>),
}

const BATCH_MAX: usize = 100;
const BATCH_EVERY: Duration = Duration::from_millis(500);

/// Handle to the background batch writer. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Batcher {
    tx: mpsc::UnboundedSender<Item>,
}

impl Batcher {
    /// Starts the writer task on the current tokio runtime.
    pub fn spawn(db: Db) -> Batcher {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(db, rx));
        Batcher { tx }
    }

    /// Queues a command result. Final statuses are never overwritten.
    pub fn command_result(
        &self,
        command_id: Uuid,
        computer_id: u64,
        status: ResultStatus,
        detail: String,
    ) {
        let _ = self.tx.send(Item::Result {
            command_id,
            computer_id,
            status,
            detail,
        });
    }

    /// Queues an audit entry (timestamped now).
    pub fn audit(&self, entry: AuditEntry) {
        let _ = self.tx.send(Item::Audit(entry, now_unix()));
    }

    /// Waits until everything queued before this call is written.
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Item::Flush(tx)).is_ok() {
            let _ = rx.await;
        }
    }
}

async fn run(db: Db, mut rx: mpsc::UnboundedReceiver<Item>) {
    let mut buf: Vec<Item> = Vec::new();
    let mut waiters: Vec<oneshot::Sender<()>> = Vec::new();
    let mut tick = tokio::time::interval(BATCH_EVERY);
    loop {
        let closed = tokio::select! {
            item = rx.recv() => match item {
                Some(Item::Flush(w)) => { waiters.push(w); false }
                Some(item) => { buf.push(item); if buf.len() < BATCH_MAX { continue; } false }
                None => true,
            },
            _ = tick.tick() => false,
        };
        // Drain whatever is already queued so a flush covers it.
        while let Ok(item) = rx.try_recv() {
            match item {
                Item::Flush(w) => waiters.push(w),
                item => buf.push(item),
            }
        }
        while !buf.is_empty() {
            let n = buf.len().min(BATCH_MAX);
            let batch: Vec<Item> = buf.drain(..n).collect();
            if let Err(e) = write_batch(&db, batch).await {
                tracing::error!("batch write failed: {e}");
            }
        }
        for w in waiters.drain(..) {
            let _ = w.send(());
        }
        if closed {
            return;
        }
    }
}

async fn write_batch(db: &Db, batch: Vec<Item>) -> Result<(), sqlx::Error> {
    let mut tx = db.write.begin().await?;
    let now = now_unix();
    for item in batch {
        match item {
            Item::Result {
                command_id,
                computer_id,
                status,
                detail,
            } => {
                sqlx::query(
                    "UPDATE command_targets SET status = ?, detail = ?, updated_at = ? \
                     WHERE command_id = ? AND computer_id = ? AND status IN ('pending', 'delivered')",
                )
                .bind(status_name(status))
                .bind(detail)
                .bind(now)
                .bind(command_id.to_string())
                .bind(i64::try_from(computer_id).unwrap_or(i64::MAX))
                .execute(&mut *tx)
                .await?;
            }
            Item::Audit(e, at) => {
                sqlx::query(
                    "INSERT INTO audit_log(at, actor_user_id, action, target, detail, source) \
                     VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(at)
                .bind(e.actor_user_id)
                .bind(e.action)
                .bind(e.target)
                .bind(e.detail)
                .bind(match e.source {
                    AuditSource::Server => "server",
                    AuditSource::Local => "local",
                })
                .execute(&mut *tx)
                .await?;
            }
            Item::Flush(_) => {}
        }
    }
    tx.commit().await
}
