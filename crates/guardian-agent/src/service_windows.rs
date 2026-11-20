//! Running `guardian-agent` as the Windows service `GuardianAgent`.

use std::ffi::OsString;
use std::process::{Command, ExitCode};
use std::sync::OnceLock;
use std::time::Duration;

use clap::Subcommand;
use windows_service::service::{
    ServiceAccess, ServiceAction as ScAction, ServiceActionType, ServiceControl,
    ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceFailureActions,
    ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus,
    ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::{Cli, Cmd};

/// Service name.
pub const SERVICE_NAME: &str = "GuardianAgent";
const DISPLAY_NAME: &str = "Guardian Agent";

/// What to do with the service.
#[derive(Debug, Clone, Copy, Subcommand)]
pub enum ServiceAction {
    /// Register the service (automatic start, LocalSystem, restart on failure)
    /// with the current options, protect the data directory, and start it.
    Install,
    /// Stop and remove the service.
    Uninstall,
    /// Run as the service (used by the Service Control Manager).
    Run,
}

static CLI: OnceLock<Cli> = OnceLock::new();

/// Whether this invocation is `service run` (it logs to a file).
pub fn is_run(cli: &Cli) -> bool {
    matches!(
        cli.command,
        Cmd::Service {
            action: ServiceAction::Run
        }
    )
}

fn icacls(dir: &std::path::Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new("icacls")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("icacls: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "icacls {}: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stdout).trim()
        ))
    }
}

/// SYSTEM and Administrators only: the directory holds the agent's private key,
/// its state and the local whitelist. Users can create folders in
/// `%ProgramData%`, so the owner is reset to Administrators first — an owner can
/// always rewrite the DACL. SIDs avoid localized group names.
pub fn protect_dir(dir: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    icacls(dir, &["/setowner", "*S-1-5-32-544", "/T", "/C", "/Q"])?;
    let out = Command::new("icacls")
        .arg(dir)
        .args(guardian_platform::protected_acl_args(&[]))
        .output()
        .map_err(|e| format!("icacls: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "icacls {}: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stdout).trim()
        ))
    }
}

/// Runs a `service` subcommand.
pub fn dispatch(action: ServiceAction, cli: &Cli) -> ExitCode {
    let r = match action {
        ServiceAction::Install => install(cli),
        ServiceAction::Uninstall => uninstall(),
        ServiceAction::Run => {
            let _ = CLI.set(cli.clone());
            ffi::start().map_err(|e| e.to_string())
        }
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn service_args(cli: &Cli) -> Vec<OsString> {
    let p = cli.paths();
    vec![
        "--config".into(),
        p.config.into_os_string(),
        "--data-dir".into(),
        p.data.into_os_string(),
        "--log".into(),
        cli.log.clone().into(),
        "service".into(),
        "run".into(),
    ]
}

fn install(cli: &Cli) -> Result<(), String> {
    let paths = cli.paths();
    match guardian_agent::config::AgentConfig::load(&paths.config) {
        Ok(c) if c.computer_id.is_some() => {}
        _ => return Err("not enrolled: run `guardian-agent enroll` first".into()),
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    protect_dir(&paths.data)?;
    if let Some(dir) = paths.config.parent()
        && dir != paths.data
    {
        protect_dir(dir)?;
    }
    let mgr = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(|e| format!("open service manager (run as administrator): {e}"))?;
    let info = ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: service_args(cli),
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let svc = mgr
        .create_service(&info, ServiceAccess::START | ServiceAccess::CHANGE_CONFIG)
        .map_err(|e| format!("create service: {e}"))?;
    let _ = svc.set_description("Guardian 3.0 agent (enforces this computer's profile)");
    let restart = || ScAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(5),
    };
    svc.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 3600)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart(), restart(), restart()]),
    })
    .map_err(|e| format!("set recovery actions: {e}"))?;
    let _ = svc.set_failure_actions_on_non_crash_failures(true);
    svc.start::<&str>(&[])
        .map_err(|e| format!("start service: {e}"))?;
    println!("Service {SERVICE_NAME} installed and started.");
    println!("Log: {}", paths.log().display());
    Ok(())
}

fn uninstall() -> Result<(), String> {
    let mgr = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| format!("open service manager (run as administrator): {e}"))?;
    let svc = mgr
        .open_service(
            SERVICE_NAME,
            ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
        )
        .map_err(|e| format!("open service: {e}"))?;
    if let Ok(st) = svc.query_status()
        && st.current_state != ServiceState::Stopped
    {
        let _ = svc.stop();
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(200));
            if svc
                .query_status()
                .is_ok_and(|s| s.current_state == ServiceState::Stopped)
            {
                break;
            }
        }
    }
    svc.delete().map_err(|e| format!("delete service: {e}"))?;
    println!("Service {SERVICE_NAME} removed.");
    Ok(())
}

// The Windows service entry point needs an `extern "system"` trampoline that
// the crate's macro writes with one `unsafe` block (argument parsing).
#[allow(unsafe_code)]
mod ffi {
    windows_service::define_windows_service!(ffi_service_main, service_main);

    fn service_main(_args: Vec<std::ffi::OsString>) {
        super::service_main();
    }

    /// Hands this thread to the Service Control Manager until the service stops.
    pub(super) fn start() -> windows_service::Result<()> {
        windows_service::service_dispatcher::start(super::SERVICE_NAME, ffi_service_main)
    }
}

fn status(
    state: ServiceState,
    accept: ServiceControlAccept,
    exit: ServiceExitCode,
) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: exit,
        checkpoint: 0,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    }
}

fn service_main() {
    let Some(cli) = CLI.get().cloned() else {
        return;
    };
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let tx = std::sync::Mutex::new(Some(tx));
    let handler = move |c| match c {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            if let Ok(mut g) = tx.lock()
                && let Some(t) = g.take()
            {
                let _ = t.send(());
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let Ok(h) = service_control_handler::register(SERVICE_NAME, handler) else {
        return;
    };
    let ok = ServiceExitCode::Win32(0);
    let _ = h.set_service_status(status(
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        ok,
    ));
    // Running is reported only once the saved state is enforced.
    let ready = || {
        let _ = h.set_service_status(status(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            ok,
        ));
    };
    let result = tokio::runtime::Runtime::new()
        .map_err(|e| e.to_string())
        .and_then(|rt| {
            rt.block_on(crate::run_until(&cli, ready, async {
                let _ = rx.await;
            }))
            .map_err(|e| e.to_string())
        });
    let exit = match result {
        Ok(()) => ok,
        Err(e) => {
            tracing::error!("{e}");
            ServiceExitCode::ServiceSpecific(1)
        }
    };
    let _ = h.set_service_status(status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        exit,
    ));
}
