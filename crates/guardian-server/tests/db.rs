#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_proto::v1::ResultStatus;
use guardian_server::batch::{AuditEntry, AuditSource, Batcher};
use guardian_server::db::{Db, now_unix};

async fn fresh() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(&dir.path().join("g.db")).await.unwrap();
    (dir, db)
}

#[tokio::test]
async fn opens_fresh_db_with_wal_and_schema() {
    let (_d, db) = fresh().await;
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&db.read)
        .await
        .unwrap();
    assert_eq!(mode, "wal");
    let av: i64 = sqlx::query_scalar("PRAGMA auto_vacuum")
        .fetch_one(&db.read)
        .await
        .unwrap();
    assert_eq!(av, 2, "auto_vacuum=INCREMENTAL");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN \
         ('users','profiles','rooms','computers','commands','audit_log')",
    )
    .fetch_one(&db.read)
    .await
    .unwrap();
    assert_eq!(n, 6);
}

#[tokio::test]
async fn reopening_runs_no_new_migrations_and_keeps_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g.db");
    {
        let db = Db::open(&path).await.unwrap();
        sqlx::query(
            "INSERT INTO users(login,password_hash,role,created_at) VALUES('a','h','admin',1)",
        )
        .execute(&db.write)
        .await
        .unwrap();
        db.close().await;
    }
    let db = Db::open(&path).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&db.read)
        .await
        .unwrap();
    assert_eq!(n, 1);
    let m: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&db.read)
        .await
        .unwrap();
    assert_eq!(m, 1);
}

#[tokio::test]
async fn concurrent_writes_never_fail_busy() {
    let (_d, db) = fresh().await;
    let mut tasks = Vec::new();
    for i in 0..200 {
        let w = db.write.clone();
        tasks.push(tokio::spawn(async move {
            sqlx::query(
                "INSERT INTO audit_log(at,action,target,detail,source) VALUES(?,'x','t','d','server')",
            )
            .bind(i64::from(i))
            .execute(&w)
            .await
        }));
    }
    for t in tasks {
        t.await.unwrap().unwrap();
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log")
        .fetch_one(&db.read)
        .await
        .unwrap();
    assert_eq!(n, 200);
}

#[tokio::test]
async fn read_pool_is_read_only() {
    let (_d, db) = fresh().await;
    let r = sqlx::query(
        "INSERT INTO users(login,password_hash,role,created_at) VALUES('a','h','admin',1)",
    )
    .execute(&db.read)
    .await;
    assert!(r.is_err());
}

#[tokio::test]
async fn foreign_keys_are_enforced() {
    let (_d, db) = fresh().await;
    let r = sqlx::query("INSERT INTO rooms(name, active_profile_id) VALUES('r', 999)")
        .execute(&db.write)
        .await;
    assert!(r.is_err());
}

#[tokio::test]
async fn batcher_flushes_audit_in_batches() {
    let (_d, db) = fresh().await;
    let b = Batcher::spawn(db.clone());
    for i in 0..250 {
        b.audit(AuditEntry {
            actor_user_id: None,
            action: "test".into(),
            target: format!("t{i}"),
            detail: String::new(),
            source: AuditSource::Server,
        });
    }
    b.flush().await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log")
        .fetch_one(&db.read)
        .await
        .unwrap();
    assert_eq!(n, 250);
}

#[tokio::test]
async fn batcher_updates_command_results_but_not_final_ones() {
    let (_d, db) = fresh().await;
    let id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO commands(id,payload,created_at,expires_at) VALUES(?,x'00',?,?)")
        .bind(id.to_string())
        .bind(now_unix())
        .bind(now_unix() + 600)
        .execute(&db.write)
        .await
        .unwrap();
    for c in [1i64, 2] {
        sqlx::query(
            "INSERT INTO command_targets(command_id,computer_id,status,updated_at) VALUES(?,?,'pending',0)",
        )
        .bind(id.to_string())
        .bind(c)
        .execute(&db.write)
        .await
        .unwrap();
    }
    sqlx::query("UPDATE command_targets SET status='expired' WHERE computer_id=2")
        .execute(&db.write)
        .await
        .unwrap();
    let b = Batcher::spawn(db.clone());
    b.command_result(id, 1, ResultStatus::ResultOk, "done".into());
    b.command_result(id, 2, ResultStatus::ResultOk, "late".into());
    b.flush().await;
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT computer_id, status, detail FROM command_targets ORDER BY computer_id",
    )
    .fetch_all(&db.read)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            (1, "ok".into(), "done".into()),
            (2, "expired".into(), String::new())
        ]
    );
}
