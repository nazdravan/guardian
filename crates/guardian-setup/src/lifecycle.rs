//! The setup's operations on the agent: install, update, uninstall, re-enroll
//! and the status. Everything goes through `Host`, so all of it is tested on
//! the fake.

use std::path::{Path, PathBuf};
use std::time::Duration;

use guardian_agent::config::AgentConfig;
use guardian_core::local::DEFAULT_LOCAL_PORT;
use serde::Serialize;

use crate::host::{Host, HostError, ServiceState};
use crate::model::{
    FIREWALL_RULE, LEGACY_SERVICE, Layout, SERVICE, Secret, Whitelist, WinPath, bundled_bins,
    generated_whitelist, layout, os_arch,
};

/// How long a service gets to start or stop.
pub const SERVICE_TIMEOUT: Duration = Duration::from_secs(30);

/// Why an operation failed.
#[derive(Debug, thiserror::Error)]
pub enum OpError {
    /// Needs administrator rights.
    #[error("Run this from an elevated prompt (Run as administrator).")]
    NeedsAdmin,
    /// Guardian 2.x is installed.
    #[error("Guardian 2.x (ProcSentinelAgent) is installed. Remove it first (--remove-legacy).")]
    LegacyAgent,
    /// The agent could not enroll (its words).
    #[error("Enrollment failed: {0}")]
    Enroll(String),
    /// Another mutating operation holds the lock.
    #[error("Another Guardian Setup operation is in progress.")]
    Busy,
    /// No agent here.
    #[error("The Guardian agent is not installed on this computer.")]
    NotInstalled,
    /// The archive's binaries are not next to the setup.
    #[error("{} is missing; run guardian-setup.exe from the unpacked archive.", .0.display())]
    MissingBinaries(PathBuf),
    /// A host call failed.
    #[error(transparent)]
    Host(#[from] HostError),
    /// A bug (a panic in the operation).
    #[error("Internal error: {0}")]
    Internal(String),
}

fn fs_err(what: &str, p: &Path, e: std::io::Error) -> OpError {
    OpError::Host(HostError::Failed(format!("{what} {}: {e}", p.display())))
}

/// What `install` asks for.
#[derive(Debug, Clone)]
pub struct InstallArgs {
    /// `wss://host:port`.
    pub server: String,
    /// The enrollment code.
    pub code: Secret,
    /// Pinned server certificate.
    pub cert_sha256: Option<String>,
    /// The local fallback port (`None`: keep / 11500).
    pub local_port: Option<u16>,
    /// Remove Guardian 2.x first.
    pub remove_legacy: bool,
    /// Register the service but leave it stopped.
    pub no_start: bool,
}

/// What `install` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Installed {
    /// This computer on the server.
    pub computer_id: u64,
    /// Enrolled now (false: an earlier enrollment was kept).
    pub enrolled_now: bool,
    /// Where the binaries are.
    pub program_dir: WinPath,
}

/// The state of this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    /// The agent binaries and the service are there.
    pub installed: bool,
    /// The service's state.
    pub service: Option<ServiceState>,
    /// The enrollment.
    pub computer_id: Option<u64>,
    /// The server address.
    pub server: Option<String>,
    /// The local fallback port (0 = off).
    pub local_port: u16,
    /// The firewall rule exists.
    pub firewall_rule: bool,
    /// Guardian 2.x is still here.
    pub legacy_agent: bool,
    /// `guardian-agent --version`.
    pub agent_version: Option<String>,
    /// What needs attention, in words.
    pub problems: Vec<String>,
}

fn need_admin(h: &dyn Host) -> Result<Layout, OpError> {
    if !h.is_elevated() {
        return Err(OpError::NeedsAdmin);
    }
    Ok(layout(&|k| h.env(k)))
}

fn load_config(h: &dyn Host, l: &Layout) -> Option<AgentConfig> {
    AgentConfig::load(&h.io_path(&l.config)).ok()
}

fn remove_tree(p: &Path) -> Result<(), OpError> {
    match std::fs::remove_dir_all(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(fs_err("cannot remove", p, e)),
    }
}

fn remove_file(p: &Path) -> Result<(), OpError> {
    match std::fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(fs_err("cannot remove", p, e)),
    }
}

fn copy(from: &Path, to: &Path) -> Result<(), OpError> {
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| fs_err("cannot copy to", to, e))
}

