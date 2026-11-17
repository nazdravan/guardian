#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::{ChangePasswordRequest, Empty, InitRequest, LoginRequest};
use guardian_server::ServerConfig;
use guardian_server::testing::{TestServer, req};
use tonic::Code;

async fn auth(s: &TestServer) -> AuthClient<tonic::transport::Channel> {
    AuthClient::new(s.grpc().await)
}

fn init(code: &str) -> InitRequest {
    InitRequest {
        bootstrap_code: code.into(),
        login: "admin".into(),
        password: "admin-password".into(),
    }
}

#[tokio::test]
async fn fresh_server_has_bootstrap_code_and_seeded_profiles() {
    let s = TestServer::start().await;
    assert_eq!(s.handle.bootstrap_code().map(|c| c.len()), Some(8));
    let db = s.open_db_read_only().await;
    let names: Vec<(String, String)> =
        sqlx::query_as("SELECT name, mode FROM profiles ORDER BY id")
            .fetch_all(&db)
            .await
            .unwrap();
    assert_eq!(
        names,
        vec![
            ("Liber".into(), "free".into()),
            ("Lecție".into(), "blacklist".into()),
            ("Examen".into(), "whitelist".into())
        ]
    );
    let default: String = sqlx::query_scalar(
        "SELECT p.name FROM settings s JOIN profiles p ON p.id = s.default_profile_id",
    )
    .fetch_one(&db)
    .await
    .unwrap();
    assert_eq!(default, "Lecție");
}

#[tokio::test]
async fn init_works_once_with_the_right_code() {
    let s = TestServer::start().await;
    let code = s.handle.bootstrap_code().unwrap();
    let mut a = auth(&s).await;
    let e = a.init(init("WRONG234")).await.unwrap_err();
    assert_eq!(e.code(), Code::PermissionDenied);
    // Lowercase with a dash still matches.
    let pretty = format!("{}-{}", &code[..4], &code[4..]).to_lowercase();
    let r = a.init(init(&pretty)).await.unwrap().into_inner();
    assert!(!r.token.is_empty());
    assert_eq!(r.user.unwrap().login, "admin");
    let e = a.init(init(&code)).await.unwrap_err();
    assert_eq!(e.code(), Code::FailedPrecondition);
    assert_eq!(s.handle.bootstrap_code(), None);
}

#[tokio::test]
async fn login_errors_do_not_reveal_which_part_was_wrong() {
    let s = TestServer::start().await;
    s.admin_token().await;
    let mut a = auth(&s).await;
    let e1 = a
        .login(LoginRequest {
            login: "nobody".into(),
            password: "admin-password".into(),
        })
        .await
        .unwrap_err();
    let e2 = a
        .login(LoginRequest {
            login: "admin".into(),
            password: "wrong-password".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(e1.code(), Code::Unauthenticated);
    assert_eq!(e1.code(), e2.code());
    assert_eq!(e1.message(), e2.message());
    let ok = a
        .login(LoginRequest {
            login: "admin".into(),
            password: "admin-password".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(ok.expires_at_unix > 0);
}

#[tokio::test]
async fn calls_without_token_are_rejected() {
    let s = TestServer::start().await;
    s.admin_token().await;
    let mut a = auth(&s).await;
    assert_eq!(
        a.who_am_i(Empty {}).await.unwrap_err().code(),
        Code::Unauthenticated
    );
    assert_eq!(
        a.who_am_i(req("bogus", Empty {})).await.unwrap_err().code(),
        Code::Unauthenticated
    );
}

#[tokio::test]
async fn change_password_kills_other_sessions() {
    let s = TestServer::start().await;
    let t1 = s.admin_token().await;
    let mut a = auth(&s).await;
    let t2 = a
        .login(LoginRequest {
            login: "admin".into(),
            password: "admin-password".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .token;
    let e = a
        .change_password(req(
            &t1,
            ChangePasswordRequest {
                old_password: "nope-nope".into(),
                new_password: "new-password".into(),
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::PermissionDenied);
    a.change_password(req(
        &t1,
        ChangePasswordRequest {
            old_password: "admin-password".into(),
            new_password: "new-password".into(),
        },
    ))
    .await
    .unwrap();
    assert_eq!(
        a.who_am_i(req(&t2, Empty {})).await.unwrap_err().code(),
        Code::Unauthenticated
    );
    assert_eq!(
        a.who_am_i(req(&t1, Empty {}))
            .await
            .unwrap()
            .into_inner()
            .login,
        "admin"
    );
}

#[tokio::test]
async fn logout_ends_the_session() {
    let s = TestServer::start().await;
    let t = s.admin_token().await;
    let mut a = auth(&s).await;
    a.logout(req(&t, Empty {})).await.unwrap();
    assert_eq!(
        a.who_am_i(req(&t, Empty {})).await.unwrap_err().code(),
        Code::Unauthenticated
    );
}

#[tokio::test]
async fn login_is_rate_limited_per_ip() {
    let s = TestServer::start_with(|c: &mut ServerConfig| c.auth_rate_per_minute = 10).await;
    let mut a = auth(&s).await;
    for _ in 0..10 {
        let e = a
            .login(LoginRequest {
                login: "x".into(),
                password: "y".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(e.code(), Code::Unauthenticated);
    }
    let e = a
        .login(LoginRequest {
            login: "x".into(),
            password: "y".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::ResourceExhausted);
}

#[tokio::test]
async fn restart_keeps_users_and_does_not_reseed() {
    let s = TestServer::start().await;
    s.admin_token().await;
    let s = s.restart().await;
    assert_eq!(s.handle.bootstrap_code(), None);
    let db = s.open_db_read_only().await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM profiles")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(n, 3);
    let mut a = auth(&s).await;
    a.login(LoginRequest {
        login: "admin".into(),
        password: "admin-password".into(),
    })
    .await
    .unwrap();
}
