//! What an agent needs to join this server: the address, the certificate pin
//! and the enrollment command lines. Pure logic: the hostname and addresses are
//! given by the caller.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use crate::embedded::EmbeddedState;
use crate::settings::Mode;

/// The embedded server shows the "agents may not find this PC" hint after this
/// long without any computer online.
pub const NO_AGENT_AFTER: Duration = Duration::from_secs(120);

/// Where agents connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTarget {
    /// `wss://host:port`.
    pub server: String,
    /// Pinned certificate SHA-256.
    pub cert_sha256: Option<String>,
}

/// Remote: the console's `https://` address as `wss://`, with its pin.
/// Embedded: `wss://<hostname>:<port>`; the caller adds the embedded server's pin.
pub fn agent_target(mode: &Mode, hostname: &str, port_override: Option<u16>) -> AgentTarget {
    match mode {
        Mode::Remote {
            server,
            cert_sha256,
        } => AgentTarget {
            server: match server.strip_prefix("https://") {
                Some(rest) => format!("wss://{rest}"),
                None => server.clone(),
            },
            cert_sha256: cert_sha256.clone(),
        },
        Mode::Embedded { port } => AgentTarget {
            server: format!("wss://{hostname}:{}", port_override.unwrap_or(*port)),
            cert_sha256: None,
        },
    }
}

/// The two commands for an elevated prompt on a Windows student computer.
pub fn windows_command(t: &AgentTarget, code: &str) -> String {
    let pin = t
        .cert_sha256
        .as_deref()
        .map(|p| format!(" --cert-sha256 {p}"))
        .unwrap_or_default();
    format!(
        "guardian-agent.exe enroll --server {}{pin} --code {code}\nguardian-agent.exe service install",
        t.server
    )
}

/// The Linux install command (`packaging/agent-linux-v3/install.sh`).
pub fn linux_command(t: &AgentTarget, code: &str) -> String {
    match &t.cert_sha256 {
        Some(p) => format!("sudo ./install.sh {} {code} {p}", t.server),
        None => format!("sudo ./install.sh {} {code}", t.server),
    }
}

fn private_v4(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 10 || (o[0] == 172 && (16..=31).contains(&o[1])) || (o[0] == 192 && o[1] == 168)
        }
        IpAddr::V6(_) => false,
    }
}

/// Private IPv4 addresses as `wss://ip:port`, for agents that cannot resolve
/// the computer name.
pub fn lan_fallback(addrs: &[IpAddr], port: u16) -> Vec<String> {
    addrs
        .iter()
        .filter(|a| private_v4(a))
        .map(|a| format!("wss://{a}:{port}"))
        .collect()
}

/// The embedded server has been running for two minutes and no computer has
/// come online in this run.
pub fn no_agent_warning(state: &EmbeddedState, ever_online: bool, now: Instant) -> bool {
    match state {
        EmbeddedState::Running { started_at, .. } => {
            !ever_online && now.saturating_duration_since(*started_at) >= NO_AGENT_AFTER
        }
        _ => false,
    }
}

/// This computer's name and LAN addresses.
pub fn this_computer() -> (String, Vec<IpAddr>) {
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    let addrs = if_addrs::get_if_addrs()
        .map(|v| v.into_iter().map(|i| i.ip()).collect())
        .unwrap_or_default();
    (name, addrs)
}
