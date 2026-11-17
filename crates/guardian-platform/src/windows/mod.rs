//! Windows: Toolhelp and process handles, ETW, WTS sessions.

use std::path::{Path, PathBuf};

use guardian_core::enforce::{Os, ProcessInfo};
use guardian_core::trusted::TrustedRoots;
use guardian_proto::v1::SystemInfo;

use crate::{Platform, PlatformError, ProcessEventSource, UserSession};

mod ffi_procs;
mod procs;

/// The Windows platform.
#[derive(Debug, Default, Clone, Copy)]
pub struct Windows;

fn env_dir(var: &str, default: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(default), PathBuf::from)
}

/// Trusted locations, from `%SystemRoot%`, `%ProgramFiles%` and `%ProgramData%`.
pub fn trusted_roots_from(
    system_root: &Path,
    program_files: &Path,
    program_data: &Path,
) -> TrustedRoots {
    let sr = |p: &str| system_root.join(p);
    let pf = |p: &str| program_files.join(p);
    TrustedRoots {
        recursive: vec![
            sr("System32"),
            sr("SysWOW64"),
            sr("SystemApps"),
            sr("ImmersiveControlPanel"),
            pf("WindowsApps"),
            pf("PowerShell"),
            pf("Windows Defender"),
            program_data.join(r"Microsoft\Windows Defender\Platform"),
            pf("Guardian"),
        ],
        direct_files: vec![system_root.to_path_buf()],
        // Places where ordinary users can create files.
        excluded: vec![
            sr(r"System32\spool"),
            sr(r"System32\Tasks"),
            sr(r"System32\com\dmp"),
            sr(r"System32\Microsoft\Crypto"),
            sr(r"SysWOW64\Tasks"),
            sr(r"SysWOW64\com\dmp"),
            sr("Temp"),
            sr("Tracing"),
        ],
    }
}

impl Platform for Windows {
    fn os(&self) -> Os {
        Os::Windows
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
        Err(PlatformError::Unsupported)
    }

    fn trusted_roots(&self) -> TrustedRoots {
        trusted_roots_from(
            &env_dir("SystemRoot", r"C:\Windows"),
            &env_dir("ProgramFiles", r"C:\Program Files"),
            &env_dir("ProgramData", r"C:\ProgramData"),
        )
    }

    fn system_info(&self, agent_version: &str) -> SystemInfo {
        SystemInfo {
            hostname: crate::hostname(),
            os: "windows".into(),
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
