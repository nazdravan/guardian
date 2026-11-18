//! Talking to agents directly in fallback mode: status probes, the local
//! profile switch, and signed UDP discovery when a cached address is stale.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream;
use guardian_core::keys::{KeyPair, PublicKey};
use guardian_core::random::random_bytes;
use guardian_local::channel::{self, Limits};
use guardian_local::discover;
use guardian_proto::v1::{
    CachedComputer, CachedUser, DiscoverReply, LocalActor, LocalReply, LocalRequest,
    LocalSetProfile, StatusRequest, local_reply, local_request,
};
use prost::Message as _;
use tokio::net::UdpSocket;

use super::cache::Reach;

/// Probes run at once.
pub const PARALLEL: usize = 32;
/// How long a sweep waits for discovery replies.
pub const DISCOVER_WAIT: Duration = Duration::from_secs(1);

/// The console's side of the local channel.
#[derive(Clone)]
pub struct LocalClient {
    key: Arc<KeyPair>,
    console_id: i64,
    port: u16,
    limits: Limits,
    broadcast: Vec<SocketAddr>,
}

impl std::fmt::Debug for LocalClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalClient")
            .field("console_id", &self.console_id)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

/// `255.255.255.255:port` plus the broadcast address of every IPv4 interface.
pub fn default_broadcast(port: u16) -> Vec<SocketAddr> {
    let mut v = vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), port)];
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for i in ifs {
            if let if_addrs::IfAddr::V4(a) = i.addr
                && let Some(b) = a.broadcast
                && !a.ip.is_loopback()
            {
                let s = SocketAddr::new(IpAddr::V4(b), port);
                if !v.contains(&s) {
                    v.push(s);
                }
            }
        }
    }
    v
}

fn actor(u: &CachedUser) -> LocalActor {
    LocalActor {
        user_id: u.user_id,
        login: u.login.clone(),
    }
}

