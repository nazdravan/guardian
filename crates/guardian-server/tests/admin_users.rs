#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::{
    CreateRoomRequest, CreateUserRequest, Empty, LoginRequest, Role, SetUserRoomsRequest,
    UpdateUserRequest,
};
use guardian_server::testing::{TestServer, req};
use tonic::Code;
use tonic::transport::Channel;

async fn clients(s: &TestServer) -> (AdminClient<Channel>, AuthClient<Channel>, String) {
    let ch = s.grpc().await;
    (
        AdminClient::new(ch.clone()),
        AuthClient::new(ch),
        s.admin_token().await,
    )
}

async fn login(auth: &mut AuthClient<Channel>, l: &str, p: &str) -> Result<String, tonic::Status> {
    auth.login(LoginRequest {
        login: l.into(),
        password: p.into(),
    })
    .await
    .map(|r| r.into_inner().token)
}

fn teacher(login: &str) -> CreateUserRequest {
    CreateUserRequest {
        login: login.into(),
        password: "teacher-password".into(),
        role: Role::Teacher as i32,
    }
}

#[tokio::test]
async fn create_teacher_and_login() {
    let s = TestServer::start().await;
    let (mut a, mut auth, t) = clients(&s).await;
    let u = a
        .create_user(req(&t, teacher("ion")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        (u.login.as_str(), u.role, u.active),
        ("ion", Role::Teacher as i32, true)
    );
    login(&mut auth, "ion", "teacher-password").await.unwrap();
    let users = a
        .list_users(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .users;
    assert_eq!(users.len(), 2);
    assert_eq!(
        a.create_user(req(&t, teacher("ion")))
            .await
            .unwrap_err()
            .code(),
        Code::AlreadyExists
    );
    assert_eq!(
        a.create_user(req(&t, teacher("Ion Popescu")))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let mut short = teacher("maria");
    short.password = "short".into();
    assert_eq!(
        a.create_user(req(&t, short)).await.unwrap_err().code(),
        Code::InvalidArgument
    );
    let mut norole = teacher("vasile");
    norole.role = Role::Unspecified as i32;
    assert_eq!(
        a.create_user(req(&t, norole)).await.unwrap_err().code(),
        Code::InvalidArgument
    );
}

#[tokio::test]
async fn disabling_kills_existing_sessions() {
    let s = TestServer::start().await;
    let (mut a, mut auth, t) = clients(&s).await;
    let u = a
        .create_user(req(&t, teacher("ion")))
        .await
        .unwrap()
        .into_inner();
    let tt = login(&mut auth, "ion", "teacher-password").await.unwrap();
    a.update_user(req(
        &t,
        UpdateUserRequest {
            id: u.id,
            active: Some(false),
            ..Default::default()
        },
    ))
    .await
    .unwrap();
    assert_eq!(
        auth.who_am_i(req(&tt, Empty {})).await.unwrap_err().code(),
        Code::Unauthenticated
    );
    assert_eq!(
        login(&mut auth, "ion", "teacher-password")
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    a.update_user(req(
        &t,
        UpdateUserRequest {
            id: u.id,
            active: Some(true),
            ..Default::default()
        },
    ))
    .await
    .unwrap();
    login(&mut auth, "ion", "teacher-password").await.unwrap();
}

#[tokio::test]
async fn password_reset_kills_old_sessions() {
    let s = TestServer::start().await;
    let (mut a, mut auth, t) = clients(&s).await;
    let u = a
        .create_user(req(&t, teacher("ion")))
        .await
        .unwrap()
        .into_inner();
    let tt = login(&mut auth, "ion", "teacher-password").await.unwrap();
    a.update_user(req(
        &t,
        UpdateUserRequest {
            id: u.id,
            new_password: Some("brand-new-pass".into()),
            ..Default::default()
        },
    ))
    .await
    .unwrap();
    assert_eq!(
        auth.who_am_i(req(&tt, Empty {})).await.unwrap_err().code(),
        Code::Unauthenticated
    );
    login(&mut auth, "ion", "brand-new-pass").await.unwrap();
}

#[tokio::test]
async fn last_admin_cannot_be_demoted_or_disabled() {
    let s = TestServer::start().await;
    let (mut a, _auth, t) = clients(&s).await;
    let me = AuthClient::new(s.grpc().await)
        .who_am_i(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner();
    let e = a
        .update_user(req(
            &t,
            UpdateUserRequest {
                id: me.id,
                role: Some(Role::Teacher as i32),
                ..Default::default()
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::FailedPrecondition);
    let e = a
        .update_user(req(
            &t,
            UpdateUserRequest {
                id: me.id,
                active: Some(false),
                ..Default::default()
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::FailedPrecondition);
    // With a second admin it is allowed.
    let mut second = teacher("dir");
    second.role = Role::Admin as i32;
    a.create_user(req(&t, second)).await.unwrap();
    a.update_user(req(
        &t,
        UpdateUserRequest {
            id: me.id,
            role: Some(Role::Teacher as i32),
            ..Default::default()
        },
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn set_user_rooms_is_atomic() {
    let s = TestServer::start().await;
    let (mut a, _auth, t) = clients(&s).await;
    let u = a
        .create_user(req(&t, teacher("ion")))
        .await
        .unwrap()
        .into_inner();
    let r1 = a
        .create_room(req(&t, CreateRoomRequest { name: "R1".into() }))
        .await
        .unwrap()
        .into_inner();
    let u2 = a
        .set_user_rooms(req(
            &t,
            SetUserRoomsRequest {
                user_id: u.id,
                room_ids: vec![r1.id],
            },
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(u2.room_ids, [r1.id]);
    let e = a
        .set_user_rooms(req(
            &t,
            SetUserRoomsRequest {
                user_id: u.id,
                room_ids: vec![999],
            },
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    let users = a
        .list_users(req(&t, Empty {}))
        .await
        .unwrap()
        .into_inner()
        .users;
    assert_eq!(
        users.iter().find(|x| x.id == u.id).unwrap().room_ids,
        [r1.id]
    );
}

#[tokio::test]
async fn who_am_i_shows_rooms_via_teacher_token() {
    let s = TestServer::start().await;
    let (mut a, mut auth, t) = clients(&s).await;
    let r1 = a
        .create_room(req(&t, CreateRoomRequest { name: "R1".into() }))
        .await
        .unwrap()
        .into_inner();
    let r2 = a
        .create_room(req(&t, CreateRoomRequest { name: "R2".into() }))
        .await
        .unwrap()
        .into_inner();
    let tt = s.teacher_token("ion", &[r1.id, r2.id]).await;
    let me = auth
        .who_am_i(req(&tt, Empty {}))
        .await
        .unwrap()
        .into_inner();
    assert_eq!((me.login.as_str(), me.role), ("ion", Role::Teacher as i32));
    let mut rooms = me.room_ids.clone();
    rooms.sort();
    assert_eq!(rooms, [r1.id, r2.id]);
}
