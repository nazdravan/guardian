//! The table of operations: the window and the command line both run them
//! through `run_op`, which takes the lock for mutating ones and turns a panic
//! into an error.

use std::fs::File;

use serde::Serialize;

use crate::host::{Host, HostError};
use crate::lifecycle::{
    self, InstallArgs, Installed, OpError, SERVICE_TIMEOUT, Status, read_whitelist, write_whitelist,
};
use crate::model::{SERVICE, Secret, layout};

/// One operation, as listed by `--help` and bound in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpSpec {
    /// The command (`whitelist add`).
    pub name: &'static str,
    /// One line of help.
    pub help: &'static str,
    /// Changes the computer (needs admin and the lock).
    pub mutating: bool,
    /// Has a control in the window.
    pub ui: bool,
}

const fn spec(name: &'static str, help: &'static str, mutating: bool, ui: bool) -> OpSpec {
    OpSpec {
        name,
        help,
        mutating,
        ui,
    }
}

/// Every operation.
pub const OPS: &[OpSpec] = &[
    spec(
        "status",
        "Show the agent's state and what needs attention",
        false,
        true,
    ),
    spec(
        "install",
        "Install the agent, enroll it and start it",
        true,
        true,
    ),
    spec(
        "update",
        "Replace the agent with the bundled version",
        true,
        true,
    ),
    spec(
        "uninstall",
        "Remove the agent (keeps its data unless --purge)",
        true,
        true,
    ),
    spec(
        "reenroll",
        "Give this computer a new identity with a new code",
        true,
        true,
    ),
    spec("whitelist list", "List the local whitelist", false, true),
    spec(
        "whitelist add",
        "Add a program to the local whitelist",
        true,
        true,
    ),
    spec(
        "whitelist remove",
        "Remove a program from the local whitelist",
        true,
        true,
    ),
    spec("processes", "List the running programs", false, true),
    spec(
        "service start",
        "Start the GuardianAgent service",
        true,
        true,
    ),
    spec("service stop", "Stop the GuardianAgent service", true, true),
    spec(
        "service restart",
        "Restart the GuardianAgent service",
        true,
        true,
    ),
    spec("logs", "Show the end of the agent's log", false, false),
    spec("version", "Show the setup's version", false, false),
];

/// The operations the window has a control for (checked against `OPS`).
pub fn ui_ops() -> &'static [&'static str] {
    &[
        "status",
        "install",
        "update",
        "uninstall",
        "reenroll",
        "whitelist list",
        "whitelist add",
        "whitelist remove",
        "processes",
        "service start",
        "service stop",
        "service restart",
    ]
}

/// An operation with its arguments.
#[derive(Debug, Clone)]
pub enum Op {
    /// `status`.
    Status {
        /// Exit 10 when something needs attention.
        check: bool,
    },
    /// `install`.
    Install(InstallArgs),
    /// `update`.
    Update,
    /// `uninstall`.
    Uninstall {
        /// Also the data.
        purge: bool,
    },
    /// `reenroll`.
    Reenroll {
        /// New server (default: the current one).
        server: Option<String>,
        /// The code.
        code: Secret,
        /// New pin.
        cert_sha256: Option<String>,
    },
    /// `whitelist list`.
    WhitelistList,
    /// `whitelist add`.
    WhitelistAdd(String),
    /// `whitelist remove`.
    WhitelistRemove(String),
    /// `processes`.
    Processes,
    /// `service start`.
    ServiceStart,
    /// `service stop`.
    ServiceStop,
    /// `service restart`.
    ServiceRestart,
    /// `logs`.
    Logs {
        /// How many lines.
        lines: usize,
    },
    /// `version`.
    Version,
    /// Panics (tests of the runner).
    #[cfg(feature = "testing")]
    Panic,
}

impl Op {
    /// The table entry's name.
    pub fn name(&self) -> &'static str {
        match self {
            Op::Status { .. } => "status",
            Op::Install(_) => "install",
            Op::Update => "update",
            Op::Uninstall { .. } => "uninstall",
            Op::Reenroll { .. } => "reenroll",
            Op::WhitelistList => "whitelist list",
            Op::WhitelistAdd(_) => "whitelist add",
            Op::WhitelistRemove(_) => "whitelist remove",
            Op::Processes => "processes",
            Op::ServiceStart => "service start",
            Op::ServiceStop => "service stop",
            Op::ServiceRestart => "service restart",
            Op::Logs { .. } => "logs",
            Op::Version => "version",
            #[cfg(feature = "testing")]
            Op::Panic => "update",
        }
    }

    fn mutating(&self) -> bool {
        OPS.iter().any(|o| o.name == self.name() && o.mutating)
    }
}

/// What an operation returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Output {
    /// `status`.
    Status(Status),
    /// `install`.
    Installed(Installed),
    /// A list (whitelist, processes, log lines).
    List(Vec<String>),
    /// A sentence.
    Done(String),
}

impl Output {
    /// For people.
    pub fn text(&self) -> String {
        match self {
            Output::Status(s) => {
                let mut out = String::new();
                out.push_str(&format!(
                    "installed: {}\nservice: {}\ncomputer: {}\nserver: {}\nlocal fallback port: {} ({})\nagent: {}\n",
                    if s.installed { "yes" } else { "no" },
                    s.service.map_or("not registered".to_string(), |st| format!("{st:?}").to_lowercase()),
                    s.computer_id.map_or("not enrolled".to_string(), |id| id.to_string()),
                    s.server.as_deref().unwrap_or("-"),
                    s.local_port,
                    if s.firewall_rule { "firewall rule present" } else { "no firewall rule" },
                    s.agent_version.as_deref().unwrap_or("-"),
                ));
                for p in &s.problems {
                    out.push_str(&format!("problem: {p}\n"));
                }
                out
            }
            Output::Installed(i) => format!(
                "Installed in {} as computer {}{}.\n",
                i.program_dir,
                i.computer_id,
                if i.enrolled_now {
                    ""
                } else {
                    " (enrollment kept)"
                }
            ),
            Output::List(v) => v.iter().map(|l| format!("{l}\n")).collect(),
            Output::Done(m) => format!("{m}\n"),
        }
    }
}

