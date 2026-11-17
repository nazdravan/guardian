//! Linux: `/proc`, signals, the proc connector and `loginctl`.

use std::path::Path;

use guardian_core::enforce::{Os, ProcessInfo};
use guardian_core::trusted::TrustedRoots;
use guardian_proto::v1::SystemInfo;

use crate::{Platform, PlatformError, ProcessEventSource, UserSession};

mod events;
mod procs;

/// The Linux platform.
#[derive(Debug, Default, Clone, Copy)]
pub struct Linux;

impl Platform for Linux {
    fn os(&self) -> Os {
        Os::Linux
    }

    fn list_processes(&self) -> Result<Vec<ProcessInfo>, PlatformError> {
        procs::list()
    }

    fn process_info(&self, pid: u32) -> Result<ProcessInfo, PlatformError> {
        procs::info(pid)
    }

    fn kill(&self, p: &ProcessInfo) -> Result<(), PlatformError> {
        procs::kill_checked(p)
    }

    fn process_events(&self) -> Result<Box<dyn ProcessEventSource>, PlatformError> {
        Ok(Box::new(events::ProcConnector::start()?))
    }

    fn trusted_roots(&self) -> TrustedRoots {
        TrustedRoots {
            recursive: [
                "/usr",
                "/bin",
                "/sbin",
                "/lib",
                "/lib64",
                "/opt/guardian-agent",
            ]
            .iter()
            .map(Into::into)
            .collect(),
            ..TrustedRoots::none()
        }
    }

    fn system_info(&self, agent_version: &str) -> SystemInfo {
        SystemInfo {
            hostname: crate::hostname(),
            os: "linux".into(),
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
