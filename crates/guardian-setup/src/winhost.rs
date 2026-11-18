//! The real Windows host: the service manager (`windows-service`), `netsh`
//! for the firewall, the installed agent as a child process, and the
//! process list from `guardian-platform`.

use std::ffi::OsStr;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use guardian_core::enforce::ProcessInfo;
use guardian_core::trusted::TrustedRoots;
use windows_service::service::{ServiceAccess, ServiceState as WinState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::host::{Host, HostError, RunOutput, ServiceState};
use crate::model::{WinPath, firewall_args, sysnative};

/// No console window for child processes.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;
const ERROR_SERVICE_ALREADY_RUNNING: i32 = 1056;
const ERROR_SERVICE_NOT_ACTIVE: i32 = 1062;
const ERROR_ACCESS_DENIED: i32 = 5;

/// The host this process runs on.
#[derive(Debug, Default, Clone, Copy)]
pub struct WinHost;

fn win_err(e: windows_service::Error) -> HostError {
    if let windows_service::Error::Winapi(io) = &e
        && io.raw_os_error() == Some(ERROR_ACCESS_DENIED)
    {
        return HostError::AccessDenied;
    }
    HostError::Failed(e.to_string())
}

fn os_code(e: &windows_service::Error) -> Option<i32> {
    match e {
        windows_service::Error::Winapi(io) => io.raw_os_error(),
        _ => None,
    }
}

fn state(s: WinState) -> ServiceState {
    match s {
        WinState::Running => ServiceState::Running,
        WinState::Stopped => ServiceState::Stopped,
        WinState::StartPending => ServiceState::Starting,
        WinState::StopPending => ServiceState::Stopping,
        _ => ServiceState::Other,
    }
}

impl WinHost {
    fn system_root(&self) -> WinPath {
        WinPath::new(
            &self
                .env("SystemRoot")
                .unwrap_or_else(|| r"C:\Windows".into()),
        )
    }

    fn netsh(&self) -> PathBuf {
        PathBuf::from(
            self.system_root()
                .join("System32")
                .join("netsh.exe")
                .as_str(),
        )
    }

    fn netsh_run(&self, args: &[String]) -> Result<RunOutput, HostError> {
        self.run(&self.netsh(), args)
    }

    fn wait_for(&self, name: &str, want: ServiceState, timeout: Duration) -> Result<(), HostError> {
        let end = Instant::now() + timeout;
        loop {
            match self.service(name)? {
                Some(s) if s == want => return Ok(()),
                None => return Err(HostError::Failed(format!("{name} is not installed"))),
                Some(s) if Instant::now() >= end => {
                    return Err(HostError::Failed(format!(
                        "{name} is still {s:?} after {}s",
                        timeout.as_secs()
                    )));
                }
                Some(_) => std::thread::sleep(Duration::from_millis(250)),
            }
        }
    }
}

impl Host for WinHost {
    fn env(&self, var: &str) -> Option<String> {
        std::env::var(var).ok().filter(|v| !v.is_empty())
    }

    fn is_elevated(&self) -> bool {
        guardian_platform::windows_util::is_elevated()
    }

    fn setup_dir(&self) -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn io_path(&self, canonical: &WinPath) -> PathBuf {
        let wow64 = self.env("PROCESSOR_ARCHITEW6432").is_some();
        PathBuf::from(sysnative(canonical, &self.system_root(), wow64).as_str())
    }

    fn service(&self, name: &str) -> Result<Option<ServiceState>, HostError> {
        let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(win_err)?;
        match mgr.open_service(name, ServiceAccess::QUERY_STATUS) {
            Ok(svc) => Ok(Some(state(
                svc.query_status().map_err(win_err)?.current_state,
            ))),
            Err(e) if os_code(&e) == Some(ERROR_SERVICE_DOES_NOT_EXIST) => Ok(None),
            Err(e) => Err(win_err(e)),
        }
    }

    fn start_service(&self, name: &str, timeout: Duration) -> Result<(), HostError> {
        let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(win_err)?;
        let svc = mgr
            .open_service(name, ServiceAccess::START | ServiceAccess::QUERY_STATUS)
            .map_err(win_err)?;
        match svc.start::<&OsStr>(&[]) {
            Ok(()) => {}
            Err(e) if os_code(&e) == Some(ERROR_SERVICE_ALREADY_RUNNING) => {}
            Err(e) => return Err(win_err(e)),
        }
        self.wait_for(name, ServiceState::Running, timeout)
    }

    fn stop_service(&self, name: &str, timeout: Duration) -> Result<(), HostError> {
        let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(win_err)?;
        let svc = mgr
            .open_service(name, ServiceAccess::STOP | ServiceAccess::QUERY_STATUS)
            .map_err(win_err)?;
        match svc.stop() {
            Ok(_) => {}
            Err(e) if os_code(&e) == Some(ERROR_SERVICE_NOT_ACTIVE) => {}
            Err(e) => return Err(win_err(e)),
        }
        self.wait_for(name, ServiceState::Stopped, timeout)
    }

    fn delete_service(&self, name: &str) -> Result<(), HostError> {
        let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(win_err)?;
        let svc = mgr
            .open_service(name, ServiceAccess::DELETE)
            .map_err(win_err)?;
        svc.delete().map_err(win_err)
    }

    fn run(&self, exe: &Path, args: &[String]) -> Result<RunOutput, HostError> {
        let out = Command::new(exe)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| HostError::Failed(format!("{}: {e}", exe.display())))?;
        Ok(RunOutput {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    fn processes(&self) -> Result<Vec<ProcessInfo>, HostError> {
        guardian_platform::current()
            .list_processes()
            .map_err(|e| HostError::Failed(e.to_string()))
    }

    fn trusted_roots(&self) -> TrustedRoots {
        guardian_platform::current().trusted_roots()
    }

    fn firewall_rule(&self, name: &str) -> Result<bool, HostError> {
        let out = self.netsh_run(&[
            "advfirewall".into(),
            "firewall".into(),
            "show".into(),
            "rule".into(),
            format!("name={name}"),
        ])?;
        Ok(out.code == 0)
    }

    fn set_firewall_rule(&self, name: &str, port: u16) -> Result<(), HostError> {
        self.delete_firewall_rule(name)?;
        for proto in ["TCP", "UDP"] {
            let out = self.netsh_run(&firewall_args(name, port, proto))?;
            if out.code != 0 {
                return Err(HostError::Failed(format!(
                    "netsh could not add the {proto} rule: {}",
                    out.stdout.trim()
                )));
            }
        }
        Ok(())
    }

    fn delete_firewall_rule(&self, name: &str) -> Result<(), HostError> {
        // Exit 1 with "No rules match" when there is none: that is fine.
        let _ = self.netsh_run(&[
            "advfirewall".into(),
            "firewall".into(),
            "delete".into(),
            "rule".into(),
            format!("name={name}"),
        ])?;
        Ok(())
    }

    fn wait(&self, d: Duration) {
        std::thread::sleep(d);
    }
}