/// The bundled agent and worker for this computer, checked to exist.
fn bundle(h: &dyn Host) -> Result<(PathBuf, PathBuf), OpError> {
    let (agent, worker) = bundled_bins(&h.setup_dir(), os_arch(&|k| h.env(k)));
    for p in [&agent, &worker] {
        if !p.is_file() {
            return Err(OpError::MissingBinaries(p.clone()));
        }
    }
    Ok((agent, worker))
}

fn stop_if_running(h: &dyn Host, name: &str) -> Result<(), OpError> {
    if let Some(st) = h.service(name)?
        && st != ServiceState::Stopped
    {
        h.stop_service(name, SERVICE_TIMEOUT)?;
    }
    Ok(())
}

fn enroll(
    h: &dyn Host,
    l: &Layout,
    server: &str,
    code: &Secret,
    cert: Option<&str>,
) -> Result<u64, OpError> {
    let mut args = vec![
        "enroll".to_string(),
        "--server".to_string(),
        server.to_string(),
        "--code".to_string(),
        code.expose().to_string(),
    ];
    if let Some(c) = cert {
        args.push("--cert-sha256".into());
        args.push(c.to_string());
    }
    let out = h.run(&h.io_path(&l.agent_exe), &args)?;
    if out.code != 0 {
        let msg = out.stderr.trim();
        let msg = msg.strip_prefix("error: ").unwrap_or(msg);
        return Err(OpError::Enroll(
            msg.replace(code.expose(), "***").trim().to_string(),
        ));
    }
    load_config(h, l)
        .and_then(|c| c.computer_id)
        .ok_or_else(|| OpError::Enroll("the agent wrote no computer id".into()))
}

fn remove_legacy(h: &dyn Host, l: &Layout, progress: &dyn Fn(&str)) -> Result<(), OpError> {
    if h.service(LEGACY_SERVICE)?.is_some() {
        progress("Removing Guardian 2.x (ProcSentinelAgent)…");
        stop_if_running(h, LEGACY_SERVICE)?;
        h.delete_service(LEGACY_SERVICE)?;
    }
    remove_tree(&h.io_path(&l.legacy_dir))
}

fn legacy_present(h: &dyn Host, l: &Layout) -> bool {
    h.service(LEGACY_SERVICE).ok().flatten().is_some() || h.io_path(&l.legacy_dir).exists()
}

/// Installs (or repairs) the agent; an existing enrollment is kept.
pub fn install(
    h: &dyn Host,
    a: &InstallArgs,
    progress: &dyn Fn(&str),
) -> Result<Installed, OpError> {
    let l = need_admin(h)?;
    if legacy_present(h, &l) {
        if !a.remove_legacy {
            return Err(OpError::LegacyAgent);
        }
        remove_legacy(h, &l, progress)?;
    }
    let (agent, worker) = bundle(h)?;
    stop_if_running(h, SERVICE)?;
    progress(&format!("Copying the agent to {}…", l.program_dir));
    let dir = h.io_path(&l.program_dir);
    std::fs::create_dir_all(&dir).map_err(|e| fs_err("cannot create", &dir, e))?;
    copy(&agent, &h.io_path(&l.agent_exe))?;
    copy(&worker, &h.io_path(&l.worker_exe))?;

    let existing = load_config(h, &l).and_then(|c| c.computer_id);
    let (computer_id, enrolled_now) = match existing {
        Some(id) => {
            progress(&format!("Keeping the enrollment as computer {id}."));
            (id, false)
        }
        None => {
            progress(&format!("Enrolling with {}…", a.server));
            (
                enroll(h, &l, &a.server, &a.code, a.cert_sha256.as_deref())?,
                true,
            )
        }
    };
    if let Some(port) = a.local_port
        && let Some(mut c) = load_config(h, &l)
    {
        c.local_port = Some(port);
        let p = h.io_path(&l.config);
        c.save(&p)
            .map_err(|e| OpError::Host(HostError::Failed(e.to_string())))?;
    }
    let wl = h.io_path(&l.data_dir.join("whitelist.txt"));
    if !wl.exists() {
        progress("Writing whitelist.txt from the programs running now…");
        let procs = h.processes().unwrap_or_else(|e| {
            progress(&format!(
                "Cannot list processes ({e}); the whitelist starts empty."
            ));
            Vec::new()
        });
        let w = generated_whitelist(&procs, &h.trusted_roots());
        if let Some(d) = wl.parent() {
            std::fs::create_dir_all(d).map_err(|e| fs_err("cannot create", d, e))?;
        }
        std::fs::write(&wl, w.render()).map_err(|e| fs_err("cannot write", &wl, e))?;
    }
    let port = load_config(h, &l)
        .map(|c| c.local_port())
        .unwrap_or(DEFAULT_LOCAL_PORT);
    if port == 0 {
        h.delete_firewall_rule(FIREWALL_RULE)?;
    } else {
        progress(&format!("Opening port {port} for local fallback mode…"));
        h.set_firewall_rule(FIREWALL_RULE, port)?;
    }
    if h.service(SERVICE)?.is_none() {
        progress("Registering the GuardianAgent service…");
        let out = h.run(
            &h.io_path(&l.agent_exe),
            &["service".to_string(), "install".to_string()],
        )?;
        if out.code != 0 {
            return Err(OpError::Host(HostError::Failed(
                out.stderr.trim().to_string(),
            )));
        }
        if a.no_start {
            h.stop_service(SERVICE, SERVICE_TIMEOUT)?;
        }
    } else if !a.no_start {
        progress("Starting the GuardianAgent service…");
        h.start_service(SERVICE, SERVICE_TIMEOUT)?;
    }
    progress("Installation completed.");
    Ok(Installed {
        computer_id,
        enrolled_now,
        program_dir: l.program_dir,
    })
}

