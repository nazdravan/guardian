//! The local fallback listener (§11): TCP for the Noise channel, UDP for
//! discovery, on the same port. Only trusted consoles get anything back.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use guardian_core::keys::{KeyPair, PublicKey};
use guardian_local::channel::{self, ChannelError, Limits};
use guardian_local::discover::{self, MAX_DATAGRAM};
use guardian_proto::v1::{DiscoverRequest, LocalReply, LocalRequest};
use prost::Message as _;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Semaphore, watch};
use tokio::task::{JoinHandle, JoinSet};

/// Most connections handled at once.
pub const MAX_CONNECTIONS: usize = 16;
/// Most discovery replies per second.
pub const MAX_REPLIES_PER_SECOND: u32 = 50;

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

async fn serve(
    tcp: TcpListener,
    udp: UdpSocket,
    port: u16,
    limits: Limits,
    key: Arc<KeyPair>,
    svc: Arc<dyn LocalService>,
    mut stop: watch::Receiver<bool>,
) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let mut conns = JoinSet::new();
    let mut buf = vec![0u8; MAX_DATAGRAM + 1];
    let mut window = Instant::now();
    let mut replies = 0u32;
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            Some(_) = conns.join_next(), if !conns.is_empty() => {}
            a = tcp.accept() => match a {
                Ok((stream, peer)) => match slots.clone().try_acquire_owned() {
                    Ok(permit) => {
                        let (key, svc) = (key.clone(), svc.clone());
                        conns.spawn(async move {
                            connection(stream, peer, &key, svc, limits).await;
                            drop(permit);
                        });
                    }
                    Err(_) => tracing::debug!("local fallback: too many connections; dropping {peer}"),
                },
                Err(e) => {
                    tracing::warn!("local fallback: accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            r = udp.recv_from(&mut buf) => match r {
                Ok((n, from)) => {
                    if window.elapsed() >= Duration::from_secs(1) {
                        window = Instant::now();
                        replies = 0;
                    }
                    if n > MAX_DATAGRAM || replies >= MAX_REPLIES_PER_SECOND {
                        continue;
                    }
                    if let Some(out) = answer(&buf[..n], port, &key, &*svc) {
                        replies += 1;
                        let _ = udp.send_to(&out, from).await;
                    }
                }
                Err(e) => tracing::debug!("local fallback: UDP receive failed: {e}"),
            },
        }
    }
    conns.shutdown().await;
}

/// The encoded reply to a discovery datagram, if it comes from a trusted console.
fn answer(datagram: &[u8], port: u16, key: &KeyPair, svc: &dyn LocalService) -> Option<Vec<u8>> {
    let req = DiscoverRequest::decode(datagram).ok()?;
    discover::verify_request(&req, &svc.trusted())?;
    Some(discover::reply(key, svc.computer_id(), &req.nonce, port, svc.online()).encode_to_vec())
}

async fn connection(
    stream: TcpStream,
    peer: SocketAddr,
    key: &KeyPair,
    svc: Arc<dyn LocalService>,
    limits: Limits,
) {
    let (mut ch, console_id) = match channel::accept(stream, key, &svc.trusted(), limits).await {
        Ok(x) => x,
        Err(e) => {
            tracing::debug!("local fallback: refused {peer}: {e}");
            return;
        }
    };
    tracing::info!("local fallback: console {console_id} connected from {peer}");
    loop {
        let req: LocalRequest = match ch.recv().await {
            Ok(r) => r,
            Err(ChannelError::Closed | ChannelError::Timeout) => return,
            Err(e) => {
                tracing::debug!("local fallback: console {console_id}: {e}");
                return;
            }
        };
        let reply = svc.request(console_id, req);
        if let Err(e) = ch.send(&reply).await {
            tracing::debug!("local fallback: console {console_id}: {e}");
            return;
        }
    }
}
