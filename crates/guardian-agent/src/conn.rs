//! The WebSocket connection to the server: enrollment, the challenge handshake,
//! and a reconnect loop with exponential backoff.
//!
//! This module only transports frames. Which state to apply, command dedupe and
//! answers belong to [`crate::Agent`].

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use guardian_core::feature::PROTOCOL_VERSION;
use guardian_core::keys::{AuthContext, KeyPair};
use guardian_core::random::random_bytes;
use guardian_proto::v1::{
    AgentFrame, AgentState, ChallengeResponse, CommandEnvelope, CommandResult, Enroll, Hello,
    ServerFrame, SystemInfo, Welcome, agent_frame, server_frame,
};
use prost::Message as _;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The handshake must finish within this.
const HANDSHAKE: Duration = Duration::from_secs(10);
/// Our own ping interval.
const PING_EVERY: Duration = Duration::from_secs(20);
/// Reconnect when nothing arrives for this long.
const IDLE: Duration = Duration::from_secs(60);
/// Largest frame accepted (matches the server).
const MAX_MESSAGE: usize = 1024 * 1024;

/// A connection attempt failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnError {
    /// The server answered with `ServerError`.
    #[error("server refused ({code}): {message}")]
    Rejected {
        /// `invalid_code`, `unauthorized`, `revoked`, `protocol`, `rate_limited`.
        code: String,
        /// Human text.
        message: String,
    },
    /// Network / TLS failure.
    #[error("connection failed: {0}")]
    Io(String),
    /// Unexpected frame or closed stream.
    #[error("protocol error: {0}")]
    Protocol(String),
}

/// What the connection reports to the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnEvent {
    /// Authenticated; the server's full `Welcome`.
    Online {
        /// State, pending commands, trust.
        welcome: Welcome,
    },
    /// A pushed state.
    State(AgentState),
    /// New trusted consoles / users (only those two fields are set).
    Trust(Welcome),
    /// A command.
    Command(CommandEnvelope),
    /// The connection was lost (a reconnect follows).
    Offline {
        /// Why.
        reason: String,
    },
    /// The computer was revoked; the loop has stopped for good.
    Revoked,
}

/// Where and how to connect.
#[derive(Debug, Clone)]
pub struct ConnConfig {
    /// `wss://host:port` (the `/agent/ws` path is added).
    pub server: String,
    /// Pinned certificate SHA-256.
    pub pin: Option<String>,
    /// From enrollment.
    pub computer_id: u64,
    /// Feature names announced in `Hello`.
    pub capabilities: Vec<String>,
    /// First retry delay (1 s).
    pub backoff_min: Duration,
    /// Longest retry delay (60 s).
    pub backoff_max: Duration,
}

fn ws_url(server: &str) -> String {
    format!("{}/agent/ws", server.trim_end_matches('/'))
}

