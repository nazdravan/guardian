//! The local fallback listener (§11): TCP for the Noise channel, UDP for
//! discovery, on the same port. Only trusted consoles get anything back.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use guardian_core::keys::{KeyPair, PublicKey};
use guardian_local::channel::{self, ChannelError, Limits};
use guardian_local::discover::{self, MAX_DATAGRAM};
use guardian_proto::v1::{DiscoverRequest, LocalReply, LocalRequest};
use prost::Message as _;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Semaphore, watch};
use tokio::task::{AbortHandle, JoinHandle, JoinSet};

/// Most signed-in console sessions at once.
pub const MAX_SESSIONS: usize = 16;
/// Most connections still in the handshake (before their first request
/// decrypts); a new one evicts the oldest.
pub const MAX_PENDING: usize = 16;
/// Most pending handshakes from one address; a new one evicts that address's oldest.
pub const MAX_PENDING_PER_IP: usize = 4;
/// Most discovery replies per second.
pub const MAX_REPLIES_PER_SECOND: u32 = 50;
/// Most discovery replies per second to one address.
pub const MAX_REPLIES_PER_IP: u32 = 5;
/// Discovery nonces remembered (each is answered once).
const SEEN_NONCES: usize = 256;

/// Where to listen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalOptions {
    /// The Noise channel.
    pub tcp: SocketAddr,
    /// Discovery.
    pub udp: SocketAddr,
    /// Channel timeouts.
    pub limits: Limits,
}

/// What the listener needs from the agent.
pub(crate) trait LocalService: Send + Sync + 'static {
    /// Answers one request from an authenticated console.
    fn request(&self, console_id: i64, req: LocalRequest) -> LocalReply;
    /// The consoles the server trusts now.
    fn trusted(&self) -> Vec<(i64, PublicKey)>;
    /// Connected to the server now.
    fn online(&self) -> bool;
    /// This computer.
    fn computer_id(&self) -> u64;
}

/// The running listener.
pub(crate) struct Listener {
    addrs: (SocketAddr, SocketAddr),
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl Listener {
    /// Binds both sockets; `None` (logged) when either port is taken.
    pub(crate) async fn start(
        o: LocalOptions,
        key: KeyPair,
        svc: Arc<dyn LocalService>,
    ) -> Option<Listener> {
        let tcp = match TcpListener::bind(o.tcp).await {
            Ok(t) => t,
            Err(e) => {
                tracing::error!("local fallback is off: cannot listen on TCP {}: {e}", o.tcp);
                return None;
            }
        };
        let udp = match UdpSocket::bind(o.udp).await {
            Ok(u) => u,
            Err(e) => {
                tracing::error!("local fallback is off: cannot listen on UDP {}: {e}", o.udp);
                return None;
            }
        };
        let addrs = (tcp.local_addr().ok()?, udp.local_addr().ok()?);
        tracing::info!(
            "local fallback: listening on TCP {} and UDP {}",
            addrs.0,
            addrs.1
        );
        let (stop, stop_rx) = watch::channel(false);
        let task = tokio::spawn(serve(
            tcp,
            udp,
            addrs.0.port(),
            o.limits,
            Arc::new(key),
            svc,
            stop_rx,
        ));
        Some(Listener { addrs, stop, task })
    }

    /// The bound TCP and UDP addresses.
    pub(crate) fn addrs(&self) -> (SocketAddr, SocketAddr) {
        self.addrs
    }

    /// Stops listening and closes every connection.
    pub(crate) async fn shutdown(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

/// Connections still in the handshake, oldest first.
#[derive(Default)]
struct Pending {
    next: u64,
    list: VecDeque<(u64, IpAddr, AbortHandle)>,
}

impl Pending {
    fn evict_oldest(&mut self, ip: Option<IpAddr>) {
        let pos = match ip {
            Some(ip) => self.list.iter().position(|(_, i, _)| *i == ip),
            None => (!self.list.is_empty()).then_some(0),
        };
        if let Some((_, peer, h)) = pos.and_then(|p| self.list.remove(p)) {
            tracing::debug!("local fallback: dropping a pending handshake from {peer}");
            h.abort();
        }
    }

    fn remove(&mut self, id: u64) {
        self.list.retain(|(i, _, _)| *i != id);
    }
}

fn lock_pending(p: &Mutex<Pending>) -> std::sync::MutexGuard<'_, Pending> {
    match p.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    }
}

/// Discovery: nonces already answered and per-address counters, both per second.
#[derive(Default)]
struct DiscoverGate {
    window: Option<Instant>,
    total: u32,
    per_ip: HashMap<IpAddr, u32>,
    seen: VecDeque<(i64, Vec<u8>)>,
}

impl DiscoverGate {
    /// Whether another reply to `from` fits this second.
    fn allow(&mut self, from: IpAddr) -> bool {
        if self
            .window
            .is_none_or(|w| w.elapsed() >= Duration::from_secs(1))
        {
            self.window = Some(Instant::now());
            self.total = 0;
            self.per_ip.clear();
        }
        let n = self.per_ip.entry(from).or_default();
        if self.total >= MAX_REPLIES_PER_SECOND || *n >= MAX_REPLIES_PER_IP {
            return false;
        }
        *n += 1;
        self.total += 1;
        true
    }

