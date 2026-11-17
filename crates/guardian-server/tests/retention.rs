#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_server::db::{Db, now_unix};
use guardian_server::retention::{RetentionReport, run_once};

const DAY: i64 = 86_400;

async fn fresh() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("g.db")).await.unwrap();
    guardian_server::db::profiles::seed_defaults(&db)
        .await
        .unwrap();
    (dir, db)
}

async fn exec(db: &Db, sql: &str, binds: &[i64]) {
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.to_string()));
    for b in binds {
        q = q.bind(*b);
    }
    q.execute(&db.write).await.unwrap();
}

async fn count(db: &Db, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(&db.read)
        .await
        .unwrap()
}

async fn command(db: &Db, id: &str, created_at: i64, expires_at: i64, status: &str) {
    sqlx::query(
        "INSERT INTO commands(id, payload, created_at, expires_at) VALUES (?, x'00', ?, ?)",
    )
    .bind(id)
    .bind(created_at)
    .bind(expires_at)
    .execute(&db.write)
    .await
    .unwrap();
    sqlx::query("INSERT INTO command_targets(command_id, computer_id, status, updated_at) VALUES (?, 1, ?, 0)")
        .bind(id)
        .bind(status)
        .execute(&db.write)
        .await
        .unwrap();
}

#[tokio::test]
async fn old_rows_go_recent_rows_stay() {
    let (_d, db) = fresh().await;
    let now = now_unix();
    exec(&db, "UPDATE settings SET retention_days = 1", &[]).await;
    command(&db, "old-done", now - 3 * DAY, now - 3 * DAY + 600, "ok").await;
    command(&db, "old-open", now - 3 * DAY, now + 600, "pending").await;
    command(&db, "new-done", now - 60, now + 600, "ok").await;
    exec(&db, "INSERT INTO audit_log(at, action, target, detail, source) VALUES (?, 'a', 't', '', 'server')", &[now - 3 * DAY]).await;
    exec(&db, "INSERT INTO audit_log(at, action, target, detail, source) VALUES (?, 'a', 't', '', 'server')", &[now - 60]).await;
    exec(&db, "INSERT INTO enroll_codes(code, kind, uses_left, max_uses, expires_at) VALUES ('EXPIRED2', 'agent', 1, 1, ?)", &[now - 1]).await;
    exec(&db, "INSERT INTO enroll_codes(code, kind, uses_left, max_uses, expires_at) VALUES ('USEDUP22', 'agent', 0, 5, ?)", &[now + DAY]).await;
    exec(&db, "INSERT INTO enroll_codes(code, kind, uses_left, max_uses, expires_at) VALUES ('LIVE2345', 'agent', 1, 1, ?)", &[now + DAY]).await;
    exec(&db, "INSERT INTO users(id, login, password_hash, role, created_at) VALUES (1, 'a', 'h', 'admin', 0)", &[]).await;
    exec(
        &db,
        "INSERT INTO sessions(token_hash, user_id, expires_at) VALUES ('old', 1, ?)",
        &[now - 1],
    )
    .await;
    exec(
        &db,
        "INSERT INTO sessions(token_hash, user_id, expires_at) VALUES ('new', 1, ?)",
        &[now + DAY],
    )
    .await;

    let r = run_once(&db, now).await.unwrap();
    assert_eq!(
        r,
        RetentionReport {
            commands: 1,
            audit: 1,
            codes: 2,
            sessions: 1
        }
    );
    let left: Vec<String> = sqlx::query_scalar("SELECT id FROM commands ORDER BY id")
        .fetch_all(&db.read)
        .await
        .unwrap();
    assert_eq!(left, ["new-done", "old-open"]);
    assert_eq!(
        count(&db, "command_targets").await,
        2,
        "targets cascade with their command"
    );
    assert_eq!(count(&db, "audit_log").await, 1);
    assert_eq!(count(&db, "enroll_codes").await, 1);
    assert_eq!(count(&db, "sessions").await, 1);
}

#[tokio::test]
async fn default_retention_is_90_days() {
    let (_d, db) = fresh().await;
    let now = now_unix();
    command(&db, "c89", now - 89 * DAY, now - 89 * DAY + 600, "expired").await;
    command(&db, "c91", now - 91 * DAY, now - 91 * DAY + 600, "expired").await;
    let r = run_once(&db, now).await.unwrap();
    assert_eq!(r.commands, 1);
    let left: Vec<String> = sqlx::query_scalar("SELECT id FROM commands")
        .fetch_all(&db.read)
        .await
        .unwrap();
    assert_eq!(left, ["c89"]);
}
