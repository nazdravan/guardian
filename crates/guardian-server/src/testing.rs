//! Test helpers (feature `testing`): an in-process server on a random port.

use std::time::Duration;

use guardian_proto::v1::admin_client::AdminClient;
use guardian_proto::v1::auth_client::AuthClient;
use guardian_proto::v1::{
    CreateUserRequest, Empty, InitRequest, LoginRequest, Role, SetUserRoomsRequest,
};
use tonic::transport::Channel;

use crate::{ServerConfig, ServerHandle, TlsMode, start};

/// Password of the admin created by [`TestServer::admin_token`].
pub const ADMIN_PASSWORD: &str = "admin-password";
/// Password of teachers created by [`TestServer::teacher_token`].
pub const TEACHER_PASSWORD: &str = "teacher-password";

/// A request carrying `authorization: Bearer <token>`.
pub fn req<T>(token: &str, msg: T) -> tonic::Request<T> {
    let mut r = tonic::Request::new(msg);
    if let Ok(v) = format!("Bearer {token}").parse() {
        r.metadata_mut().insert("authorization", v);
    }
    r
}

/// An in-process server on `127.0.0.1:0` with shortened timeouts.
#[derive(Debug)]
pub struct TestServer {
    /// The running server.
    pub handle: ServerHandle,
    /// Its data directory (deleted on drop).
    pub dir: tempfile::TempDir,
    config: ServerConfig,
}

#[allow(clippy::expect_used)]
impl TestServer {
    /// Plaintext server with test timeouts.
    pub async fn start() -> TestServer {
        TestServer::start_with(|_| {}).await
    }

    /// Like [`TestServer::start`], after `f` adjusts the config.
    pub async fn start_with(f: impl FnOnce(&mut ServerConfig)) -> TestServer {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = ServerConfig::new(dir.path().to_path_buf(), TlsMode::InsecurePlaintext);
        config.listen = "127.0.0.1:0".parse().expect("addr");
        config.handshake_timeout = Duration::from_secs(1);
        config.idle_timeout = Duration::from_secs(2);
        config.ping_interval = Duration::from_millis(500);
        config.expiry_interval = Duration::from_millis(100);
        config.auth_rate_per_minute = 10_000;
        f(&mut config);
        let handle = start(config.clone()).await.expect("server starts");
        TestServer {
            handle,
            dir,
            config,
        }
    }

    /// Stops the server and starts it again on the same data directory.
    pub async fn restart(self) -> TestServer {
        let TestServer {
            handle,
            dir,
            mut config,
        } = self;
        handle.shutdown().await;
        config.listen = "127.0.0.1:0".parse().expect("addr");
        let handle = start(config.clone()).await.expect("server restarts");
        TestServer {
            handle,
            dir,
            config,
        }
    }

    /// The config the server runs with.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    fn tls(&self) -> bool {
        self.handle.cert_sha256().is_some()
    }

    /// `http://127.0.0.1:port` (or `https://`).
    pub fn url(&self) -> String {
        let scheme = if self.tls() { "https" } else { "http" };
        format!("{scheme}://{}", self.handle.local_addr())
    }

    /// `ws://127.0.0.1:port/agent/ws` (or `wss://`).
    pub fn ws_url(&self) -> String {
        let scheme = if self.tls() { "wss" } else { "ws" };
        format!("{scheme}://{}/agent/ws", self.handle.local_addr())
    }

    /// The certificate pin, when TLS is on.
    pub fn pin(&self) -> Option<String> {
        self.handle.cert_sha256()
    }

    /// A gRPC channel to the server.
    pub async fn grpc(&self) -> Channel {
        Channel::from_shared(self.url())
            .expect("url")
            .connect()
            .await
            .expect("connect")
    }

    /// Initializes the server (first call) or logs in as `admin`, returning a token.
    pub async fn admin_token(&self) -> String {
        let mut a = AuthClient::new(self.grpc().await);
        if let Some(code) = self.handle.bootstrap_code() {
            return a
                .init(InitRequest {
                    bootstrap_code: code,
                    login: "admin".into(),
                    password: ADMIN_PASSWORD.into(),
                })
                .await
                .expect("init")
                .into_inner()
                .token;
        }
        a.login(LoginRequest {
            login: "admin".into(),
            password: ADMIN_PASSWORD.into(),
        })
        .await
        .expect("login")
        .into_inner()
        .token
    }

    /// Creates (or reuses) teacher `login` with password [`TEACHER_PASSWORD`], assigns
    /// exactly `rooms`, and returns a fresh session token.
    pub async fn teacher_token(&self, login: &str, rooms: &[i64]) -> String {
        let admin = self.admin_token().await;
        let mut a = AdminClient::new(self.grpc().await);
        let existing = a
            .list_users(req(&admin, Empty {}))
            .await
            .expect("list users")
            .into_inner()
            .users
            .into_iter()
            .find(|u| u.login == login);
        let id = match existing {
            Some(u) => u.id,
            None => {
                a.create_user(req(
                    &admin,
                    CreateUserRequest {
                        login: login.into(),
                        password: TEACHER_PASSWORD.into(),
                        role: Role::Teacher as i32,
                    },
                ))
                .await
                .expect("create teacher")
                .into_inner()
                .id
            }
        };
        a.set_user_rooms(req(
            &admin,
            SetUserRoomsRequest {
                user_id: id,
                room_ids: rooms.to_vec(),
            },
        ))
        .await
        .expect("set rooms");
        AuthClient::new(self.grpc().await)
            .login(LoginRequest {
                login: login.into(),
                password: TEACHER_PASSWORD.into(),
            })
            .await
            .expect("teacher login")
            .into_inner()
            .token
    }

    /// A read-only pool on the server's database file.
    pub async fn open_db_read_only(&self) -> sqlx::SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(self.dir.path().join("guardian.db"))
            .read_only(true);
        sqlx::SqlitePool::connect_with(opts).await.expect("open db")
    }
}