    /// `true` the first time a (console, nonce) pair is seen.
    fn fresh(&mut self, console_id: i64, nonce: &[u8]) -> bool {
        if self
            .seen
            .iter()
            .any(|(c, n)| *c == console_id && n.as_slice() == nonce)
        {
            return false;
        }
        self.seen.push_back((console_id, nonce.to_vec()));
        if self.seen.len() > SEEN_NONCES {
            self.seen.pop_front();
        }
        true
    }
}

async fn serve(
    tcp: TcpListener,
    udp: UdpSocket,
    port: u16,
    limits: Limits,
    key: Arc<KeyPair>,
    svc: Arc<dyn LocalService>,
    mut stop: watch::Receiver<bool>,
) {
    let sessions = Arc::new(Semaphore::new(MAX_SESSIONS));
    let pending = Arc::new(Mutex::new(Pending::default()));
    let mut conns = JoinSet::new();
    let mut buf = vec![0u8; MAX_DATAGRAM + 1];
    let mut gate = DiscoverGate::default();
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            Some(_) = conns.join_next(), if !conns.is_empty() => {}
            a = tcp.accept() => match a {
                Ok((stream, peer)) => {
                    let ip = peer.ip();
                    let mut p = lock_pending(&pending);
                    if p.list.iter().filter(|(_, i, _)| *i == ip).count() >= MAX_PENDING_PER_IP {
                        p.evict_oldest(Some(ip));
                    }
                    if p.list.len() >= MAX_PENDING {
                        p.evict_oldest(None);
                    }
                    let id = p.next;
                    p.next += 1;
                    let (key, svc, sessions, pend) =
                        (key.clone(), svc.clone(), sessions.clone(), pending.clone());
                    let h = conns.spawn(async move {
                        connection(stream, peer, &key, svc, limits, sessions, &pend, id).await;
                        lock_pending(&pend).remove(id);
                    });
                    p.list.push_back((id, ip, h));
                }
                Err(e) => {
                    tracing::warn!("local fallback: accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            r = udp.recv_from(&mut buf) => match r {
                Ok((n, from)) => {
                    if n > MAX_DATAGRAM {
                        continue;
                    }
                    if let Some(out) = answer(&buf[..n], from.ip(), port, &key, &*svc, &mut gate) {
                        let _ = udp.send_to(&out, from).await;
                    }
                }
                Err(e) => tracing::debug!("local fallback: UDP receive failed: {e}"),
            },
        }
    }
    conns.shutdown().await;
}

/// The encoded reply to a discovery datagram, if it comes from a trusted
/// console, was not answered before and fits the rate limits.
fn answer(
    datagram: &[u8],
    from: IpAddr,
    port: u16,
    key: &KeyPair,
    svc: &dyn LocalService,
    gate: &mut DiscoverGate,
) -> Option<Vec<u8>> {
    let req = DiscoverRequest::decode(datagram).ok()?;
    let console = discover::verify_request(&req, &svc.trusted())?;
    if !gate.fresh(console, &req.nonce) || !gate.allow(from) {
        return None;
    }
    Some(discover::reply(key, svc.computer_id(), &req.nonce, port, svc.online()).encode_to_vec())
}

#[allow(clippy::too_many_arguments)]
async fn connection(
    stream: TcpStream,
    peer: SocketAddr,
    key: &KeyPair,
    svc: Arc<dyn LocalService>,
    limits: Limits,
    sessions: Arc<Semaphore>,
    pending: &Mutex<Pending>,
    id: u64,
) {
    // The handshake and the first request share one deadline: a recorded
    // first handshake message passes `accept`, but its sender cannot encrypt
    // a request, so it is closed as fast as a silent peer.
    let deadline = tokio::time::Instant::now() + limits.handshake;
    let first = tokio::time::timeout_at(deadline, async {
        let (mut ch, console_id) = channel::accept(stream, key, &svc.trusted(), limits).await?;
        let req: LocalRequest = ch.recv().await?;
        Ok::<_, ChannelError>((ch, console_id, req))
    })
    .await;
    let (mut ch, console_id, mut req) = match first {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => {
            tracing::debug!("local fallback: refused {peer}: {e}");
            return;
        }
        Err(_) => {
            tracing::debug!("local fallback: refused {peer}: timed out");
            return;
        }
    };
    lock_pending(pending).remove(id);
    let Ok(_permit) = sessions.try_acquire_owned() else {
        tracing::warn!("local fallback: too many console sessions; closing {peer}");
        return;
    };
    tracing::info!("local fallback: console {console_id} connected from {peer}");
    loop {
        let reply = svc.request(console_id, req);
        if let Err(e) = ch.send(&reply).await {
            tracing::debug!("local fallback: console {console_id}: {e}");
            return;
        }
        req = match ch.recv().await {
            Ok(r) => r,
            Err(ChannelError::Closed | ChannelError::Timeout) => return,
            Err(e) => {
                tracing::debug!("local fallback: console {console_id}: {e}");
                return;
            }
        };
    }
}
