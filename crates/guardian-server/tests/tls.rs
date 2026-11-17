#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::{CreateEnrollCodeRequest, EnrollCodeKind, InitRequest};
use guardian_server::testing::{SimAgent, TestServer, req};
use guardian_server::{ServerConfig, TlsMode};
use sha2::Digest;

#[tokio::test]
async fn self_signed_cert_is_stable_across_restarts() {
    let s = TestServer::start_tls().await;
    let pin = s.pin().unwrap();
    assert_eq!(pin.len(), 64);
    assert!(
        pin.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    assert!(s.dir.path().join("tls/cert.pem").exists());
    let s = s.restart().await;
    assert_eq!(s.pin().unwrap(), pin);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(s.dir.path().join("tls/key.pem"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[tokio::test]
async fn grpc_and_websocket_share_the_tls_port() {
    let s = TestServer::start_tls().await;
    assert!(s.url().starts_with("https://"));
    let ch = guardian_client::connect(&s.url(), s.pin().as_deref())
        .await
        .unwrap();
    let token = AuthClient::new(ch.clone())
        .init(InitRequest {
            bootstrap_code: s.handle.bootstrap_code().unwrap(),
            login: "admin".into(),
            password: "admin-password".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .token;
    let code = AdminClient::new(ch)
        .create_enroll_code(req(
            &token,
            CreateEnrollCodeRequest {
                kind: EnrollCodeKind::EnrollAgent as i32,
                ..Default::default()
            },
        ))
        .await
        .unwrap()
        .into_inner()
        .code;
    assert!(s.ws_url().starts_with("wss://"));
    let agent = SimAgent::enroll(&s, &code, &[]).await.unwrap();
    assert!(agent.id() > 0);
}

#[tokio::test]
async fn files_mode_serves_given_cert() {
    let dir = tempfile::tempdir().unwrap();
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["school.example".to_string()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let (cp, kp) = (dir.path().join("c.pem"), dir.path().join("k.pem"));
    std::fs::write(&cp, cert.pem()).unwrap();
    std::fs::write(&kp, key.serialize_pem()).unwrap();
    let expected = hex::encode(sha2::Sha256::digest(cert.der()));
    let s = TestServer::start_with(|c: &mut ServerConfig| {
        c.tls = TlsMode::Files {
            cert: cp.clone(),
            key: kp.clone(),
        }
    })
    .await;
    assert_eq!(s.pin().unwrap(), expected);
    AuthClient::new(s.grpc().await)
        .who_am_i(guardian_proto::v1::Empty {})
        .await
        .unwrap_err();
}

#[tokio::test]
async fn missing_cert_files_fail_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = ServerConfig::new(
        dir.path().to_path_buf(),
        TlsMode::Files {
            cert: dir.path().join("nope.pem"),
            key: dir.path().join("nope.key"),
        },
    );
    c.listen = "127.0.0.1:0".parse().unwrap();
    assert!(guardian_server::start(c).await.is_err());
}
