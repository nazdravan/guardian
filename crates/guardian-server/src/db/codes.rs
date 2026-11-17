//! `enroll_codes`: one-time and room enrollment codes.

use guardian_core::codes::{generate_code, normalize_code};
use guardian_proto::v1 as pb;

use super::{Db, DbError, now_unix};

/// What a code enrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeKind {
    /// An agent (computer).
    Agent,
    /// A teacher console.
    Console,
}

impl CodeKind {
    fn name(self) -> &'static str {
        match self {
            CodeKind::Agent => "agent",
            CodeKind::Console => "console",
        }
    }
}

/// The single message for an unknown, used up or expired code.
pub const INVALID_CODE: &str = "invalid or expired enrollment code";

/// Why a code was not accepted.
#[derive(Debug, thiserror::Error)]
pub enum CodeError {
    /// Unknown, wrong kind, used up or expired — deliberately indistinguishable.
    #[error("invalid or expired enrollment code")]
    Invalid,
    /// Database failure.
    #[error(transparent)]
    Db(#[from] DbError),
}

/// Stores a new code (regenerating on collision).
pub async fn create(
    db: &Db,
    kind: CodeKind,
    room_id: Option<i64>,
    max_uses: u32,
    ttl_secs: u32,
    by: i64,
) -> Result<pb::EnrollCode, DbError> {
    let expires_at = now_unix() + i64::from(ttl_secs);
    let mut attempt = 0;
    loop {
        let code = generate_code();
        let r = sqlx::query(
            "INSERT INTO enroll_codes(code, kind, room_id, uses_left, max_uses, expires_at, created_by) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&code)
        .bind(kind.name())
        .bind(room_id)
        .bind(i64::from(max_uses))
        .bind(i64::from(max_uses))
        .bind(expires_at)
        .bind(by)
        .execute(&db.write)
        .await
        .map_err(DbError::from);
        match r {
            Ok(_) => {
                return Ok(pb::EnrollCode {
                    code,
                    expires_at_unix: expires_at,
                    max_uses,
                    kind: match kind {
                        CodeKind::Agent => pb::EnrollCodeKind::EnrollAgent,
                        CodeKind::Console => pb::EnrollCodeKind::EnrollConsole,
                    } as i32,
                    room_id: room_id.unwrap_or(0),
                });
            }
            Err(e) if e.is_unique_violation() && attempt < 10 => attempt += 1,
            Err(e) => return Err(e),
        }
    }
}

/// Atomically uses one use of a code and returns its room.
pub async fn consume(db: &Db, raw: &str, kind: CodeKind) -> Result<Option<i64>, CodeError> {
    let code = normalize_code(raw);
    let row: Option<(Option<i64>,)> = sqlx::query_as(
        "UPDATE enroll_codes SET uses_left = uses_left - 1 \
         WHERE code = ? AND kind = ? AND uses_left > 0 AND expires_at > ? RETURNING room_id",
    )
    .bind(&code)
    .bind(kind.name())
    .bind(now_unix())
    .fetch_optional(&db.write)
    .await
    .map_err(DbError::from)?;
    row.map(|(room,)| room).ok_or(CodeError::Invalid)
}
