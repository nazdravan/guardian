//! Every other OS (e.g. macOS during development): no process control.

use std::path::Path;

use guardian_core::enforce::{Os, ProcessInfo};
use guardian_core::trusted::TrustedRoots;
use guardian_proto::v1::SystemInfo;

use crate::{Platform, PlatformError, ProcessEventSource, UserSession};

/// A platform that supports nothing.
#[cfg_attr(any(target_os = "linux", windows), allow(dead_code))]
#[derive(Debug, Default, Clone, Copy)]
pub struct Unsupported;

impl Platform for Unsupported {
    fn os(&self) -> Os {
        Os::Linux
    }
    fn list_processes(&self) -> Result<Vec<ProcessInfo>, PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn process_info(&self, _pid: u32) -> Result<ProcessInfo, PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn kill(&self, _p: &ProcessInfo) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn process_events(&self) -> Result<Box<dyn ProcessEventSource>, PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn trusted_roots(&self) -> TrustedRoots {
        TrustedRoots::none()
    }
    fn system_info(&self, agent_version: &str) -> SystemInfo {
        SystemInfo {
            hostname: crate::hostname(),
            os: std::env::consts::OS.into(),
            agent_version: agent_version.into(),
            lan_addresses: crate::lan_addresses(),
        }
    }
    fn active_session(&self) -> Result<Option<UserSession>, PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn spawn_in_session(
        &self,
        _s: &UserSession,
        _exe: &Path,
        _args: &[String],
    ) -> Result<u32, PlatformError> {
        Err(PlatformError::Unsupported)
    }
}
