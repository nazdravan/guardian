#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::{
    CreateRoomRequest, Empty, IdRequest, Mode, Profile, RenameRoomRequest, Settings,
};
use guardian_server::testing::{TestServer, req};
use tonic::Code;
use tonic::transport::Channel;

async fn admin(s: &TestServer) -> (AdminClient<Channel>, String) {
    (AdminClient::new(s.grpc().await), s.admin_token().await)
}

fn profile(name: &str, mode: Mode, apps: &[&str]) -> Profile {
    Profile {
        id: 0,
        name: name.into(),
        mode: mode as i32,
        apps: apps.iter().map(|a| a.to_string()).collect(),
    }
}

async fn profile_id(a: &mut AdminClient<Channel>, t: &str, name: &str) -> i64 {
    a.list_profiles(req(t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .profiles
        .into_iter()
        .find(|p| p.name == name)
        .unwrap()
        .id
}

#[tokio::test]
async fn seeded_profiles_and_default_are_listed() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let names: Vec<String> = a
        .list_profiles(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .profiles
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["Liber", "Lecție", "Examen"]);
    let st = a
        .get_settings(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        st.default_profile_id,
        profile_id(&mut a, &t, "Lecție").await
    );
    assert_eq!(st.retention_days, 90);
}

#[tokio::test]
async fn profile_crud_normalizes_apps() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let p = a
        .create_profile(req(
            &t,
            profile(
                "Desen",
                Mode::Whitelist,
                &[" Chrome.EXE ", "paint.exe", "chrome.exe"],
            ),
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(p.apps, ["chrome.exe", "paint.exe"]);
    let mut u = p.clone();
    u.name = "Desen 2".into();
    u.mode = Mode::Blacklist as i32;
    u.apps = vec!["VLC".into()];
    let u = a.update_profile(req(&t, u)).await.unwrap().into_inner();
    assert_eq!(
        (u.name.as_str(), u.mode, u.apps.clone()),
        ("Desen 2", Mode::Blacklist as i32, vec!["vlc".to_string()])
    );
    a.delete_profile(req(&t, IdRequest { id: p.id }))
        .await
        .unwrap();
    assert_eq!(
        a.list_profiles(req(&t, Empty {}))
            .await
            .unwrap()
            .into_inner()
            .profiles
            .len(),
        3
    );
}

#[tokio::test]
async fn bad_profiles_are_rejected() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let e = a
        .create_profile(req(&t, profile("Lecție", Mode::Free, &[])))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::AlreadyExists);
    let e = a
        .create_profile(req(&t, profile("X", Mode::Free, &["C:\\x"])))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    assert!(e.message().contains("C:\\x"), "{}", e.message());
    let e = a
        .create_profile(req(&t, profile("Y", Mode::Unspecified, &[])))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    let e = a
        .create_profile(req(&t, profile("", Mode::Free, &[])))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    let mut missing = profile("Z", Mode::Free, &[]);
    missing.id = 999;
    let e = a.update_profile(req(&t, missing)).await.unwrap_err();
    assert_eq!(e.code(), Code::NotFound);
}

#[tokio::test]
async fn delete_profile_in_use_is_refused() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    // 1. active in a room
    let p1 = a
        .create_profile(req(&t, profile("P1", Mode::Free, &[])))
        .await
        .unwrap()
        .into_inner();
    let room = a
        .create_room(req(
            &t,
            CreateRoomRequest {
                name: "Sala 12".into(),
            },
        ))
        .await
        .unwrap()
        .into_inner();
    sqlx::query("UPDATE rooms SET active_profile_id = ? WHERE id = ?")
        .bind(p1.id)
        .bind(room.id)
        .execute(&s.handle.state().db.write)
        .await
        .unwrap();
    let e = a
        .delete_profile(req(&t, IdRequest { id: p1.id }))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::FailedPrecondition);
    assert!(
        e.message().contains("used by room \"Sala 12\""),
        "{}",
        e.message()
    );
    // 2. exception on computers
    let p2 = a
        .create_profile(req(&t, profile("P2", Mode::Free, &[])))
        .await
        .unwrap()
        .into_inner();
    for k in [1u8, 2] {
        sqlx::query(
            "INSERT INTO computers(public_key, exception_profile_id, enrolled_at) VALUES (?, ?, 0)",
        )
        .bind(vec![k; 32])
        .bind(p2.id)
        .execute(&s.handle.state().db.write)
        .await
        .unwrap();
    }
    let e = a
        .delete_profile(req(&t, IdRequest { id: p2.id }))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::FailedPrecondition);
    assert!(
        e.message().contains("used as an exception by 2 computers"),
        "{}",
        e.message()
    );
    // 3. the school default
    let lesson = profile_id(&mut a, &t, "Lecție").await;
    sqlx::query("UPDATE rooms SET active_profile_id = ?")
        .bind(p1.id)
        .execute(&s.handle.state().db.write)
        .await
        .unwrap();
    let e = a
        .delete_profile(req(&t, IdRequest { id: lesson }))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::FailedPrecondition);
    assert!(
        e.message().contains("is the school default"),
        "{}",
        e.message()
    );
    // All three still exist.
    assert_eq!(
        a.list_profiles(req(&t, Empty {}))
            .await
            .unwrap()
            .into_inner()
            .profiles
            .len(),
        5
    );
}

