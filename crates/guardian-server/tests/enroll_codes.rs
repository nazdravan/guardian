#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_core::keys::KeyPair;
use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::{
    ComputerIdRequest, CreateEnrollCodeRequest, CreateRoomRequest, Empty, EnrollCodeKind,
    EnrollConsoleRequest, IdRequest, SystemInfo, UpdateComputerRequest,
};
use guardian_server::db::codes::{CodeError, CodeKind, consume};
use guardian_server::db::now_unix;
use guardian_server::testing::{TestServer, req};
use tonic::Code;
use tonic::transport::Channel;

async fn admin(s: &TestServer) -> (AdminClient<Channel>, String) {
    (AdminClient::new(s.grpc().await), s.admin_token().await)
}

fn agent_code(room_id: i64, max_uses: u32, ttl: u32) -> CreateEnrollCodeRequest {
    CreateEnrollCodeRequest {
        kind: EnrollCodeKind::EnrollAgent as i32,
        room_id,
        max_uses,
        ttl_seconds: ttl,
    }
}

fn console_code() -> CreateEnrollCodeRequest {
    CreateEnrollCodeRequest {
        kind: EnrollCodeKind::EnrollConsole as i32,
        ..Default::default()
    }
}

#[tokio::test]
async fn plain_agent_code_is_single_use_with_15_min_ttl() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let c = a
        .create_enroll_code(req(&t, agent_code(0, 50, 99999)))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(c.code.len(), 8);
    assert_eq!(c.max_uses, 1);
    assert!((c.expires_at_unix - now_unix() - 900).abs() <= 2);
    let db = &s.handle.state().db;
    assert_eq!(consume(db, &c.code, CodeKind::Agent).await.unwrap(), None);
    assert!(matches!(
        consume(db, &c.code, CodeKind::Agent).await,
        Err(CodeError::Invalid)
    ));
}

#[tokio::test]
async fn expired_wrong_kind_and_normalized_codes() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let db = &s.handle.state().db;
    let c = a
        .create_enroll_code(req(&t, agent_code(0, 1, 0)))
        .await
        .unwrap()
        .into_inner();
    sqlx::query("UPDATE enroll_codes SET expires_at = ? WHERE code = ?")
        .bind(now_unix() - 1)
        .bind(&c.code)
        .execute(&db.write)
        .await
        .unwrap();
    assert!(matches!(
        consume(db, &c.code, CodeKind::Agent).await,
        Err(CodeError::Invalid)
    ));
    let cc = a
        .create_enroll_code(req(&t, console_code()))
        .await
        .unwrap()
        .into_inner();
    assert!(matches!(
        consume(db, &cc.code, CodeKind::Agent).await,
        Err(CodeError::Invalid)
    ));
    let c2 = a
        .create_enroll_code(req(&t, agent_code(0, 1, 0)))
        .await
        .unwrap()
        .into_inner();
    let pretty = format!(" {}-{} ", &c2.code[..4], &c2.code[4..]).to_lowercase();
    assert_eq!(consume(db, &pretty, CodeKind::Agent).await.unwrap(), None);
}

#[tokio::test]
async fn room_code_rules() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let r = a
        .create_room(req(&t, CreateRoomRequest { name: "R".into() }))
        .await
        .unwrap()
        .into_inner();
    let e = a
        .create_enroll_code(req(&t, agent_code(r.id, 0, 0)))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument, "max_uses is required");
    let e = a
        .create_enroll_code(req(&t, agent_code(r.id, 501, 0)))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    let e = a
        .create_enroll_code(req(&t, agent_code(r.id, 5, 30)))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument, "ttl below 60 s");
    let e = a
        .create_enroll_code(req(&t, agent_code(999, 5, 0)))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument, "unknown room");
    let c = a
        .create_enroll_code(req(&t, agent_code(r.id, 5, 0)))
        .await
        .unwrap()
        .into_inner();
    assert_eq!((c.max_uses, c.room_id), (5, r.id));
    assert!((c.expires_at_unix - now_unix() - 86400).abs() <= 2);
}

