//! Windows-only collection and actions. All via safe APIs (windows-service, winreg,
//! is_elevated, std::process with CREATE_NO_WINDOW); no unsafe in this crate.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use guardian_core::agent;
use guardian_core::read::Read;
use guardian_core::snapshot::{MachineInfo, ServiceInfo, ServiceState, StartType};
use windows_service::service::{ServiceAccess, ServiceStartType, ServiceState as WsState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use winreg::RegKey;
use winreg::enums::HKEY_LOCAL_MACHINE;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn run_hidden(program: &str, args: &[&str]) -> std::io::Result<Output> {
    Command::new(program).args(args).creation_flags(CREATE_NO_WINDOW).output()
}

pub fn wow64() -> bool {
    cfg!(target_pointer_width = "32") && std::env::var_os("PROCESSOR_ARCHITEW6432").is_some()
}

pub fn windir() -> String {
    std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())
}

pub fn io_path(p: &Path) -> PathBuf {
    guardian_core::paths::io_path(p, wow64(), &windir())
}

pub fn image_path() -> Option<String> {
    let key = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(format!(r"SYSTEM\CurrentControlSet\Services\{}", agent::SERVICE_NAME)).ok()?;
    let v: String = key.get_value("ImagePath").ok()?;
    Some(guardian_core::paths::unquote_image_path(&v))
}

fn win_err<T>(e: windows_service::Error) -> Read<T> {
    if let windows_service::Error::Winapi(io) = &e {
        match io.raw_os_error() {
            Some(1060) => return Read::Missing, // ERROR_SERVICE_DOES_NOT_EXIST
            Some(5) => return Read::Denied,
            _ => {}
        }
    }
    Read::Failed(e.to_string())
}

pub fn service_info() -> Read<ServiceInfo> {
    let mgr = match ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT) {
        Ok(m) => m,
        Err(e) => return win_err(e),
    };
    let svc = match mgr.open_service(agent::SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG) {
        Ok(s) => s,
        Err(e) => return win_err(e),
    };
    let state = match svc.query_status() {
        Ok(st) => match st.current_state {
            WsState::Running => ServiceState::Running,
            WsState::Stopped => ServiceState::Stopped,
            WsState::StartPending | WsState::StopPending | WsState::ContinuePending | WsState::PausePending => ServiceState::Pending,
            _ => ServiceState::Other,
        },
        Err(e) => return win_err(e),
    };
    let start_type = match svc.query_config() {
        Ok(c) => match c.start_type {
            ServiceStartType::AutoStart => StartType::Auto,
            ServiceStartType::OnDemand => StartType::Manual,
            ServiceStartType::Disabled => StartType::Disabled,
            _ => StartType::Other,
        },
        Err(e) => return win_err(e),
    };
    let image = image_path().unwrap_or_default();
    let exe_exists = !image.is_empty() && io_path(Path::new(&image)).is_file();
    Read::Ok(ServiceInfo { state, start_type, image_path: image, exe_exists })
}

pub fn agent_version(image: &str) -> Read<String> {
    if image.is_empty() {
        return Read::Missing;
    }
    let exe = io_path(Path::new(image));
    match Command::new(&exe).arg("-version").creation_flags(CREATE_NO_WINDOW).output() {
        Ok(o) if o.status.success() => Read::Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        Ok(o) => Read::Failed(format!("exit {}", o.status)),
        Err(e) => crate::files::from_io(e),
    }
}

pub fn tasklist() -> Read<Vec<String>> {
    match run_hidden("tasklist", &["/FO", "CSV", "/NH"]) {
        Ok(o) if o.status.success() => Read::Ok(guardian_core::parse::tasklist(&crate::text::decode_output(&o.stdout))),
        Ok(o) => Read::Failed(crate::text::decode_output(&o.stderr).trim().to_string()),
        Err(e) => Read::Failed(e.to_string()),
    }
}

pub fn events() -> Read<Vec<guardian_core::parse::AgentEvent>> {
    let q = format!("/q:*[System[Provider[@Name='{}']]]", agent::SERVICE_NAME);
    match run_hidden("wevtutil", &["qe", "Application", &q, "/c:2000", "/rd:true", "/f:xml"]) {
        Ok(o) if o.status.success() => guardian_core::parse::wevtutil(&crate::text::decode_output(&o.stdout)).map_or_else(Read::Failed, Read::Ok),
        Ok(o) => {
            let err = crate::text::decode_output(&o.stderr);
            if err.to_lowercase().contains("access is denied") { Read::Denied } else { Read::Failed(err.trim().to_string()) }
        }
        Err(e) => Read::Failed(e.to_string()),
    }
}

pub fn machine() -> MachineInfo {
    let cv = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion").ok();
    let get = |k: &str| cv.as_ref().and_then(|c| c.get_value::<String, _>(k).ok()).unwrap_or_default();
    let os = format!("{} {} (build {})", get("ProductName"), get("DisplayVersion"), get("CurrentBuild")).trim().to_string();
    let arch = std::env::var("PROCESSOR_ARCHITEW6432").or_else(|_| std::env::var("PROCESSOR_ARCHITECTURE")).unwrap_or_default();
    MachineInfo { hostname: std::env::var("COMPUTERNAME").unwrap_or_default(), os, arch }
}

pub fn is_elevated() -> bool {
    is_elevated::is_elevated()
}

/// Re-launch this exe with UAC (spec R3). Ok(()) once the elevated process has started;
/// Err when the user cancelled UAC or PowerShell is unavailable.
pub fn relaunch_elevated(args: &[String]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let q = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let arglist = if args.is_empty() { String::new() } else { format!(" -ArgumentList {}", args.iter().map(|a| q(a)).collect::<Vec<_>>().join(",")) };
    let script = format!("Start-Process -FilePath {}{} -Verb RunAs", q(&exe.to_string_lossy()), arglist);
    match run_hidden("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script]) {
        Ok(o) if o.status.success() => Ok(()),
        Ok(_) => Err("Elevation was cancelled".into()),
        Err(e) => Err(e.to_string()),
    }
}

fn wait_for(svc: &windows_service::service::Service, want: WsState, limit: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        let st = svc.query_status().map_err(|e| e.to_string())?;
        if st.current_state == want {
            return Ok(());
        }
        if start.elapsed() > limit {
            return Err(format!("the service is still {:?} after {} s", st.current_state, limit.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Stop + start with 30 s per phase (spec R14.5).
pub fn restart_service(progress: &dyn Fn(&str)) -> Result<(), String> {
    let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(|e| e.to_string())?;
    let svc =
        mgr.open_service(agent::SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::START).map_err(|e| e.to_string())?;
    let limit = Duration::from_secs(30);
    if svc.query_status().map_err(|e| e.to_string())?.current_state != WsState::Stopped {
        progress("Stopping ProcSentinelAgent…");
        svc.stop().map_err(|e| e.to_string())?;
        wait_for(&svc, WsState::Stopped, limit)?;
    }
    progress("Starting ProcSentinelAgent…");
    svc.start::<&str>(&[]).map_err(|e| e.to_string())?;
    wait_for(&svc, WsState::Running, limit)
}