async fn open(server: &str, pin: Option<&str>) -> Result<Ws, ConnError> {
    let url = ws_url(server);
    let connector = if url.starts_with("wss://") {
        Some(guardian_client::ws_connector(pin).map_err(|e| ConnError::Io(e.to_string()))?)
    } else {
        None
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let (ws, _) =
        tokio_tungstenite::connect_async_tls_with_config(url, Some(config), false, connector)
            .await
            .map_err(|e| ConnError::Io(e.to_string()))?;
    Ok(ws)
}

async fn send(ws: &mut Ws, msg: agent_frame::Msg) -> Result<(), ConnError> {
    ws.send(Message::Binary(
        AgentFrame { msg: Some(msg) }.encode_to_vec().into(),
    ))
    .await
    .map_err(|e| ConnError::Io(e.to_string()))
}

/// The next server frame; `ServerError` becomes `Rejected`.
async fn recv(ws: &mut Ws) -> Result<server_frame::Msg, ConnError> {
    loop {
        match ws.next().await {
            Some(Ok(Message::Binary(b))) => {
                let f = ServerFrame::decode(b.as_ref())
                    .map_err(|e| ConnError::Protocol(e.to_string()))?;
                return match f.msg {
                    Some(server_frame::Msg::Error(e)) => Err(ConnError::Rejected {
                        code: e.code,
                        message: e.message,
                    }),
                    Some(m) => Ok(m),
                    None => Err(ConnError::Protocol("empty frame".into())),
                };
            }
            Some(Ok(Message::Close(_))) | None => {
                return Err(ConnError::Protocol("connection closed".into()));
            }
            Some(Err(e)) => return Err(ConnError::Io(e.to_string())),
            Some(Ok(_)) => {}
        }
    }
}

/// Registers this computer with an enrollment code; returns its id.
pub async fn enroll(
    server: &str,
    pin: Option<&str>,
    code: &str,
    key: &KeyPair,
    info: SystemInfo,
) -> Result<u64, ConnError> {
    let mut ws = tokio::time::timeout(HANDSHAKE, open(server, pin))
        .await
        .map_err(|_| ConnError::Io("timed out".into()))??;
    send(
        &mut ws,
        agent_frame::Msg::Enroll(Enroll {
            code: code.into(),
            public_key: key.public().as_bytes().to_vec(),
            info: Some(info),
        }),
    )
    .await?;
    let r = tokio::time::timeout(HANDSHAKE, recv(&mut ws))
        .await
        .map_err(|_| ConnError::Io("timed out".into()))?;
    let _ = ws.close(None).await;
    match r? {
        server_frame::Msg::Enrolled(e) => Ok(e.computer_id),
        other => Err(ConnError::Protocol(format!(
            "expected Enrolled, got {other:?}"
        ))),
    }
}

async fn handshake(
    cfg: &ConnConfig,
    key: &KeyPair,
    info: &SystemInfo,
) -> Result<(Ws, Welcome), ConnError> {
    let mut ws = open(&cfg.server, cfg.pin.as_deref()).await?;
    send(
        &mut ws,
        agent_frame::Msg::Hello(Hello {
            computer_id: cfg.computer_id,
            protocol_version: PROTOCOL_VERSION,
            capabilities: cfg.capabilities.clone(),
            info: Some(info.clone()),
        }),
    )
    .await?;
    let server_frame::Msg::Challenge(c) = recv(&mut ws).await? else {
        return Err(ConnError::Protocol("expected Challenge".into()));
    };
    let nonce: [u8; 32] = c
        .nonce
        .as_slice()
        .try_into()
        .map_err(|_| ConnError::Protocol("nonce must be 32 bytes".into()))?;
    let sig = key.sign_challenge(AuthContext::Agent, cfg.computer_id, &nonce);
    send(
        &mut ws,
        agent_frame::Msg::ChallengeResponse(ChallengeResponse {
            signature: sig.to_bytes().to_vec(),
        }),
    )
    .await?;
    match recv(&mut ws).await? {
        server_frame::Msg::Welcome(w) => Ok((ws, w)),
        other => Err(ConnError::Protocol(format!(
            "expected Welcome, got {other:?}"
        ))),
    }
}

#[derive(Debug, Default)]
struct Status {
    online_since: Option<Instant>,
    offline_since: Option<Instant>,
}

/// The running connection loop.
pub struct Connection {
    status: Arc<Mutex<Status>>,
    results: mpsc::UnboundedSender<CommandResult>,
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("online", &self.online_since().is_some())
            .finish()
    }
}

/// Delay before attempt `n` (0-based): `min(max, min * 2^n)` ± 20 %.
pub fn backoff(n: u32, min: Duration, max: Duration) -> Duration {
    let base = min.saturating_mul(2u32.saturating_pow(n.min(16))).min(max);
    let r = f64::from(u16::from_le_bytes(random_bytes::<2>())) / f64::from(u16::MAX);
    base.mul_f64(0.8 + 0.4 * r).min(max.mul_f64(1.2))
}

enum Ended {
    Lost(String),
    Revoked,
    Stopped,
}

