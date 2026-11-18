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
    /// Some files could not be read without administrator rights.
    pub needs_admin: bool,
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

/// Attempts for a file operation that may hit a file still in use.
const FILE_ATTEMPTS: u32 = 20;
/// Pause between them (10 s in all).
const FILE_PAUSE: Duration = Duration::from_millis(500);

/// A stopped service's process (and its worker) can hold its files a little
/// longer: sharing violations and access denials are retried for a while.
fn busy(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(32)
}

fn retrying<T>(h: &dyn Host, mut f: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut attempt = 1;
    loop {
        match f() {
            Err(e) if busy(&e) && attempt < FILE_ATTEMPTS => {
                attempt += 1;
                h.wait(FILE_PAUSE);
            }
            other => return other,
        }
    }
}

fn remove_tree(h: &dyn Host, p: &Path) -> Result<(), OpError> {
    match retrying(h, || std::fs::remove_dir_all(p)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(fs_err("cannot remove", p, e)),
    }
}

fn remove_file(h: &dyn Host, p: &Path) -> Result<(), OpError> {
    match retrying(h, || std::fs::remove_file(p)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(fs_err("cannot remove", p, e)),
    }
}

fn copy(h: &dyn Host, from: &Path, to: &Path) -> Result<(), OpError> {
    retrying(h, || std::fs::copy(from, to))
        .map(|_| ())
        .map_err(|e| fs_err("cannot copy to", to, e))
}

fn rename(h: &dyn Host, from: &Path, to: &Path, what: &str) -> Result<(), OpError> {
    retrying(h, || std::fs::rename(from, to)).map_err(|e| fs_err(what, from, e))
}

/// Waits until `name` runs (a fresh `service install` returns while it starts).
fn wait_running(h: &dyn Host, name: &str) -> Result<(), OpError> {
    let polls = SERVICE_TIMEOUT.as_millis() / FILE_PAUSE.as_millis();
    for _ in 0..polls {
        match h.service(name)? {
            Some(ServiceState::Running) => return Ok(()),
            Some(ServiceState::Starting) => h.wait(FILE_PAUSE),
            _ => break,
        }
    }
    Err(OpError::Host(HostError::Failed(format!(
        "The {name} service did not start; see agent.log in the agent's data folder."
    ))))
}