fn old(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

/// Replaces the binaries with the bundled ones; puts the old ones back if the
/// service does not start.
pub fn update(h: &dyn Host, progress: &dyn Fn(&str)) -> Result<(), OpError> {
    let l = need_admin(h)?;
    let installed = h.io_path(&l.agent_exe);
    if !installed.is_file() || h.service(SERVICE)?.is_none() {
        return Err(OpError::NotInstalled);
    }
    let (agent, worker) = bundle(h)?;
    progress("Stopping the GuardianAgent service…");
    stop_if_running(h, SERVICE)?;
    let pairs = [(agent, installed), (worker, h.io_path(&l.worker_exe))];
    for (_, to) in &pairs {
        let backup = old(to);
        remove_file(&backup)?;
        if to.is_file() {
            std::fs::rename(to, &backup).map_err(|e| fs_err("cannot move", to, e))?;
        }
    }
    let copied = pairs.iter().try_for_each(|(from, to)| copy(from, to));
    let started = copied.and_then(|()| {
        progress("Starting the new agent…");
        h.start_service(SERVICE, SERVICE_TIMEOUT)
            .map_err(OpError::from)
    });
    match started {
        Ok(()) => {
            for (_, to) in &pairs {
                let _ = remove_file(&old(to));
            }
            progress("Update completed.");
            Ok(())
        }
        Err(e) => {
            progress("The new agent did not start; restoring the previous one…");
            for (_, to) in &pairs {
                let backup = old(to);
                if backup.is_file() {
                    let _ = remove_file(to);
                    std::fs::rename(&backup, to).map_err(|e| fs_err("cannot restore", to, e))?;
                }
            }
            h.start_service(SERVICE, SERVICE_TIMEOUT)?;
            Err(e)
        }
    }
}

/// Removes the service, the firewall rule and the binaries; `purge` also the
/// data (key, enrollment, whitelist).
pub fn uninstall(h: &dyn Host, purge: bool, progress: &dyn Fn(&str)) -> Result<(), OpError> {
    let l = need_admin(h)?;
    let dir = h.io_path(&l.program_dir);
    let registered = h.service(SERVICE)?.is_some();
    let purgeable = purge && h.io_path(&l.data_dir).exists();
    if !(registered || dir.exists() || purgeable) {
        return Err(OpError::NotInstalled);
    }
    if registered {
        progress("Removing the GuardianAgent service…");
        stop_if_running(h, SERVICE)?;
        h.delete_service(SERVICE)?;
    }
    h.delete_firewall_rule(FIREWALL_RULE)?;
    remove_tree(&dir)?;
    if purge {
        progress("Removing the agent's data…");
        remove_tree(&h.io_path(&l.data_dir))?;
    }
    progress("Uninstall completed.");
    Ok(())
}

/// A new identity: the key, the enrollment and the saved state go, then the
/// agent enrolls with `code` (§10.3).
pub fn reenroll(
    h: &dyn Host,
    server: Option<String>,
    code: Secret,
    cert: Option<String>,
    progress: &dyn Fn(&str),
) -> Result<u64, OpError> {
    let l = need_admin(h)?;
    if !h.io_path(&l.agent_exe).is_file() {
        return Err(OpError::NotInstalled);
    }
    let previous = load_config(h, &l);
    let Some(server) = server.or_else(|| previous.as_ref().map(|c| c.server.clone())) else {
        return Err(OpError::Enroll("no server address (use --server)".into()));
    };
    let cert = cert.or_else(|| previous.as_ref().and_then(|c| c.cert_sha256.clone()));
    stop_if_running(h, SERVICE)?;
    let data = h.io_path(&l.data_dir);
    for f in ["agent.key", "state.pb", "seen.txt", "local.pb"] {
        remove_file(&data.join(f))?;
    }
    let cleared = AgentConfig {
        server: server.clone(),
        cert_sha256: cert.clone(),
        computer_id: None,
        local_port: previous.and_then(|c| c.local_port),
    };
    cleared
        .save(&h.io_path(&l.config))
        .map_err(|e| OpError::Host(HostError::Failed(e.to_string())))?;
    progress(&format!("Enrolling again with {server}…"));
    let id = enroll(h, &l, &server, &code, cert.as_deref())?;
    if h.service(SERVICE)?.is_some() {
        h.start_service(SERVICE, SERVICE_TIMEOUT)?;
    } else {
        let out = h.run(
            &h.io_path(&l.agent_exe),
            &["service".to_string(), "install".to_string()],
        )?;
        if out.code != 0 {
            return Err(OpError::Host(HostError::Failed(
                out.stderr.trim().to_string(),
            )));
        }
    }
    progress(&format!("Enrolled as computer {id}."));
    Ok(id)
}

/// The whitelist file's path for this host.
pub fn whitelist_path(h: &dyn Host) -> PathBuf {
    h.io_path(&layout(&|k| h.env(k)).data_dir.join("whitelist.txt"))
}

/// Reads the whitelist (missing = empty) with its warnings.
pub fn read_whitelist(h: &dyn Host) -> (Whitelist, Vec<String>) {
    match std::fs::read_to_string(whitelist_path(h)) {
        Ok(t) => Whitelist::parse(&t),
        Err(_) => (Whitelist::default(), Vec::new()),
    }
}

/// Writes the whitelist (the agent picks it up at its next full scan).
pub fn write_whitelist(h: &dyn Host, w: &Whitelist) -> Result<(), OpError> {
    if !h.is_elevated() {
        return Err(OpError::NeedsAdmin);
    }
    let p = whitelist_path(h);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| fs_err("cannot create", d, e))?;
    }
    std::fs::write(&p, w.render()).map_err(|e| fs_err("cannot write", &p, e))
}

