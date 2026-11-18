//! Everything the setup needs from Windows, behind one trait: the real
//! `WinHost` and, for tests, `FakeHost`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use guardian_core::enforce::ProcessInfo;
use guardian_core::trusted::TrustedRoots;
use serde::Serialize;

use crate::model::WinPath;

/// A Windows service's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceState {
    /// Running.
    Running,
    /// Stopped.
    Stopped,
    /// Start pending.
    Starting,
    /// Stop pending.
    Stopping,
    /// Paused or anything else.
    Other,
}

/// A host call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    /// Windows said no (not elevated, or locked down).
    #[error("access denied")]
    AccessDenied,
    /// Anything else, in Windows' words.
    #[error("{0}")]
    Failed(String),
}

/// What a program printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunOutput {
    /// Exit code (-1 when it was killed).
    pub code: i32,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
}

/// What touches the OS.
pub trait Host: Send + Sync {
    /// An environment variable.
    fn env(&self, var: &str) -> Option<String>;
    /// Running with administrator rights.
    fn is_elevated(&self) -> bool;
    /// The folder `guardian-setup.exe` runs from (with `bin\x64`, `bin\x86`).
    fn setup_dir(&self) -> PathBuf;
    /// The real path for a canonical Windows path (`System32` becomes
    /// `Sysnative` for this 32-bit process on 64-bit Windows, only here).
    fn io_path(&self, canonical: &WinPath) -> PathBuf;
    /// A service's state; `None` when it is not registered.
    fn service(&self, name: &str) -> Result<Option<ServiceState>, HostError>;
    /// Starts a service and waits until it runs.
    fn start_service(&self, name: &str, timeout: Duration) -> Result<(), HostError>;
    /// Stops a service and waits until it stopped.
    fn stop_service(&self, name: &str, timeout: Duration) -> Result<(), HostError>;
    /// Removes a (stopped) service.
    fn delete_service(&self, name: &str) -> Result<(), HostError>;
    /// Runs a program and waits for it; `env` is added to its environment
    /// (secrets go there, never on the command line).
    fn run(
        &self,
        exe: &Path,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<RunOutput, HostError>;
    /// The running processes.
    fn processes(&self) -> Result<Vec<ProcessInfo>, HostError>;
    /// Where protected system programs live.
    fn trusted_roots(&self) -> TrustedRoots;
    /// Whether the firewall rule exists.
    fn firewall_rule(&self, name: &str) -> Result<bool, HostError>;
    /// Adds the rule (TCP and UDP on `port`, local subnet), replacing an old one.
    fn set_firewall_rule(&self, name: &str, port: u16) -> Result<(), HostError>;
    /// Removes the rule (missing is fine).
    fn delete_firewall_rule(&self, name: &str) -> Result<(), HostError>;
    /// Waits (a no-op in tests).
    fn wait(&self, d: Duration);
}

/// A host for other systems: it reads the environment and refuses the rest
/// (the setup is for Windows; this keeps `--help` and `version` working).
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedHost;

fn unsupported() -> HostError {
    HostError::Failed("guardian-setup manages the agent on Windows only".into())
}

impl Host for UnsupportedHost {
    fn env(&self, var: &str) -> Option<String> {
        std::env::var(var).ok()
    }
    fn is_elevated(&self) -> bool {
        false
    }
    fn setup_dir(&self) -> PathBuf {
        PathBuf::from(".")
    }
    fn io_path(&self, canonical: &WinPath) -> PathBuf {
        PathBuf::from(canonical.as_str())
    }
    fn service(&self, _name: &str) -> Result<Option<ServiceState>, HostError> {
        Err(unsupported())
    }
    fn start_service(&self, _name: &str, _timeout: Duration) -> Result<(), HostError> {
        Err(unsupported())
    }
    fn stop_service(&self, _name: &str, _timeout: Duration) -> Result<(), HostError> {
        Err(unsupported())
    }
    fn delete_service(&self, _name: &str) -> Result<(), HostError> {
        Err(unsupported())
    }
    fn run(
        &self,
        _exe: &Path,
        _args: &[String],
        _env: &[(String, String)],
    ) -> Result<RunOutput, HostError> {
        Err(unsupported())
    }
    fn processes(&self) -> Result<Vec<ProcessInfo>, HostError> {
        Err(unsupported())
    }
    fn trusted_roots(&self) -> TrustedRoots {
        TrustedRoots::none()
    }
    fn firewall_rule(&self, _name: &str) -> Result<bool, HostError> {
        Err(unsupported())
    }
    fn set_firewall_rule(&self, _name: &str, _port: u16) -> Result<(), HostError> {
        Err(unsupported())
    }
    fn delete_firewall_rule(&self, _name: &str) -> Result<(), HostError> {
        Err(unsupported())
    }
    fn wait(&self, d: Duration) {
        std::thread::sleep(d);
    }
}
