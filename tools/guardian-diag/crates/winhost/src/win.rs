//! Windows-only collection and actions. All via safe APIs (windows-service, winreg,
//! is_elevated, std::process with CREATE_NO_WINDOW); no unsafe in this crate.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use guardian_core::agent;
use guardian_core::read::Read;
use guardian_core::snapshot::{MachineInfo, ServiceInfo, ServiceState, StartType};
use windows_service::service::{ServiceAccess, ServiceStartType, ServiceState as WsState};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use winreg::RegKey;
use winreg::enums::HKEY_LOCAL_MACHINE;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Absolute path of a Windows tool under `%WINDIR%\System32` (SysWOW64 for a 32-bit process on
/// 64-bit Windows, which has the same tools). A bare name would make `Command` search the exe's
/// own folder first, so a `tasklist.exe` planted beside Guardian.exe would run — elevated, too.
pub fn system32(rel: &str) -> String {
    format!(r"{}\System32\{rel}", windir().trim_end_matches('\\'))
}

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

/// `Error::Winapi` displays only "IO error in winapi call"; the cause is its source.
fn ws_err(e: windows_service::Error) -> String {
    match std::error::Error::source(&e) {
        Some(src) => match src.downcast_ref::<std::io::Error>().and_then(|io| io.raw_os_error()) {
            Some(code) => format!("{e}: {src} (os error {code})"),
            None => format!("{e}: {src}"),
        },
        None => e.to_string(),
    }
}

fn win_err<T>(e: windows_service::Error) -> Read<T> {
    if let windows_service::Error::Winapi(io) = &e {
        match io.raw_os_error() {
            Some(1060) => return Read::Missing, // ERROR_SERVICE_DOES_NOT_EXIST
            Some(5) => return Read::Denied,
            _ => {}
        }
    }
    Read::Failed(ws_err(e))
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
    let mut child = match Command::new(&exe)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return crate::files::from_io(e),
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Read::Failed("no output".into());
    };
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut buf);
        buf
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if start.elapsed() > Duration::from_secs(5) => {
                let _ = child.kill();
                let _ = child.wait();
                return Read::Failed("timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Read::Failed(e.to_string()),
        }
    };
    let out = reader.join().unwrap_or_default();
    if !status.success() {
        return Read::Failed(format!("exit {status}"));
    }
    let text = String::from_utf8_lossy(&out).trim().to_string();
    if text.is_empty() { Read::Failed("no output".into()) } else { Read::Ok(text) }
}

pub fn tasklist() -> Read<Vec<String>> {
    match run_hidden(&system32("tasklist.exe"), &["/FO", "CSV", "/NH"]) {
        Ok(o) if o.status.success() => Read::Ok(guardian_core::parse::tasklist(&crate::text::decode_output(&o.stdout))),
        Ok(o) => Read::Failed(crate::text::decode_output(&o.stderr).trim().to_string()),
        Err(e) => Read::Failed(e.to_string()),
    }
}

pub fn events() -> Read<Vec<guardian_core::parse::AgentEvent>> {
    let q = format!("/q:*[System[Provider[@Name='{}']]]", agent::SERVICE_NAME);
    match run_hidden(&system32("wevtutil.exe"), &["qe", "Application", &q, "/c:2000", "/rd:true", "/f:xml"]) {
        Ok(o) if o.status.success() => guardian_core::parse::wevtutil(&crate::text::decode_output(&o.stdout)).map_or_else(Read::Failed, Read::Ok),
        Ok(o) => {
            let err = crate::text::decode_output(&o.stderr);
            if o.status.code() == Some(5) || err.to_lowercase().contains("access is denied") {
                Read::Denied
            } else {
                Read::Failed(err.trim().to_string())
            }
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
    let script = crate::text::runas_script(&exe.to_string_lossy(), args);
    match run_hidden(&system32(r"WindowsPowerShell\v1.0\powershell.exe"), &["-NoProfile", "-NonInteractive", "-Command", &script]) {
        Ok(o) if o.status.success() => Ok(()),
        Ok(_) => Err("Elevation was cancelled".into()),
        Err(e) => Err(e.to_string()),
    }
}

fn state_name(s: WsState) -> &'static str {
    match s {
        WsState::Stopped => "Stopped",
        WsState::StartPending => "StartPending",
        WsState::StopPending => "StopPending",
        WsState::Running => "Running",
        WsState::ContinuePending => "ContinuePending",
        WsState::PausePending => "PausePending",
        WsState::Paused => "Paused",
    }
}

fn wait_for(svc: &windows_service::service::Service, want: WsState, limit: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        let st = svc.query_status().map_err(ws_err)?;
        if st.current_state == want {
            return Ok(());
        }
        if start.elapsed() > limit {
            return Err(format!(
                "the service is still {} after {} s (waiting for {})",
                state_name(st.current_state),
                limit.as_secs(),
                state_name(want)
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Bring the service to Running from whatever state it is in, 30 s per phase (spec R14.5).
pub fn restart_service(progress: &dyn Fn(&str)) -> Result<(), String> {
    let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(ws_err)?;
    let svc = mgr.open_service(agent::SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::START).map_err(ws_err)?;
    let limit = Duration::from_secs(30);
    let current = svc.query_status().map_err(ws_err)?.current_state;
    match current {
        WsState::StartPending => return wait_for(&svc, WsState::Running, limit),
        WsState::Stopped => {}
        WsState::StopPending => {
            progress("Waiting for ProcSentinelAgent to stop\u{2026}");
            wait_for(&svc, WsState::Stopped, limit)?;
        }
        _ => {
            progress("Stopping ProcSentinelAgent\u{2026}");
            svc.stop().map_err(ws_err)?;
            wait_for(&svc, WsState::Stopped, limit)?;
        }
    }
    progress("Starting ProcSentinelAgent\u{2026}");
    svc.start::<&str>(&[]).map_err(ws_err)?;
    wait_for(&svc, WsState::Running, limit)
}
