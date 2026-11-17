#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_proto::v1::Empty;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_server::testing::TestServer;

#[tokio::test]
async fn right_pin_connects() {
    let s = TestServer::start_tls().await;
    let ch = guardian_client::connect(&s.url(), s.pin().as_deref())
        .await
        .unwrap();
    // Reaches the server: an unauthenticated call is answered, not a transport error.
    let e = AuthClient::new(ch).who_am_i(Empty {}).await.unwrap_err();
    assert_eq!(e.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn wrong_pin_is_refused() {
    let s = TestServer::start_tls().await;
    let wrong = "0".repeat(64);
    let r = guardian_client::connect(&s.url(), Some(&wrong)).await;
    match r {
        Err(_) => {}
        Ok(ch) => {
            let e = AuthClient::new(ch).who_am_i(Empty {}).await.unwrap_err();
            assert_eq!(e.code(), tonic::Code::Unavailable, "{e}");
        }
    }
}

#[tokio::test]
async fn no_pin_rejects_self_signed() {
    let s = TestServer::start_tls().await;
    let r = guardian_client::connect(&s.url(), None).await;
    match r {
        Err(_) => {}
        Ok(ch) => {
            let e = AuthClient::new(ch).who_am_i(Empty {}).await.unwrap_err();
            assert_eq!(e.code(), tonic::Code::Unavailable, "{e}");
        }
    }
}

#[tokio::test]
async fn malformed_pin_is_an_error() {
    assert!(guardian_client::rustls_config(Some("xyz")).is_err());
}

#[tokio::test]
async fn token_interceptor_authenticates() {
    let s = TestServer::start_tls().await;
    let token = s.admin_token().await;
    let ch = guardian_client::connect(&s.url(), s.pin().as_deref())
        .await
        .unwrap();
    let me = AuthClient::new(guardian_client::with_token(ch, &token).unwrap())
        .who_am_i(Empty {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(me.login, "admin");
}

#[tokio::test]
async fn plain_http_only_when_explicit() {
    let s = TestServer::start().await;
    assert!(s.url().starts_with("http://"));
    guardian_client::connect(&s.url(), None).await.unwrap();
}
