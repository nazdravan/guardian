//! `commands` and `command_targets`.

use guardian_proto::v1 as pb;
use guardian_proto::v1::{CommandEnvelope, ResultStatus};
use prost::Message as _;
use uuid::Uuid;

use super::{Db, DbError, now_unix, status_from_name, status_name};

fn id_i64(id: u64) -> i64 {
    i64::try_from(id).unwrap_or(i64::MAX)
}

/// Stores a command and its targets with their initial statuses.
pub async fn create(
    db: &Db,
    env: &CommandEnvelope,
    by: i64,
    targets: &[(u64, ResultStatus)],
) -> Result<(), DbError> {
    let now = now_unix();
    let mut tx = db.write.begin().await?;
    sqlx::query(
        "INSERT INTO commands(id, payload, created_by, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&env.id)
    .bind(env.encode_to_vec())
    .bind(by)
    .bind(now)
    .bind(env.expires_at_unix)
    .execute(&mut *tx)
    .await?;
    for (computer, status) in targets {
        sqlx::query(
            "INSERT INTO command_targets(command_id, computer_id, status, updated_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&env.id)
        .bind(id_i64(*computer))
        .bind(status_name(*status))
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Unexpired commands still `pending` for a computer, oldest first.
pub async fn pending_for(
    db: &Db,
    computer_id: u64,
    now: i64,
) -> Result<Vec<CommandEnvelope>, DbError> {
    let payloads: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT c.payload FROM command_targets t JOIN commands c ON c.id = t.command_id \
         WHERE t.computer_id = ? AND t.status = 'pending' AND c.expires_at > ? ORDER BY c.created_at, c.id",
    )
    .bind(id_i64(computer_id))
    .bind(now)
    .fetch_all(&db.read)
    .await?;
    Ok(payloads
        .iter()
        .filter_map(|p| CommandEnvelope::decode(p.as_slice()).ok())
        .collect())
}

/// Moves a target from `from` to `to`; `false` if it was not in `from`.
pub async fn transition(
    db: &Db,
    command_id: &str,
    computer_id: u64,
    from: ResultStatus,
    to: ResultStatus,
) -> Result<bool, DbError> {
    let n = sqlx::query(
        "UPDATE command_targets SET status = ?, updated_at = ? \
         WHERE command_id = ? AND computer_id = ? AND status = ?",
    )
    .bind(status_name(to))
    .bind(now_unix())
    .bind(command_id)
    .bind(id_i64(computer_id))
    .bind(status_name(from))
    .execute(&db.write)
    .await?
    .rows_affected();
    Ok(n > 0)
}

/// Whether `computer_id` is a target of `command_id`.
pub async fn is_target(db: &Db, command_id: &str, computer_id: u64) -> Result<bool, DbError> {
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM command_targets WHERE command_id = ? AND computer_id = ?",
    )
    .bind(command_id)
    .bind(id_i64(computer_id))
    .fetch_one(&db.read)
    .await?;
    Ok(n > 0)
}

/// A command with every target's status, plus its creator.
pub async fn status(
    db: &Db,
    id: Uuid,
) -> Result<Option<(pb::CommandStatus, Option<i64>)>, DbError> {
    let row: Option<(Vec<u8>, Option<i64>, i64, i64)> = sqlx::query_as(
        "SELECT payload, created_by, created_at, expires_at FROM commands WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(&db.read)
    .await?;
    let Some((payload, by, created_at, expires_at)) = row else {
        return Ok(None);
    };
    let env = CommandEnvelope::decode(payload.as_slice()).unwrap_or_default();
    let targets: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT computer_id, status, detail FROM command_targets WHERE command_id = ? ORDER BY computer_id",
    )
    .bind(id.to_string())
    .fetch_all(&db.read)
    .await?;
    Ok(Some((
        pb::CommandStatus {
            command_id: id.to_string(),
            command: env.command,
            created_at_unix: created_at,
            expires_at_unix: expires_at,
            results: targets
                .into_iter()
                .map(|(c, s, d)| pb::ComputerResult {
                    computer_id: u64::try_from(c).unwrap_or(0),
                    status: status_from_name(&s) as i32,
                    detail: d,
                })
                .collect(),
        },
        by,
    )))
}

/// Marks unfinished targets of expired commands `expired`; returns them.
pub async fn expire_due(db: &Db, now: i64) -> Result<Vec<(Uuid, u64)>, DbError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "UPDATE command_targets SET status = 'expired', updated_at = ? \
         WHERE status IN ('pending', 'delivered') \
         AND command_id IN (SELECT id FROM commands WHERE expires_at <= ?) \
         RETURNING command_id, computer_id",
    )
    .bind(now)
    .bind(now)
    .fetch_all(&db.write)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(c, id)| Some((Uuid::parse_str(&c).ok()?, u64::try_from(id).ok()?)))
        .collect())
}