/// Registers the service through the installed agent and waits until it runs.
fn register_service(h: &dyn Host, l: &Layout) -> Result<(), OpError> {
    let out = h.run(
        &h.io_path(&l.agent_exe),
        &["service".to_string(), "install".to_string()],
        &[],
    )?;
    if out.code != 0 {
        return Err(OpError::Host(HostError::Failed(
            out.stderr.trim().to_string(),
        )));
    }
    wait_running(h, SERVICE)
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
    ];
    if let Some(c) = cert {
        args.push("--cert-sha256".into());
        args.push(c.to_string());
    }
    // The code goes through the environment: a command line is visible to
    // every user (Task Manager, process auditing).
    let env = [("GUARDIAN_AGENT_CODE".to_string(), code.expose().to_string())];
    let out = h.run(&h.io_path(&l.agent_exe), &args, &env)?;
    if out.code != 0 {
        let msg = out.stderr.trim();
        let msg = msg.strip_prefix("error:").unwrap_or(msg).trim();
        let msg = if code.expose().is_empty() {
            msg.to_string()
        } else {
            msg.replace(code.expose(), "***")
        };
        return Err(OpError::Enroll(if msg.is_empty() {
            format!(
                "guardian-agent stopped with exit code {} and no message (a missing system library?)",
                out.code
            )
        } else {
            msg
        }));
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
    remove_tree(h, &h.io_path(&l.legacy_dir))
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
    // Nothing is removed before we know the new agent can be installed.
    let (agent, worker) = bundle(h)?;
    if legacy_present(h, &l) {
        if !a.remove_legacy {
            return Err(OpError::LegacyAgent);
        }
        remove_legacy(h, &l, progress)?;
    }
    stop_if_running(h, SERVICE)?;
    progress(&format!("Copying the agent to {}…", l.program_dir));
    let dir = h.io_path(&l.program_dir);
    std::fs::create_dir_all(&dir).map_err(|e| fs_err("cannot create", &dir, e))?;
    copy(h, &agent, &h.io_path(&l.agent_exe))?;
    copy(h, &worker, &h.io_path(&l.worker_exe))?;

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
        register_service(h, &l)?;
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

fn bak(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".bak");
    PathBuf::from(s)
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
        remove_file(h, &backup)?;
        if to.is_file() {
            rename(h, to, &backup, "cannot move")?;
        }
    }
    let copied = pairs.iter().try_for_each(|(from, to)| copy(h, from, to));
    let started = copied.and_then(|()| {
        progress("Starting the new agent…");
        h.start_service(SERVICE, SERVICE_TIMEOUT)
            .map_err(OpError::from)
    });
    match started {
        Ok(()) => {
            for (_, to) in &pairs {
                let _ = remove_file(h, &old(to));
            }
            progress("Update completed.");
            Ok(())
        }
        Err(e) => {
            progress("The new agent did not start; restoring the previous one…");
            // The service manager may have started the new agent again on its
            // own (recovery actions): stop it before touching its files.
            if let Err(stop) = stop_if_running(h, SERVICE) {
                tracing::warn!("cannot stop the new agent before restoring: {stop}");
            }
            for (_, to) in &pairs {
                let backup = old(to);
                if backup.is_file() {
                    remove_file(h, to).map_err(|r| {
                        OpError::Host(HostError::Failed(format!(
                            "The new agent did not start ({e}) and the previous one could not be put back ({r})."
                        )))
                    })?;
                    rename(h, &backup, to, "cannot restore").map_err(|r| {
                        OpError::Host(HostError::Failed(format!(
                            "The new agent did not start ({e}) and the previous one could not be put back ({r})."
                        )))
                    })?;
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
    remove_tree(h, &dir)?;
    if purge {
        progress("Removing the agent's data…");
        remove_tree(h, &h.io_path(&l.data_dir))?;
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
    let config = h.io_path(&l.config);
    // The old identity is kept aside until the new one exists, so a wrong
    // code or an unreachable server leaves the computer as it was.
    let mut kept = Vec::new();
    for p in ["agent.key", "state.pb", "seen.txt", "local.pb"]
        .iter()
        .map(|f| data.join(f))
        .chain(std::iter::once(config.clone()))
    {
        if p.is_file() {
            let b = bak(&p);
            remove_file(h, &b)?;
            rename(h, &p, &b, "cannot set aside")?;
            kept.push(p);
        }
    }
    let restore = |h: &dyn Host| {
        let _ = remove_file(h, &config);
        for p in &kept {
            if let Err(e) = rename(h, &bak(p), p, "cannot restore") {
                tracing::error!("{e}");
            }
        }
        if h.service(SERVICE).ok().flatten().is_some()
            && let Err(e) = h.start_service(SERVICE, SERVICE_TIMEOUT)
        {
            tracing::error!("cannot restart the agent with its old identity: {e}");
        }
    };
    let cleared = AgentConfig {
        server: server.clone(),
        cert_sha256: cert.clone(),
        computer_id: None,
        local_port: previous.and_then(|c| c.local_port),
    };
    if let Err(e) = cleared.save(&config) {
        restore(h);
        return Err(OpError::Host(HostError::Failed(e.to_string())));
    }
    progress(&format!("Enrolling again with {server}…"));
    let id = match enroll(h, &l, &server, &code, cert.as_deref()) {
        Ok(id) => id,
        Err(e) => {
            progress("Enrollment failed; the previous identity is back.");
            restore(h);
            return Err(e);
        }
    };
    for p in &kept {
        let _ = remove_file(h, &bak(p));
    }
    if h.service(SERVICE)?.is_some() {
        h.start_service(SERVICE, SERVICE_TIMEOUT)?;
    } else {
        register_service(h, &l)?;
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
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => (
            Whitelist::default(),
            vec!["Run as administrator to read the whitelist.".to_string()],
        ),
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
    // The config is in the admin-only data folder: without rights it exists
    // but cannot be read, which is not the same as "not enrolled".
    let needs_admin = config.is_none()
        && std::fs::File::open(h.io_path(&l.config))
            .err()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied);
    let local_port = config
        .as_ref()
        .map(AgentConfig::local_port)
        .unwrap_or(DEFAULT_LOCAL_PORT);
    let firewall_rule = h.firewall_rule(FIREWALL_RULE).unwrap_or(false);
    let legacy_agent = legacy_present(h, &l);
    let agent_version = if exe.is_file() {
        h.run(&exe, &["--version".to_string()], &[])
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
        if computer_id.is_none() && !needs_admin {
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
        needs_admin,
        problems,
    }
}
