//! `/agent/ws`: agent enrollment, the ed25519 challenge handshake, and the
//! connection loop.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::response::Response;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use guardian_core::feature::PROTOCOL_VERSION;
use guardian_core::keys::{AuthContext, PublicKey, Signature};
use guardian_core::random::random_bytes;
use guardian_proto::v1::{
    AgentFrame, Challenge, Enrolled, Hello, ServerError, ServerFrame, SystemInfo, Welcome,
    agent_frame, server_frame,
};
use prost::Message as _;
use tokio::sync::mpsc;

use crate::db::codes::{CodeError, CodeKind, INVALID_CODE};
use crate::db::{self, now_unix};
use crate::hub::{HubEvent, build_agent_state, build_trust};
use crate::state::AppState;

/// Largest WebSocket message (1 MiB).
pub const MAX_WS_MESSAGE: usize = 1024 * 1024;
/// While connected, `last_seen` is written at most this often.
const LAST_SEEN_EVERY: Duration = Duration::from_secs(300);
/// Frames queued per connection before `Hub::send` starts failing.
const QUEUE: usize = 256;

type Sink = SplitSink<WebSocket, Message>;
type Stream = SplitStream<WebSocket>;

/// The axum handler.
pub async fn handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Response {
    ws.max_message_size(MAX_WS_MESSAGE)
        .max_frame_size(MAX_WS_MESSAGE)
        .on_upgrade(move |socket| run(socket, state, peer))
}

fn frame(msg: server_frame::Msg) -> ServerFrame {
    ServerFrame { msg: Some(msg) }
}

fn error(code: &str, message: &str) -> ServerFrame {
    frame(server_frame::Msg::Error(ServerError {
        code: code.into(),
        message: message.into(),
    }))
}

async fn send(sink: &mut Sink, f: &ServerFrame) -> Result<(), axum::Error> {
    sink.send(Message::Binary(f.encode_to_vec().into())).await
}