fn request_id() -> String {
    random_bytes::<8>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn reach_of(r: LocalReply) -> Reach {
    match r.kind {
        Some(local_reply::Kind::Status(s)) => Reach::Reachable(s),
        Some(local_reply::Kind::Refused(x)) if x.code == "online" => Reach::StillOnline,
        Some(local_reply::Kind::Refused(x)) => Reach::Refused(x.message),
        Some(local_reply::Kind::CommandResult(_)) | None => {
            Reach::Refused("Unexpected answer.".into())
        }
    }
}

impl LocalClient {
    /// A client for `console_id` reaching agents on `port`; probes give up on an
    /// address after 2 s.
    pub fn new(key: KeyPair, console_id: i64, port: u16) -> LocalClient {
        LocalClient {
            key: Arc::new(key),
            console_id,
            port,
            limits: Limits {
                handshake: Duration::from_secs(2),
                idle: Duration::from_secs(5),
            },
            broadcast: default_broadcast(port),
        }
    }

    /// Other timeouts (tests).
    pub fn with_limits(mut self, l: Limits) -> LocalClient {
        self.limits = l;
        self
    }

    /// Other discovery targets (tests).
    pub fn with_broadcast(mut self, b: Vec<SocketAddr>) -> LocalClient {
        self.broadcast = b;
        self
    }

    fn agent_key(c: &CachedComputer) -> Option<PublicKey> {
        PublicKey::from_bytes(&c.public_key).ok()
    }

    async fn ask(&self, addr: SocketAddr, c: &CachedComputer, req: LocalRequest) -> Option<Reach> {
        let key = Self::agent_key(c)?;
        let mut ch = channel::connect(addr, &self.key, self.console_id, &key, self.limits)
            .await
            .ok()?;
        ch.send(&req).await.ok()?;
        let r: LocalReply = ch.recv().await.ok()?;
        Some(reach_of(r))
    }

    fn candidates(&self, c: &CachedComputer, hint: Option<SocketAddr>) -> Vec<SocketAddr> {
        let mut v: Vec<SocketAddr> = hint.into_iter().collect();
        for a in &c.lan_addresses {
            if let Ok(ip) = a.parse::<IpAddr>() {
                let s = SocketAddr::new(ip, self.port);
                if !v.contains(&s) {
                    v.push(s);
                }
            }
        }
        v
    }

    /// Asks one computer for its status: `hint` first, then each cached LAN
    /// address. Returns what it said and the address that answered.
    pub async fn probe(
        &self,
        c: &CachedComputer,
        user: &CachedUser,
        hint: Option<SocketAddr>,
    ) -> (Reach, Option<SocketAddr>) {
        for addr in self.candidates(c, hint) {
            let req = LocalRequest {
                id: request_id(),
                actor: Some(actor(user)),
                kind: Some(local_request::Kind::Status(StatusRequest {})),
            };
            if let Some(r) = self.ask(addr, c, req).await {
                return (r, Some(addr));
            }
        }
        (Reach::Unreachable, None)
    }

    /// Switches one computer's room profile.
    pub async fn set_profile(
        &self,
        addr: SocketAddr,
        c: &CachedComputer,
        user: &CachedUser,
        room_id: i64,
        profile_id: i64,
    ) -> Reach {
        let req = LocalRequest {
            id: request_id(),
            actor: Some(actor(user)),
            kind: Some(local_request::Kind::SetProfile(LocalSetProfile {
                room_id,
                profile_id,
            })),
        };
        self.ask(addr, c, req).await.unwrap_or(Reach::Unreachable)
    }

    /// Broadcasts one signed request and collects the replies signed by the
    /// keys of `computers` until `wait` is over.
    pub async fn discover(
        &self,
        computers: &[CachedComputer],
        wait: Duration,
    ) -> BTreeMap<u64, SocketAddr> {
        let mut found = BTreeMap::new();
        let Ok(sock) = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).await else {
            return found;
        };
        let _ = sock.set_broadcast(true);
        let (req, nonce) = discover::request(&self.key, self.console_id);
        let bytes = req.encode_to_vec();
        for b in &self.broadcast {
            if let Err(e) = sock.send_to(&bytes, b).await {
                tracing::debug!("discovery to {b}: {e}");
            }
        }
        let keys: BTreeMap<u64, PublicKey> = computers
            .iter()
            .filter_map(|c| Self::agent_key(c).map(|k| (c.id, k)))
            .collect();
        let mut buf = vec![0u8; discover::MAX_DATAGRAM];
        let end = tokio::time::Instant::now() + wait;
        loop {
            let r = tokio::time::timeout_at(end, sock.recv_from(&mut buf)).await;
            let Ok(Ok((n, from))) = r else { break };
            let Some(slice) = buf.get(..n) else { continue };
            let Ok(rep) = DiscoverReply::decode(slice) else {
                continue;
            };
            let Some(key) = keys.get(&rep.computer_id) else {
                continue;
            };
            if discover::verify_reply(&rep, &nonce, key)
                && let Ok(port) = u16::try_from(rep.port)
            {
                found.insert(rep.computer_id, SocketAddr::new(from.ip(), port));
            }
        }
        found
    }

    /// Probes every computer (at most [`PARALLEL`] at once, `known` addresses
    /// first); those that do not answer are looked for once by discovery and
    /// asked again at the address found.
    pub async fn sweep(
        &self,
        computers: &[CachedComputer],
        user: &CachedUser,
        known: &BTreeMap<u64, SocketAddr>,
    ) -> BTreeMap<u64, (Reach, Option<SocketAddr>)> {
        let mut out: BTreeMap<u64, (Reach, Option<SocketAddr>)> = stream::iter(computers)
            .map(|c| async move { (c.id, self.probe(c, user, known.get(&c.id).copied()).await) })
            .buffer_unordered(PARALLEL)
            .collect()
            .await;
        let missing: Vec<CachedComputer> = computers
            .iter()
            .filter(|c| matches!(out.get(&c.id), Some((Reach::Unreachable, _)) | None))
            .cloned()
            .collect();
        if missing.is_empty() {
            return out;
        }
        let found = self.discover(&missing, DISCOVER_WAIT).await;
        let retried: Vec<(u64, (Reach, Option<SocketAddr>))> = stream::iter(missing.iter())
            .filter_map(|c| {
                let addr = found.get(&c.id).copied();
                async move { addr.map(|a| (c, a)) }
            })
            .map(|(c, a)| async move {
                let req = LocalRequest {
                    id: request_id(),
                    actor: Some(actor(user)),
                    kind: Some(local_request::Kind::Status(StatusRequest {})),
                };
                let r = self.ask(a, c, req).await.unwrap_or(Reach::Unreachable);
                let addr = (!matches!(r, Reach::Unreachable)).then_some(a);
                (c.id, (r, addr))
            })
            .buffer_unordered(PARALLEL)
            .collect()
            .await;
        out.extend(retried);
        out
    }
}
