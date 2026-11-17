//! The operating system behind one trait: processes, process-start events, the
//! active user session, system information and trusted executable locations.
//!
//! Only the Windows FFI modules (`windows/ffi_*.rs`) contain `unsafe`; everything
//! else, including all of Linux, is safe Rust.

#![deny(unsafe_code)]

use std::path::Path;
use std::sync::Arc;

use guardian_core::enforce::{Os, ProcessInfo};
use guardian_core::trusted::TrustedRoots;
use guardian_proto::v1::SystemInfo;

#[cfg(feature = "testing")]
pub mod fake;
pub mod ipc;
#[cfg(target_os = "linux")]
mod linux;
mod unsupported;
#[cfg(windows)]
mod windows;

/// A platform operation failed.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// Not available on this OS.
    #[error("not supported on this operating system")]
    Unsupported,
    /// The process (or session) does not exist.
    #[error("not found")]
    NotFound,
    /// Missing privileges.
    #[error("access denied")]
    AccessDenied,
    /// The PID now belongs to a different process; or (from an event source)
    /// events were lost and a full scan is needed.
    #[error("process changed")]
    Changed,
    /// I/O error.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// Other OS error.
    #[error("{0}")]
    Os(String),
}

/// A new process was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessStart {
    /// Its PID; details come from [`Platform::process_info`].
    pub pid: u32,
}

/// The interactive session of the user at the computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSession {
    /// OS session id (logind session / WTS session).
    pub id: String,
    /// User name.
    pub user: String,
    /// Unix uid (Linux).
    pub uid: Option<u32>,
    /// Environment for processes started in it.
    pub env: Vec<(String, String)>,
}

/// A stream of process starts. An error means the source died (or, for
/// [`PlatformError::Changed`], that events were lost and a full scan is needed).
#[async_trait::async_trait]
pub trait ProcessEventSource: Send {
    /// The next process start.
    async fn next(&mut self) -> Result<ProcessStart, PlatformError>;
}

/// Everything the agent needs from the operating system.
pub trait Platform: Send + Sync + 'static {
    /// Which OS rules apply.
    fn os(&self) -> Os;
    /// All running processes.
    fn list_processes(&self) -> Result<Vec<ProcessInfo>, PlatformError>;
    /// One process.
    fn process_info(&self, pid: u32) -> Result<ProcessInfo, PlatformError>;
    /// Kills `p` only if its PID still has the same name, executable and start
    /// marker (otherwise [`PlatformError::Changed`]).
    fn kill(&self, p: &ProcessInfo) -> Result<(), PlatformError>;
    /// Starts the process-start notification source.
    fn process_events(&self) -> Result<Box<dyn ProcessEventSource>, PlatformError>;
    /// Where protected names are trusted.
    fn trusted_roots(&self) -> TrustedRoots;
    /// Hostname, OS, agent version, LAN addresses.
    fn system_info(&self, agent_version: &str) -> SystemInfo;
    /// The active local graphical session, if a user is logged in.
    fn active_session(&self) -> Result<Option<UserSession>, PlatformError>;
    /// Starts `exe args` as the session's user; returns its PID.
    fn spawn_in_session(
        &self,
        s: &UserSession,
        exe: &Path,
        args: &[String],
    ) -> Result<u32, PlatformError>;
}

/// The platform of the running OS.
pub fn current() -> Arc<dyn Platform> {
    #[cfg(target_os = "linux")]
    {
        Arc::new(linux::Linux)
    }
    #[cfg(windows)]
    {
        Arc::new(windows::Windows)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Arc::new(unsupported::Unsupported)
    }
}

/// LAN addresses: no loopback, no link-local.
pub(crate) fn lan_addresses() -> Vec<String> {
    let Ok(ifs) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    ifs.into_iter()
        .map(|i| i.ip())
        .filter(|ip| !ip.is_loopback())
        .filter(|ip| match ip {
            std::net::IpAddr::V4(v4) => !v4.is_link_local(),
            std::net::IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80,
        })
        .map(|ip| ip.to_string())
        .collect()
}

/// This machine's hostname.
pub(crate) fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}
