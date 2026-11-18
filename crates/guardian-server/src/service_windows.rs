//! Running `guardian-server` as the Windows service `GuardianServer`.

use std::ffi::OsString;
use std::process::{Command, ExitCode};
use std::sync::OnceLock;
use std::time::Duration;

use clap::Subcommand;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

use crate::Cli;

/// Service name.
pub const SERVICE_NAME: &str = "GuardianServer";
const DISPLAY_NAME: &str = "Guardian Server";
const FIREWALL_RULE: &str = "Guardian Server";

/// `guardian-server service …`
#[derive(Debug, Clone, Subcommand)]
pub enum ServiceCmd {
    /// Manage the Windows service.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

/// What to do with the service.
#[derive(Debug, Clone, Copy, Subcommand)]
pub enum ServiceAction {
    /// Register the service (automatic start) with the current options, open the
    /// firewall port, and start it.
    Install,
    /// Stop and remove the service and its firewall rule.
    Uninstall,
    /// Run as the service (used by the Service Control Manager).
    Run,
}

static CLI: OnceLock<Cli> = OnceLock::new();

/// Whether this invocation is `service run` (it logs to a file).
pub fn is_run(cli: &Cli) -> bool {
    matches!(
        cli.command,
        Some(ServiceCmd::Service {
            action: ServiceAction::Run
        })
    )
}

/// Gives the data directory a protected ACL: SYSTEM and Administrators only
/// (it holds the TLS private key and the database). SIDs avoid localized names.
fn protect_data_dir(dir: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let out = Command::new("icacls")
        .arg(dir)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "/grant:r",
            "*S-1-5-32-544:(OI)(CI)F",
            "/T",
            "/C",
            "/Q",
        ])
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
pub fn dispatch(cmd: &ServiceCmd, cli: &Cli) -> ExitCode {
    let ServiceCmd::Service { action } = cmd;
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
    let mut a: Vec<OsString> = Vec::new();
    let c = cli.config();
    a.push("--data-dir".into());
    a.push(c.data_dir.into_os_string());
    a.push("--listen".into());
    a.push(cli.listen.to_string().into());
    if let (Some(cert), Some(key)) = (&cli.tls_cert, &cli.tls_key) {
        a.extend(["--tls-cert".into(), cert.clone().into_os_string()]);
        a.extend(["--tls-key".into(), key.clone().into_os_string()]);
    }
    if cli.insecure_plaintext {
        a.push("--insecure-plaintext".into());
    }
    a.extend(["--log".into(), cli.log.clone().into()]);
    a.extend(["service".into(), "run".into()]);
    a
}

fn netsh(args: &[String]) {
    match Command::new("netsh").args(args).output() {
        Ok(o) if o.status.success() => {}
        Ok(o) => eprintln!("netsh: {}", String::from_utf8_lossy(&o.stdout).trim()),
        Err(e) => eprintln!("netsh: {e}"),
    }
}

fn firewall_delete() {
    netsh(&[
        "advfirewall".into(),
        "firewall".into(),
        "delete".into(),
        "rule".into(),
        format!("name={FIREWALL_RULE}"),
    ]);
}

fn install(cli: &Cli) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let data_dir = cli.config().data_dir;
    protect_data_dir(&data_dir)?;
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
    let _ = svc.set_description("Guardian 3.0 server (gRPC + agent WebSocket)");
    firewall_delete();
    netsh(&[
        "advfirewall".into(),
        "firewall".into(),
        "add".into(),
        "rule".into(),
        format!("name={FIREWALL_RULE}"),
        "dir=in".into(),
        "action=allow".into(),
        "protocol=TCP".into(),
        format!("localport={}", cli.listen.port()),
    ]);
    svc.start::<&str>(&[])
        .map_err(|e| format!("start service: {e}"))?;
    println!("Service {SERVICE_NAME} installed and started.");
    println!(
        "The certificate SHA-256 and the bootstrap code are in {}",
        data_dir.join("guardian-server.log").display()
    );
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
    firewall_delete();
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

fn status(state: ServiceState, accept: ServiceControlAccept) -> ServiceStatus {
    status_with(state, accept, ServiceExitCode::Win32(0))
}

fn status_with(
    state: ServiceState,
    accept: ServiceControlAccept,
    exit_code: ServiceExitCode,
) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code,
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
    let _ = h.set_service_status(status(
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
    ));
    // Running is reported only once the port is bound.
    let ready = || {
        let _ = h.set_service_status(status(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        ));
    };
    let result = tokio::runtime::Runtime::new()
        .map_err(|e| e.to_string())
        .and_then(|rt| {
            rt.block_on(crate::run_until(&cli, ready, async {
                let _ = rx.await;
            }))
        });
    let exit = match result {
        Ok(()) => ServiceExitCode::Win32(0),
        Err(e) => {
            tracing::error!("{e}");
            ServiceExitCode::ServiceSpecific(1)
        }
    };
    let _ = h.set_service_status(status_with(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        exit,
    ));
}
