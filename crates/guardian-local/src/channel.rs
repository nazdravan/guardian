//! Noise IK over TCP: the console (initiator) knows the agent's key from its
//! cache; the agent (responder) accepts only the keys of trusted consoles.
//! Each frame is a big-endian `u16` length followed by a Noise message that
//! carries one protobuf message.

use std::net::SocketAddr;
use std::time::Duration;

use guardian_core::keys::{KeyPair, PublicKey};
use snow::{HandshakeState, TransportState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The Noise protocol name.
pub const NOISE_PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";

/// Largest Noise message (protocol limit).
const MAX_FRAME: usize = 65535;
/// ChaChaPoly tag.
const TAG: usize = 16;

/// Timeouts of one connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The whole handshake.
    pub handshake: Duration,
    /// Waiting for the next message.
    pub idle: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            handshake: Duration::from_secs(5),
            idle: Duration::from_secs(30),
        }
    }
}

/// Why the channel failed.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    /// Socket error.
    #[error("network: {0}")]
    Io(#[from] std::io::Error),
    /// The peer was too slow.
    #[error("timed out")]
    Timeout,
    /// The Noise handshake failed (wrong key, garbage).
    #[error("handshake failed")]
    Handshake,
    /// The console is not one of the trusted consoles.
    #[error("unknown console")]
    UnknownPeer,
    /// The message does not fit in one frame.
    #[error("message too large")]
    FrameTooLarge,
    /// A frame did not decrypt.
    #[error("bad frame")]
    Crypto,
    /// The protobuf did not decode.
    #[error("undecodable message: {0}")]
    Decode(#[from] prost::DecodeError),
    /// The peer closed the connection.
    #[error("connection closed")]
    Closed,
}

/// An established channel.
pub struct LocalStream {
    tcp: TcpStream,
    noise: TransportState,
    idle: Duration,
}

impl std::fmt::Debug for LocalStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalStream")
            .field("peer", &self.tcp.peer_addr().ok())
            .finish_non_exhaustive()
    }
}

async fn read_frame(tcp: &mut TcpStream) -> Result<Vec<u8>, ChannelError> {
    let mut len = [0u8; 2];
    match tcp.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(ChannelError::Closed);
        }
        Err(e) => return Err(e.into()),
    }
    let n = usize::from(u16::from_be_bytes(len));
    if n == 0 {
        return Err(ChannelError::Crypto);
    }
    let mut buf = vec![0u8; n];
    match tcp.read_exact(&mut buf).await {
        Ok(_) => Ok(buf),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Err(ChannelError::Closed),
        Err(e) => Err(e.into()),
    }
}

async fn write_frame(tcp: &mut TcpStream, data: &[u8]) -> Result<(), ChannelError> {
    let n = u16::try_from(data.len()).map_err(|_| ChannelError::FrameTooLarge)?;
    let mut out = Vec::with_capacity(2 + data.len());
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(data);
    tcp.write_all(&out).await?;
    Ok(())
}

fn builder_params() -> Result<snow::params::NoiseParams, ChannelError> {
    NOISE_PATTERN.parse().map_err(|_| ChannelError::Handshake)
}

async fn with_timeout<T>(
    d: Duration,
    f: impl std::future::Future<Output = Result<T, ChannelError>>,
) -> Result<T, ChannelError> {
    tokio::time::timeout(d, f)
        .await
        .map_err(|_| ChannelError::Timeout)?
}

/// Connects to the agent at `addr` whose ed25519 key is `agent` (console side).
pub async fn connect(
    addr: SocketAddr,
    me: &KeyPair,
    console_id: i64,
    agent: &PublicKey,
    limits: Limits,
) -> Result<LocalStream, ChannelError> {
    let secret = me.x25519_secret();
    let remote = agent.x25519().map_err(|_| ChannelError::Handshake)?;
    let mut hs: HandshakeState = snow::Builder::new(builder_params()?)
        .local_private_key(secret.as_slice())
        .and_then(|b| b.remote_public_key(&remote))
        .and_then(|b| b.build_initiator())
        .map_err(|_| ChannelError::Handshake)?;
    with_timeout(limits.handshake, async move {
        let mut tcp = TcpStream::connect(addr).await?;
        let _ = tcp.set_nodelay(true);
        let mut buf = vec![0u8; MAX_FRAME];
        let n = hs
            .write_message(&console_id.to_le_bytes(), &mut buf)
            .map_err(|_| ChannelError::Handshake)?;
        write_frame(&mut tcp, &buf[..n]).await?;
        let msg = read_frame(&mut tcp).await?;
        let mut payload = vec![0u8; MAX_FRAME];
        hs.read_message(&msg, &mut payload)
            .map_err(|_| ChannelError::Handshake)?;
        let noise = hs
            .into_transport_mode()
            .map_err(|_| ChannelError::Handshake)?;
        Ok(LocalStream {
            tcp,
            noise,
            idle: limits.idle,
        })
    })
    .await
}

