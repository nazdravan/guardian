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

/// A [`TestServer`] that was stopped; [`StoppedServer::start`] brings it back on
/// the same port with the same data.
#[derive(Debug)]
pub struct StoppedServer {
    dir: tempfile::TempDir,
    config: ServerConfig,
}

impl StoppedServer {
    /// Starts it again on the same address. Retries the bind for 2 s.
    #[allow(clippy::panic)]
    pub async fn start(self) -> TestServer {
        let StoppedServer { dir, config } = self;
        let addr = config.listen;
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match start(config.clone()).await {
                Ok(handle) => {
                    return TestServer {
                        handle,
                        dir,
                        config,
                    };
                }
                Err(_) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => panic!("restart on {addr}: {e}"),
            }
        }
    }
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
    /// Server with a self-signed certificate (clients pin it).
    pub async fn start_tls() -> TestServer {
        TestServer::start_with(|c| c.tls = TlsMode::SelfSigned).await
    }

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

    /// Stops the server and starts it again on the same data directory **and port**
    /// (for clients that must reconnect to the same address). Retries the bind for 2 s.
    pub async fn restart_same_port(self) -> TestServer {
        self.stop().await.start().await
    }

    /// Stops the server, keeping its data directory, config and port.
    pub async fn stop(self) -> StoppedServer {
        let TestServer {
            handle,
            dir,
            mut config,
        } = self;
        config.listen = handle.local_addr();
        handle.shutdown().await;
        StoppedServer { dir, config }
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
        guardian_client::connect(&self.url(), self.pin().as_deref())
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

/// A simulated agent speaking the real WebSocket protocol.
pub struct SimAgent {
    id: u64,
    key: guardian_core::keys::KeyPair,
    welcome: guardian_proto::v1::Welcome,
    frames: tokio::sync::mpsc::UnboundedReceiver<guardian_proto::v1::ServerFrame>,
    out: tokio::sync::mpsc::UnboundedSender<sim::Out>,
    reader: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for SimAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimAgent").field("id", &self.id).finish()
    }
}

mod sim {
    use futures_util::{SinkExt, StreamExt};
    use guardian_core::feature::PROTOCOL_VERSION;
    use guardian_core::keys::{AuthContext, KeyPair};
    use guardian_proto::v1::{
        AgentFrame, ChallengeResponse, Enroll, Hello, ServerError, ServerFrame, SystemInfo,
        Welcome, agent_frame, server_frame,
    };
    use prost::Message as _;
    use tokio_tungstenite::tungstenite::Message;

    pub(super) enum Out {
        Frame(AgentFrame),
        Close,
        Silence,
    }

    pub(super) type Ws = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    pub(super) fn closed() -> ServerError {
        ServerError {
            code: "closed".into(),
            message: "connection closed".into(),
        }
    }

    pub(super) fn info() -> SystemInfo {
        SystemInfo {
            hostname: "sim-pc".into(),
            os: "linux".into(),
            agent_version: "3.0.0-sim".into(),
            lan_addresses: vec!["127.0.0.1".into()],
        }
    }

    pub(super) async fn send(ws: &mut Ws, f: agent_frame::Msg) -> Result<(), ServerError> {
        ws.send(Message::Binary(
            AgentFrame { msg: Some(f) }.encode_to_vec().into(),
        ))
        .await
        .map_err(|_| closed())
    }