/// The lock held by a mutating operation; released on drop.
#[derive(Debug)]
pub struct LockGuard(File);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Takes the setup lock, or `Busy`.
pub fn hold_lock(h: &dyn Host) -> Result<LockGuard, OpError> {
    let p = h.io_path(&layout(&|k| h.env(k)).lock);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)
            .map_err(|e| OpError::Host(HostError::Failed(format!("{}: {e}", d.display()))))?;
    }
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&p)
        .map_err(|e| OpError::Host(HostError::Failed(format!("{}: {e}", p.display()))))?;
    match f.try_lock() {
        Ok(()) => Ok(LockGuard(f)),
        Err(std::fs::TryLockError::WouldBlock) => Err(OpError::Busy),
        Err(std::fs::TryLockError::Error(e)) => Err(OpError::Host(HostError::Failed(format!(
            "{}: {e}",
            p.display()
        )))),
    }
}

fn service_op(h: &dyn Host, stop: bool, start: bool) -> Result<Output, OpError> {
    if !h.is_elevated() {
        return Err(OpError::NeedsAdmin);
    }
    if h.service(SERVICE)?.is_none() {
        return Err(OpError::NotInstalled);
    }
    if stop {
        h.stop_service(SERVICE, SERVICE_TIMEOUT)?;
    }
    if start {
        h.start_service(SERVICE, SERVICE_TIMEOUT)?;
    }
    Ok(Output::Done(
        match (stop, start) {
            (true, true) => "The GuardianAgent service was restarted.",
            (true, false) => "The GuardianAgent service is stopped.",
            _ => "The GuardianAgent service is running.",
        }
        .to_string(),
    ))
}

fn dispatch(h: &dyn Host, op: Op, progress: &dyn Fn(&str)) -> Result<Output, OpError> {
    match op {
        Op::Status { .. } => Ok(Output::Status(lifecycle::status(h))),
        Op::Install(a) => lifecycle::install(h, &a, progress).map(Output::Installed),
        Op::Update => lifecycle::update(h, progress).map(|()| Output::Done("Updated.".into())),
        Op::Uninstall { purge } => {
            lifecycle::uninstall(h, purge, progress).map(|()| Output::Done("Uninstalled.".into()))
        }
        Op::Reenroll {
            server,
            code,
            cert_sha256,
        } => lifecycle::reenroll(h, server, code, cert_sha256, progress)
            .map(|id| Output::Done(format!("Enrolled as computer {id}."))),
        Op::WhitelistList => {
            let (w, warnings) = read_whitelist(h);
            for w in warnings {
                progress(&w);
            }
            Ok(Output::List(w.names()))
        }
        Op::WhitelistAdd(name) => {
            let (mut w, _) = read_whitelist(h);
            let added = w.add(&name).map_err(|e| {
                OpError::Host(HostError::Failed(format!(
                    "{name:?} is not a program name: {e}"
                )))
            })?;
            write_whitelist(h, &w)?;
            Ok(Output::Done(if added {
                format!("Added {}.", name.trim().to_lowercase())
            } else {
                format!("{} was already allowed.", name.trim().to_lowercase())
            }))
        }
        Op::WhitelistRemove(name) => {
            let (mut w, _) = read_whitelist(h);
            let removed = w.remove(&name);
            write_whitelist(h, &w)?;
            Ok(Output::Done(if removed {
                format!("Removed {}.", name.trim().to_lowercase())
            } else {
                format!("{} was not in the whitelist.", name.trim().to_lowercase())
            }))
        }
        Op::Processes => {
            let mut names: Vec<String> = h.processes()?.into_iter().map(|p| p.name).collect();
            names.sort_by_key(|n| n.to_lowercase());
            names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            Ok(Output::List(names))
        }
        Op::ServiceStart => service_op(h, false, true),
        Op::ServiceStop => service_op(h, true, false),
        Op::ServiceRestart => service_op(h, true, true),
        Op::Logs { lines } => {
            let p = h.io_path(&layout(&|k| h.env(k)).data_dir.join("agent.log"));
            let text = std::fs::read_to_string(&p)
                .map_err(|e| OpError::Host(HostError::Failed(format!("{}: {e}", p.display()))))?;
            let all: Vec<&str> = text.lines().collect();
            let from = all.len().saturating_sub(lines);
            Ok(Output::List(
                all.get(from..)
                    .unwrap_or_default()
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ))
        }
        Op::Version => Ok(Output::Done(format!(
            "guardian-setup {}",
            env!("CARGO_PKG_VERSION")
        ))),
        #[cfg(feature = "testing")]
        #[allow(clippy::panic)]
        Op::Panic => panic!("test panic"),
    }
}

/// Runs one operation: mutating ones under the lock, every one under
/// `catch_unwind` (a panic becomes `OpError::Internal`, logged).
pub fn run_op(h: &dyn Host, op: Op, progress: &dyn Fn(&str)) -> Result<Output, OpError> {
    let name = op.name();
    let _guard = if op.mutating() {
        if !h.is_elevated() {
            return Err(OpError::NeedsAdmin);
        }
        Some(hold_lock(h)?)
    } else {
        None
    };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dispatch(h, op, progress))) {
        Ok(r) => r,
        Err(p) => {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            tracing::error!("panic in {name}: {msg}");
            Err(OpError::Internal(msg))
        }
    }
}