/// Accepts a console on `tcp` (agent side). The console's static key must be
/// one of `trusted`, and the console id it claims must be that key's id.
pub async fn accept(
    tcp: TcpStream,
    me: &KeyPair,
    trusted: &[(i64, PublicKey)],
    limits: Limits,
) -> Result<(LocalStream, i64), ChannelError> {
    let secret = me.x25519_secret();
    let mut hs: HandshakeState = snow::Builder::new(builder_params()?)
        .local_private_key(secret.as_slice())
        .and_then(|b| b.build_responder())
        .map_err(|_| ChannelError::Handshake)?;
    let known: Vec<(i64, [u8; 32])> = trusted
        .iter()
        .filter_map(|(id, k)| k.x25519().ok().map(|x| (*id, x)))
        .collect();
    let mut tcp = tcp;
    with_timeout(limits.handshake, async move {
        let _ = tcp.set_nodelay(true);
        let msg = read_frame(&mut tcp).await?;
        let mut payload = vec![0u8; MAX_FRAME];
        let n = hs
            .read_message(&msg, &mut payload)
            .map_err(|_| ChannelError::Handshake)?;
        let claimed: [u8; 8] = payload
            .get(..n)
            .and_then(|p| p.try_into().ok())
            .ok_or(ChannelError::Handshake)?;
        let claimed = i64::from_le_bytes(claimed);
        let remote = hs.get_remote_static().ok_or(ChannelError::Handshake)?;
        let console_id = known
            .iter()
            .find(|(_, k)| k.as_slice() == remote)
            .map(|(id, _)| *id)
            .ok_or(ChannelError::UnknownPeer)?;
        if console_id != claimed {
            return Err(ChannelError::UnknownPeer);
        }
        let mut buf = vec![0u8; MAX_FRAME];
        let n = hs
            .write_message(&[], &mut buf)
            .map_err(|_| ChannelError::Handshake)?;
        write_frame(&mut tcp, &buf[..n]).await?;
        let noise = hs
            .into_transport_mode()
            .map_err(|_| ChannelError::Handshake)?;
        Ok((
            LocalStream {
                tcp,
                noise,
                idle: limits.idle,
            },
            console_id,
        ))
    })
    .await
}

impl LocalStream {
    /// Encrypts and sends one message.
    pub async fn send<M: prost::Message>(&mut self, m: &M) -> Result<(), ChannelError> {
        let plain = m.encode_to_vec();
        if plain.len() > MAX_FRAME - TAG {
            return Err(ChannelError::FrameTooLarge);
        }
        let mut buf = vec![0u8; plain.len() + TAG];
        let n = self
            .noise
            .write_message(&plain, &mut buf)
            .map_err(|_| ChannelError::Crypto)?;
        write_frame(&mut self.tcp, &buf[..n]).await
    }

    /// Waits (at most the idle limit) for one message.
    pub async fn recv<M: prost::Message + Default>(&mut self) -> Result<M, ChannelError> {
        let frame = tokio::time::timeout(self.idle, read_frame(&mut self.tcp))
            .await
            .map_err(|_| ChannelError::Timeout)??;
        let mut plain = vec![0u8; frame.len()];
        let n = self
            .noise
            .read_message(&frame, &mut plain)
            .map_err(|_| ChannelError::Crypto)?;
        Ok(M::decode(plain.get(..n).unwrap_or_default())?)
    }

    /// The peer's address.
    pub fn peer_addr(&self) -> Option<SocketAddr> {
        self.tcp.peer_addr().ok()
    }

    /// The raw socket (tests write garbage through it).
    pub fn into_inner(self) -> TcpStream {
        self.tcp
    }
}