    pub(super) async fn recv(ws: &mut Ws) -> Result<server_frame::Msg, ServerError> {
        loop {
            match ws.next().await {
                Some(Ok(Message::Binary(b))) => {
                    let f = ServerFrame::decode(b.as_ref()).map_err(|_| closed())?;
                    return match f.msg {
                        Some(server_frame::Msg::Error(e)) => Err(e),
                        Some(m) => Ok(m),
                        None => Err(closed()),
                    };
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return Err(closed()),
                Some(Ok(_)) => {}
            }
        }
    }

    pub(super) async fn hello(
        ws: &mut Ws,
        id: u64,
        key: &KeyPair,
        caps: &[&str],
        version: Option<u32>,
    ) -> Result<Welcome, ServerError> {
        send(
            ws,
            agent_frame::Msg::Hello(Hello {
                computer_id: id,
                protocol_version: version.unwrap_or(PROTOCOL_VERSION),
                capabilities: caps.iter().map(|c| c.to_string()).collect(),
                info: Some(info()),
            }),
        )
        .await?;
        let server_frame::Msg::Challenge(c) = recv(ws).await? else {
            return Err(closed());
        };
        let nonce: [u8; 32] = c.nonce.as_slice().try_into().map_err(|_| closed())?;
        let sig = key.sign_challenge(AuthContext::Agent, id, &nonce);
        send(
            ws,
            agent_frame::Msg::ChallengeResponse(ChallengeResponse {
                signature: sig.to_bytes().to_vec(),
            }),
        )
        .await?;
        match recv(ws).await? {
            server_frame::Msg::Welcome(w) => Ok(w),
            _ => Err(closed()),
        }
    }

    pub(super) async fn enroll(ws: &mut Ws, code: &str, key: &KeyPair) -> Result<u64, ServerError> {
        send(
            ws,
            agent_frame::Msg::Enroll(Enroll {
                code: code.into(),
                public_key: key.public().as_bytes().to_vec(),
                info: Some(info()),
            }),
        )
        .await?;
        match recv(ws).await? {
            server_frame::Msg::Enrolled(e) => Ok(e.computer_id),
            _ => Err(closed()),
        }
    }
}

#[allow(clippy::expect_used)]
impl SimAgent {
    async fn open(server: &TestServer) -> Result<sim::Ws, guardian_proto::v1::ServerError> {
        let connector = match server.pin() {
            Some(pin) => {
                Some(guardian_client::ws_connector(Some(&pin)).map_err(|_| sim::closed())?)
            }
            None => None,
        };
        tokio_tungstenite::connect_async_tls_with_config(server.ws_url(), None, false, connector)
            .await
            .map(|(ws, _)| ws)
            .map_err(|_| sim::closed())
    }