/// The computer's state and what needs attention.
pub fn status(h: &dyn Host) -> Status {
    let l = layout(&|k| h.env(k));
    let exe = h.io_path(&l.agent_exe);
    let service = h.service(SERVICE).ok().flatten();
    let installed = exe.is_file() && service.is_some();
    let config = load_config(h, &l);
    let local_port = config
        .as_ref()
        .map(AgentConfig::local_port)
        .unwrap_or(DEFAULT_LOCAL_PORT);
    let firewall_rule = h.firewall_rule(FIREWALL_RULE).unwrap_or(false);
    let legacy_agent = legacy_present(h, &l);
    let agent_version = if exe.is_file() {
        h.run(&exe, &["--version".to_string()])
            .ok()
            .filter(|o| o.code == 0)
            .map(|o| o.stdout.trim().to_string())
    } else {
        None
    };
    let computer_id = config.as_ref().and_then(|c| c.computer_id);
    let mut problems = Vec::new();
    if !installed {
        problems.push("The Guardian agent is not installed.".to_string());
    } else {
        if service != Some(ServiceState::Running) {
            problems.push("The GuardianAgent service is not running.".to_string());
        }
        if computer_id.is_none() {
            problems.push("The agent is not enrolled.".to_string());
        }
        if local_port != 0 && !firewall_rule {
            problems.push(format!(
                "The firewall rule for local fallback (port {local_port}) is missing."
            ));
        }
    }
    if legacy_agent {
        problems.push("Guardian 2.x (ProcSentinelAgent) is still installed.".to_string());
    }
    Status {
        installed,
        service,
        computer_id,
        server: config.map(|c| c.server),
        local_port,
        firewall_rule,
        legacy_agent,
        agent_version,
        problems,
    }
}
