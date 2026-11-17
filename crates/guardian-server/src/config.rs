//! Server configuration.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// How the listener secures connections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsMode {
    /// PEM certificate chain and private key from files.
    Files {
        /// Certificate chain (PEM).
        cert: PathBuf,
        /// Private key (PEM).
        key: PathBuf,
    },
    /// A self-signed certificate generated on first start and kept in `data_dir/tls/`.
    SelfSigned,
    /// No TLS at all. Only for tests and for a server behind a TLS-terminating proxy.
    InsecurePlaintext,
}

/// Everything `start()` needs.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Holds `guardian.db` and `tls/`.
    pub data_dir: PathBuf,
    /// Listen address (default `0.0.0.0:8443`).
    pub listen: SocketAddr,
    /// TLS mode.
    pub tls: TlsMode,
    /// An agent must finish the WebSocket handshake within this (default 10 s).
    pub handshake_timeout: Duration,
    /// An agent that sends nothing for this long is dropped (default 60 s).
    pub idle_timeout: Duration,
    /// WebSocket ping interval (default 20 s).
    pub ping_interval: Duration,
    /// How often expired commands are swept (default 10 s).
    pub expiry_interval: Duration,
    /// How often old rows are pruned (default 24 h).
    pub retention_interval: Duration,
    /// Allowed `Init`/`Login`/`EnrollConsole`/agent-enroll attempts per IP per minute (default 10).
    pub auth_rate_per_minute: u32,
}

impl ServerConfig {
    /// A config with the defaults listed on each field.
    pub fn new(data_dir: PathBuf, tls: TlsMode) -> ServerConfig {
        ServerConfig {
            data_dir,
            listen: SocketAddr::from(([0, 0, 0, 0], 8443)),
            tls,
            handshake_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(60),
            ping_interval: Duration::from_secs(20),
            expiry_interval: Duration::from_secs(10),
            retention_interval: Duration::from_secs(24 * 60 * 60),
            auth_rate_per_minute: 10,
        }
    }
}