    fn spawn(
        ws: sim::Ws,
        id: u64,
        key: guardian_core::keys::KeyPair,
        welcome: guardian_proto::v1::Welcome,
    ) -> SimAgent {
        use futures_util::{SinkExt, StreamExt};
        use prost::Message as _;
        use tokio_tungstenite::tungstenite::Message;
        let (frames_tx, frames) = tokio::sync::mpsc::unbounded_channel();
        let (out, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<sim::Out>();
        let reader = tokio::spawn(async move {
            let (mut sink, mut stream) = ws.split();
            let mut silent = false;
            loop {
                tokio::select! {
                    m = stream.next(), if !silent => match m {
                        Some(Ok(Message::Binary(b))) => {
                            if let Ok(f) = guardian_proto::v1::ServerFrame::decode(b.as_ref()) {
                                let _ = frames_tx.send(f);
                            }
                        }
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                        Some(Ok(_)) => {}
                    },
                    o = out_rx.recv() => match o {
                        Some(sim::Out::Frame(f)) => {
                            let _ = sink.send(Message::Binary(f.encode_to_vec().into())).await;
                        }
                        Some(sim::Out::Silence) => silent = true,
                        Some(sim::Out::Close) | None => {
                            let _ = sink.close().await;
                            return;
                        }
                    },
                }
            }
        });
        SimAgent {
            id,
            key,
            welcome,
            frames,
            out,
            reader,
        }
    }

    /// Enrolls with `code` and a fresh key, then completes the handshake.
    pub async fn enroll(
        server: &TestServer,
        code: &str,
        caps: &[&str],
    ) -> Result<SimAgent, guardian_proto::v1::ServerError> {
        SimAgent::enroll_with_key(
            server,
            code,
            &guardian_core::keys::KeyPair::generate(),
            caps,
        )
        .await
    }

    /// Enrolls with a given key.
    pub async fn enroll_with_key(
        server: &TestServer,
        code: &str,
        key: &guardian_core::keys::KeyPair,
        caps: &[&str],
    ) -> Result<SimAgent, guardian_proto::v1::ServerError> {
        let mut ws = SimAgent::open(server).await?;
        let id = sim::enroll(&mut ws, code, key).await?;
        let welcome = sim::hello(&mut ws, id, key, caps, None).await?;
        let key = guardian_core::keys::KeyPair::from_seed(&key.seed());
        Ok(SimAgent::spawn(ws, id, key, welcome))
    }

    /// Reconnects an enrolled computer.
    pub async fn connect(
        server: &TestServer,
        id: u64,
        key: &guardian_core::keys::KeyPair,
        caps: &[&str],
    ) -> Result<SimAgent, guardian_proto::v1::ServerError> {
        SimAgent::connect_inner(server, id, key, caps, None).await
    }

    /// Reconnects announcing another protocol version.
    pub async fn connect_with_version(
        server: &TestServer,
        id: u64,
        key: &guardian_core::keys::KeyPair,
        caps: &[&str],
        version: u32,
    ) -> Result<SimAgent, guardian_proto::v1::ServerError> {
        SimAgent::connect_inner(server, id, key, caps, Some(version)).await
    }

    async fn connect_inner(
        server: &TestServer,
        id: u64,
        key: &guardian_core::keys::KeyPair,
        caps: &[&str],
        version: Option<u32>,
    ) -> Result<SimAgent, guardian_proto::v1::ServerError> {
        let mut ws = SimAgent::open(server).await?;
        let welcome = sim::hello(&mut ws, id, key, caps, version).await?;
        let key = guardian_core::keys::KeyPair::from_seed(&key.seed());
        Ok(SimAgent::spawn(ws, id, key, welcome))
    }

    /// The `Welcome` the server sent.
    pub fn welcome(&self) -> &guardian_proto::v1::Welcome {
        &self.welcome
    }

    /// The next frame, or `None` on timeout or close.
    pub async fn next_frame(
        &mut self,
        timeout: Duration,
    ) -> Option<guardian_proto::v1::ServerFrame> {
        tokio::time::timeout(timeout, self.frames.recv())
            .await
            .ok()
            .flatten()
    }

    /// Waits until the server closes the connection, discarding frames.
    pub async fn wait_closed(&mut self, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, async {
            while self.frames.recv().await.is_some() {}
        })
        .await
        .is_ok()
    }

    /// Reports a command result.
    pub async fn send_result(&mut self, r: guardian_proto::v1::CommandResult) {
        let _ = self
            .out
            .send(sim::Out::Frame(guardian_proto::v1::AgentFrame {
                msg: Some(guardian_proto::v1::agent_frame::Msg::CommandResult(r)),
            }));
    }

    /// Sends the local fallback journal.
    pub async fn send_local_audit(&mut self, entries: Vec<guardian_proto::v1::LocalAuditEntry>) {
        let _ = self
            .out
            .send(sim::Out::Frame(guardian_proto::v1::AgentFrame {
                msg: Some(guardian_proto::v1::agent_frame::Msg::LocalAudit(
                    guardian_proto::v1::LocalAudit { entries },
                )),
            }));
    }

    /// Stops answering pings (to test the idle timeout).
    pub fn go_silent(&self) {
        let _ = self.out.send(sim::Out::Silence);
    }

    /// The computer id.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The agent's key.
    pub fn key(&self) -> &guardian_core::keys::KeyPair {
        &self.key
    }

    /// Closes the connection cleanly.
    pub async fn close(self) {
        let _ = self.out.send(sim::Out::Close);
        let _ = tokio::time::timeout(Duration::from_secs(2), self.reader).await;
    }
}