/// Reads the next binary frame, answering nothing else.
async fn recv(stream: &mut Stream) -> Option<AgentFrame> {
    loop {
        match stream.next().await? {
            Ok(Message::Binary(b)) => return AgentFrame::decode(b.as_ref()).ok(),
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

/// A handshake failure: the error frame to send before closing.
struct Reject(ServerFrame);

impl Reject {
    fn new(code: &str, message: &str) -> Reject {
        Reject(error(code, message))
    }
}

struct Authenticated {
    id: u64,
    caps: BTreeSet<String>,
    info: SystemInfo,
}

async fn handshake(
    sink: &mut Sink,
    stream: &mut Stream,
    state: &AppState,
    peer: SocketAddr,
) -> Result<Option<Authenticated>, Reject> {
    let Some(first) = recv(stream).await else {
        return Ok(None);
    };
    let (hello, enrolled_id) = match first.msg {
        Some(agent_frame::Msg::Enroll(e)) => {
            if state.limiter.check(peer.ip()).is_err() {
                return Err(Reject::new(
                    "rate_limited",
                    "too many attempts, try again later",
                ));
            }
            let invalid = || {
                state.limiter.failed(peer.ip());
                Reject::new("invalid_code", INVALID_CODE)
            };
            let key = PublicKey::from_bytes(&e.public_key).map_err(|_| invalid())?;
            if db::computers::by_key(&state.db, &key)
                .await
                .map_err(|_| Reject::new("protocol", "internal error"))?
                .is_some()
            {
                return Err(invalid());
            }
            let room = match db::codes::consume(&state.db, &e.code, CodeKind::Agent).await {
                Ok(room) => room,
                Err(CodeError::Invalid) => return Err(invalid()),
                Err(CodeError::Db(_)) => return Err(Reject::new("protocol", "internal error")),
            };
            let info = e.info.unwrap_or_default();
            let id = db::computers::insert(&state.db, &key, &info, room)
                .await
                .map_err(|_| Reject::new("invalid_code", INVALID_CODE))?;
            state.audit(
                None,
                "EnrollAgent",
                format!("computer:{id}"),
                info.hostname.clone(),
            );
            send(
                sink,
                &frame(server_frame::Msg::Enrolled(Enrolled { computer_id: id })),
            )
            .await
            .map_err(|_| Reject::new("protocol", "send failed"))?;
            let Some(next) = recv(stream).await else {
                return Ok(None);
            };
            match next.msg {
                Some(agent_frame::Msg::Hello(h)) => (h, Some(id)),
                _ => return Err(Reject::new("protocol", "expected Hello")),
            }
        }
        Some(agent_frame::Msg::Hello(h)) => (h, None),
        _ => return Err(Reject::new("protocol", "expected Enroll or Hello")),
    };
    let Hello {
        computer_id,
        protocol_version,
        capabilities,
        info,
    } = hello;
    if protocol_version != PROTOCOL_VERSION {
        return Err(Reject::new(
            "protocol",
            &format!(
                "unsupported protocol version {protocol_version}, server speaks {PROTOCOL_VERSION}"
            ),
        ));
    }
    if enrolled_id.is_some_and(|e| e != computer_id) {
        return Err(Reject::new("protocol", "Hello must use the enrolled id"));
    }
    let row = db::computers::get(&state.db, computer_id)
        .await
        .map_err(|_| Reject::new("protocol", "internal error"))?
        .ok_or_else(|| Reject::new("unauthorized", "unknown computer"))?;
    let key = PublicKey::from_bytes(&row.public_key)
        .map_err(|_| Reject::new("unauthorized", "unknown computer"))?;
    let nonce = random_bytes::<32>();
    send(
        sink,
        &frame(server_frame::Msg::Challenge(Challenge {
            nonce: nonce.to_vec(),
        })),
    )
    .await
    .map_err(|_| Reject::new("protocol", "send failed"))?;
    let Some(resp) = recv(stream).await else {
        return Ok(None);
    };
    let Some(agent_frame::Msg::ChallengeResponse(cr)) = resp.msg else {
        return Err(Reject::new("protocol", "expected ChallengeResponse"));
    };
    let sig = Signature::from_bytes(&cr.signature)
        .map_err(|_| Reject::new("unauthorized", "bad signature"))?;
    key.verify_challenge(AuthContext::Agent, computer_id, &nonce, &sig)
        .map_err(|_| Reject::new("unauthorized", "bad signature"))?;
    Ok(Some(Authenticated {
        id: computer_id,
        caps: capabilities.into_iter().collect(),
        info: info.unwrap_or_default(),
    }))
}

async fn welcome(
    state: &AppState,
    id: u64,
    caps: &BTreeSet<String>,
) -> Result<Welcome, db::DbError> {
    let agent_state = build_agent_state(&state.db, id).await?;
    let (consoles, users) = build_trust(&state.db, id).await?;
    Ok(Welcome {
        state: Some(agent_state),
        pending: state.take_pending(id, caps).await,
        consoles,
        users,
    })
}

async fn run(socket: WebSocket, state: Arc<AppState>, peer: SocketAddr) {
    let (mut sink, mut stream) = socket.split();
    let hs = tokio::time::timeout(
        state.config.handshake_timeout,
        handshake(&mut sink, &mut stream, &state, peer),
    )
    .await;
    let auth = match hs {
        Ok(Ok(Some(a))) => a,
        Ok(Err(Reject(f))) => {
            let _ = send(&mut sink, &f).await;
            let _ = sink.close().await;
            return;
        }
        Ok(Ok(None)) | Err(_) => {
            let _ = sink.close().await;
            return;
        }
    };
    let id = auth.id;
    if let Err(e) = db::computers::set_system_info(&state.db, id, &auth.info).await {
        tracing::warn!("computer {id}: storing system info failed: {e}");
    }
    let (tx, mut rx) = mpsc::channel(QUEUE);
    let caps = auth.caps.clone();
    let token = state.hub.register(id, auth.caps, tx);
    // Welcome is written straight to the socket, before anything the hub queued;
    // queued states carry versions the agent compares against.
    let ok = match welcome(&state, id, &caps).await {
        Ok(w) => send(&mut sink, &frame(server_frame::Msg::Welcome(w)))
            .await
            .is_ok(),
        Err(e) => {
            tracing::error!("computer {id}: building welcome failed: {e}");
            false
        }
    };
    if ok {
        tracing::info!("computer {id} connected from {peer}");
        state.hub.emit(HubEvent::ComputerChanged(id));
        connection_loop(&state, id, &mut sink, &mut stream, &mut rx).await;
    }
    let _ = sink.close().await;
    if state.hub.unregister(id, token) {
        tracing::info!("computer {id} disconnected");
        let _ = db::computers::touch_last_seen(&state.db, id, now_unix()).await;
        state.hub.emit(HubEvent::ComputerChanged(id));
    }
}

async fn connection_loop(
    state: &AppState,
    id: u64,
    sink: &mut Sink,
    stream: &mut Stream,
    rx: &mut mpsc::Receiver<ServerFrame>,
) {
    let mut ping = tokio::time::interval(state.config.ping_interval);
    ping.tick().await;
    let mut last_rx = Instant::now();
    let mut last_seen_written = Instant::now();
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                Some(f) => {
                    if send(sink, &f).await.is_err() {
                        return;
                    }
                }
                // Replaced, revoked or shutting down.
                None => return,
            },
            msg = stream.next() => match msg {
                Some(Ok(Message::Binary(b))) => {
                    last_rx = Instant::now();
                    match AgentFrame::decode(b.as_ref()) {
                        Ok(AgentFrame { msg: Some(agent_frame::Msg::CommandResult(r)) }) => {
                            state.command_result_from_agent(id, r).await;
                        }
                        Ok(_) => tracing::debug!("computer {id}: ignoring unexpected frame"),
                        Err(e) => {
                            tracing::warn!("computer {id}: undecodable frame: {e}");
                            let _ = send(sink, &error("protocol", "undecodable frame")).await;
                            return;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => last_rx = Instant::now(),
            },
            _ = ping.tick() => {
                if last_rx.elapsed() >= state.config.idle_timeout {
                    tracing::info!("computer {id}: no traffic for {:?}, dropping", state.config.idle_timeout);
                    return;
                }
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return;
                }
                if last_seen_written.elapsed() >= LAST_SEEN_EVERY {
                    last_seen_written = Instant::now();
                    let _ = db::computers::touch_last_seen(&state.db, id, now_unix()).await;
                }
            }
        }
    }
}