#[tokio::test]
async fn rooms_start_with_the_default_profile() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let lesson = profile_id(&mut a, &t, "Lecție").await;
    let r = a
        .create_room(req(
            &t,
            CreateRoomRequest {
                name: " Sala 12 ".into(),
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!((r.name.as_str(), r.active_profile_id), ("Sala 12", lesson));
    let e = a
        .create_room(req(
            &t,
            CreateRoomRequest {
                name: "Sala 12".into(),
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::AlreadyExists);
    let r2 = a
        .rename_room(req(
            &t,
            RenameRoomRequest {
                id: r.id,
                name: "Sala 13".into(),
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(r2.name, "Sala 13");
    a.delete_room(req(&t, IdRequest { id: r.id }))
        .await
        .unwrap();
    assert!(
        a.list_rooms(req(&t, Empty {}))
            .await
            .unwrap()
            .into_inner()
            .rooms
            .is_empty()
    );
    let e = a
        .delete_room(req(&t, IdRequest { id: r.id }))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::NotFound);
}

#[tokio::test]
async fn deleting_a_room_leaves_its_computers_roomless() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let r = a
        .create_room(req(&t, CreateRoomRequest { name: "R".into() }))
        .await
        .unwrap()
        .into_inner();
    sqlx::query("INSERT INTO computers(public_key, room_id, enrolled_at) VALUES (x'01', ?, 0)")
        .bind(r.id)
        .execute(&s.handle.state().db.write)
        .await
        .unwrap();
    a.delete_room(req(&t, IdRequest { id: r.id }))
        .await
        .unwrap();
    let room: Option<i64> = sqlx::query_scalar("SELECT room_id FROM computers")
        .fetch_one(&s.handle.state().db.read)
        .await
        .unwrap();
    assert_eq!(room, None);
}

#[tokio::test]
async fn teacher_cannot_use_admin() {
    let s = TestServer::start().await;
    let (mut a, _t) = admin(&s).await;
    // Insert a teacher directly (user management is Task 9).
    let hash = guardian_server::auth::hash_password("teacher-pass").unwrap();
    sqlx::query(
        "INSERT INTO users(login,password_hash,role,created_at) VALUES('ion',?,'teacher',0)",
    )
    .bind(hash)
    .execute(&s.handle.state().db.write)
    .await
    .unwrap();
    let mut auth = guardian_proto::v1::auth_client::AuthClient::new(s.grpc().await);
    let tt = auth
        .login(guardian_proto::v1::LoginRequest {
            login: "ion".into(),
            password: "teacher-pass".into(),
        })
        .await
        .unwrap()
        .into_inner()
        .token;
    let e = a
        .create_room(req(&tt, CreateRoomRequest { name: "R".into() }))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::PermissionDenied);
    let e = a.list_profiles(req(&tt, Empty {})).await.unwrap_err();
    assert_eq!(e.code(), Code::PermissionDenied);
}

#[tokio::test]
async fn settings_are_validated() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    let e = a
        .update_settings(req(
            &t,
            Settings {
                default_profile_id: 999,
                retention_days: 30,
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    let free = profile_id(&mut a, &t, "Liber").await;
    let e = a
        .update_settings(req(
            &t,
            Settings {
                default_profile_id: free,
                retention_days: 0,
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    let st = a
        .update_settings(req(
            &t,
            Settings {
                default_profile_id: free,
                retention_days: 30,
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!((st.default_profile_id, st.retention_days), (free, 30));
}

#[tokio::test]
async fn changes_are_audited() {
    let s = TestServer::start().await;
    let (mut a, t) = admin(&s).await;
    a.create_room(req(&t, CreateRoomRequest { name: "R".into() }))
        .await
        .unwrap();
    s.handle.state().batcher.flush().await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'CreateRoom'")
        .fetch_one(&s.handle.state().db.read)
        .await
        .unwrap();
    assert_eq!(n, 1);
}