async fn session(
    mut ws: Ws,
    events: &mpsc::Sender<ConnEvent>,
    results: &mut mpsc::UnboundedReceiver<CommandResult>,
    stop: &mut watch::Receiver<bool>,
) -> Ended {
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut last_rx = Instant::now();
    loop {
        tokio::select! {
            _ = stop.changed() => {
                let _ = ws.close(None).await;
                return Ended::Stopped;
            }
            r = results.recv() => {
                let Some(r) = r else { return Ended::Stopped };
                if let Err(e) = send(&mut ws, agent_frame::Msg::CommandResult(r)).await {
                    return Ended::Lost(e.to_string());
                }
            }
            m = ws.next() => {
                last_rx = Instant::now();
                match m {
                    Some(Ok(Message::Binary(b))) => {
                        let Ok(f) = ServerFrame::decode(b.as_ref()) else {
                            return Ended::Lost("undecodable frame".into());
                        };
                        let ev = match f.msg {
                            Some(server_frame::Msg::State(st)) => ConnEvent::State(st),
                            Some(server_frame::Msg::TrustUpdate(w)) => ConnEvent::Trust(w),
                            Some(server_frame::Msg::Command(c)) => ConnEvent::Command(c),
                            Some(server_frame::Msg::Error(e)) if e.code == "revoked" => return Ended::Revoked,
                            Some(server_frame::Msg::Error(e)) => return Ended::Lost(format!("{}: {}", e.code, e.message)),
                            Some(_) | None => continue,
                        };
                        if events.send(ev).await.is_err() {
                            return Ended::Stopped;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return Ended::Lost("closed by server".into()),
                    Some(Err(e)) => return Ended::Lost(e.to_string()),
                    Some(Ok(_)) => {}
                }
            }
            _ = ping.tick() => {
                if last_rx.elapsed() >= IDLE {
                    return Ended::Lost(format!("no traffic for {IDLE:?}"));
                }
                if ws.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return Ended::Lost("ping failed".into());
                }
            }
        }
    }
}

impl Connection {
    /// Starts the loop; events go to `events`.
    pub fn spawn(
        cfg: ConnConfig,
        key: KeyPair,
        info: SystemInfo,
        events: mpsc::Sender<ConnEvent>,
    ) -> Connection {
        let status = Arc::new(Mutex::new(Status {
            online_since: None,
            offline_since: Some(Instant::now()),
        }));
        let (results_tx, mut results) = mpsc::unbounded_channel();
        let (stop, mut stop_rx) = watch::channel(false);
        let st = status.clone();
        let set = move |online: bool| {
            if let Ok(mut s) = st.lock() {
                if online {
                    s.online_since = Some(Instant::now());
                    s.offline_since = None;
                } else if s.online_since.take().is_some() || s.offline_since.is_none() {
                    s.offline_since = Some(Instant::now());
                }
            }
        };
        let task = tokio::spawn(async move {
            let mut attempt = 0u32;
            loop {
                let hs = tokio::select! {
                    _ = stop_rx.changed() => return,
                    r = tokio::time::timeout(HANDSHAKE, handshake(&cfg, &key, &info)) => r,
                };
                let reason = match hs {
                    Ok(Ok((ws, welcome))) => {
                        attempt = 0;
                        set(true);
                        tracing::info!("connected to {}", cfg.server);
                        if events.send(ConnEvent::Online { welcome }).await.is_err() {
                            return;
                        }
                        match session(ws, &events, &mut results, &mut stop_rx).await {
                            Ended::Stopped => return,
                            Ended::Revoked => {
                                set(false);
                                tracing::warn!("this computer was revoked by the server");
                                let _ = events.send(ConnEvent::Revoked).await;
                                return;
                            }
                            Ended::Lost(r) => r,
                        }
                    }
                    Ok(Err(ConnError::Rejected { code, .. })) if code == "revoked" => {
                        set(false);
                        let _ = events.send(ConnEvent::Revoked).await;
                        return;
                    }
                    Ok(Err(e)) => e.to_string(),
                    Err(_) => "handshake timed out".into(),
                };
                set(false);
                tracing::warn!("server connection lost: {reason}");
                if events.send(ConnEvent::Offline { reason }).await.is_err() {
                    return;
                }
                let wait = backoff(attempt, cfg.backoff_min, cfg.backoff_max);
                attempt = attempt.saturating_add(1);
                tokio::select! {
                    _ = stop_rx.changed() => return,
                    _ = tokio::time::sleep(wait) => {}
                }
            }
        });
        Connection {
            status,
            results: results_tx,
            stop,
            task,
        }
    }

    /// Queues a command result (sent on the current or next connection).
    pub fn send_result(&self, r: CommandResult) {
        let _ = self.results.send(r);
    }

    /// When the current authenticated connection began.
    pub fn online_since(&self) -> Option<Instant> {
        self.status.lock().ok().and_then(|s| s.online_since)
    }

    /// How long there has been no authenticated connection.
    pub fn offline_for(&self) -> Option<Duration> {
        self.status
            .lock()
            .ok()
            .and_then(|s| s.offline_since)
            .map(|t| t.elapsed())
    }

    /// Closes the connection and stops the loop.
    pub async fn shutdown(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let (min, max) = (Duration::from_secs(1), Duration::from_secs(60));
        let b0 = backoff(0, min, max);
        assert!(
            b0 >= Duration::from_millis(800) && b0 <= Duration::from_millis(1200),
            "{b0:?}"
        );
        let b3 = backoff(3, min, max);
        assert!(
            b3 >= Duration::from_millis(6400) && b3 <= Duration::from_millis(9600),
            "{b3:?}"
        );
        for n in [10, 30, 1000] {
            assert!(backoff(n, min, max) <= Duration::from_secs(72));
            assert!(backoff(n, min, max) >= Duration::from_secs(48));
        }
    }
}