#[tokio::test]
async fn room_code_allows_exactly_max_uses_in_parallel() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let r = a
        .create_room(req(&t, CreateRoomRequest { name: "R".into() }))
        .await
        .unwrap()
        .into_inner();
    let c = a
        .create_enroll_code(req(&t, agent_code(r.id, 5, 0)))
        .await
        .unwrap()
        .into_inner();
    let mut tasks = Vec::new();
    for _ in 0..30 {
        let db = s.handle.state().db.clone();
        let code = c.code.clone();
        tasks.push(tokio::spawn(async move {
            consume(&db, &code, CodeKind::Agent).await
        }));
    }
    let mut ok = 0;
    let mut invalid = 0;
    for t in tasks {
        match t.await.unwrap() {
            Ok(room) => {
                assert_eq!(room, Some(r.id));
                ok += 1;
            }
            Err(CodeError::Invalid) => invalid += 1,
            Err(e) => panic!("{e:?}"),
        }
    }
    assert_eq!((ok, invalid), (5, 25));
}

#[tokio::test]
async fn enroll_console_registers_a_console() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let mut auth = AuthClient::new(s.grpc().await);
    let key = KeyPair::generate();
    let enroll = |code: String, key: &KeyPair| EnrollConsoleRequest {
        code,
        public_key: key.public().as_bytes().to_vec(),
        name: "Laptop profesor".into(),
    };
    let e = auth
        .enroll_console(enroll("ABCD2345".into(), &key))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::PermissionDenied);
    assert_eq!(e.message(), "invalid or expired enrollment code");
    let c = a
        .create_enroll_code(req(&t, console_code()))
        .await
        .unwrap()
        .into_inner();
    let mut bad = enroll(c.code.clone(), &key);
    bad.public_key = vec![1, 2, 3];
    assert_eq!(
        auth.enroll_console(bad).await.unwrap_err().code(),
        Code::InvalidArgument
    );
    let r = auth
        .enroll_console(enroll(c.code, &key))
        .await
        .unwrap()
        .into_inner();
    let list = a
        .list_consoles(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .consoles;
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0].id, list[0].name.as_str()),
        (r.console_id, "Laptop profesor")
    );
    let c2 = a
        .create_enroll_code(req(&t, console_code()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        auth.enroll_console(enroll(c2.code, &key))
            .await
            .unwrap_err()
            .code(),
        Code::AlreadyExists
    );
    a.revoke_console(req(&t, IdRequest { id: r.console_id }))
        .await
        .unwrap();
    assert!(
        a.list_consoles(req(&t, Empty {}))
            .await
            .unwrap()
            .into_inner()
            .consoles
            .is_empty()
    );
}

#[tokio::test]
async fn update_and_revoke_computer() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let db = &s.handle.state().db;
    let info = SystemInfo {
        hostname: "pc-01".into(),
        os: "windows".into(),
        agent_version: "3.0.0".into(),
        lan_addresses: vec!["10.0.0.5".into()],
    };
    let id = guardian_server::db::computers::insert(db, &KeyPair::generate().public(), &info, None)
        .await
        .unwrap();
    let list = a
        .list_computers(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .computers;
    assert_eq!(list.len(), 1);
    assert_eq!(
        (
            list[0].hostname.as_str(),
            list[0].lan_addresses.clone(),
            list[0].online
        ),
        ("pc-01", vec!["10.0.0.5".to_string()], false)
    );
    let e = a
        .update_computer(req(
            &t,
            UpdateComputerRequest {
                id,
                room_id: Some(999),
                ..Default::default()
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    let e = a
        .update_computer(req(
            &t,
            UpdateComputerRequest {
                id,
                exception_profile_id: Some(999),
                ..Default::default()
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    let r = a
        .create_room(req(&t, CreateRoomRequest { name: "R".into() }))
        .await
        .unwrap()
        .into_inner();
    let c = a
        .update_computer(req(
            &t,
            UpdateComputerRequest {
                id,
                name: Some("PC 1".into()),
                room_id: Some(r.id),
                ..Default::default()
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (c.name.as_str(), c.room_id, c.effective_profile_id),
        ("PC 1", r.id, r.active_profile_id)
    );
    let c = a
        .update_computer(req(
            &t,
            UpdateComputerRequest {
                id,
                room_id: Some(0),
                ..Default::default()
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(c.room_id, 0);
    a.revoke_computer(req(&t, ComputerIdRequest { id }))
        .await
        .unwrap();
    assert!(
        a.list_computers(req(&t, Empty {}))
            .await
            .unwrap()
            .into_inner()
            .computers
            .is_empty()
    );
    assert_eq!(
        a.revoke_computer(req(&t, ComputerIdRequest { id }))
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
}
