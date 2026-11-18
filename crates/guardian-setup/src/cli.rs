//! The command line: the same operations as the window, no questions asked.
//! Exit codes: 0 ok, 1 failed, 2 usage, 5 needs admin, 10 `status --check`
//! found a problem.

use std::ffi::OsString;
use std::io::{Read, Write};

use clap::{Parser, Subcommand};
use serde_json::json;

use crate::host::Host;
use crate::lifecycle::{InstallArgs, OpError};
use crate::model::Secret;
use crate::ops::{Op, Output, run_op};

/// Success.
pub const EXIT_OK: i32 = 0;
/// The operation failed.
pub const EXIT_FAILED: i32 = 1;
/// Wrong usage.
pub const EXIT_USAGE: i32 = 2;
/// Needs administrator rights.
pub const EXIT_ADMIN: i32 = 5;
/// `status --check` found a problem.
pub const EXIT_UNHEALTHY: i32 = 10;

#[derive(Parser, Debug)]
#[command(
    name = "guardian-setup",
    version,
    about = "Installs and checks the Guardian 3.0 agent on this computer",
    after_help = "Without a command the setup window opens."
)]
struct Cli {
    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args, Debug)]
#[group(required = true, multiple = false)]
struct CodeArgs {
    /// The enrollment code.
    #[arg(long)]
    code: Option<String>,
    /// Read the code from a file (`-` = standard input), so it does not show in
    /// the process list.
    #[arg(long)]
    code_file: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Show the agent's state and what needs attention.
    Status {
        /// Exit with 10 when something needs attention.
        #[arg(long)]
        check: bool,
    },
    /// Install the agent, enroll it and start it.
    Install {
        /// The server, `wss://host:port`.
        #[arg(long)]
        server: String,
        #[command(flatten)]
        code: CodeArgs,
        /// The server certificate's SHA-256 (from the console).
        #[arg(long)]
        cert_sha256: Option<String>,
        /// The local fallback port (default 11500, 0 = off).
        #[arg(long)]
        local_port: Option<u16>,
        /// Remove Guardian 2.x (ProcSentinelAgent) first.
        #[arg(long)]
        remove_legacy: bool,
        /// Register the service but do not start it.
        #[arg(long)]
        no_start: bool,
    },
    /// Replace the agent with the bundled version.
    Update,
    /// Remove the agent (keeps its data unless --purge).
    Uninstall {
        /// Confirm.
        #[arg(long)]
        yes: bool,
        /// Also delete the enrollment, key and whitelist.
        #[arg(long)]
        purge: bool,
    },
    /// Give this computer a new identity with a new code.
    Reenroll {
        #[command(flatten)]
        code: CodeArgs,
        /// Another server.
        #[arg(long)]
        server: Option<String>,
        /// Another certificate SHA-256.
        #[arg(long)]
        cert_sha256: Option<String>,
    },
    /// The local whitelist.
    Whitelist {
        #[command(subcommand)]
        cmd: WhitelistCmd,
    },
    /// List the running programs.
    Processes,
    /// The GuardianAgent service.
    Service {
        #[command(subcommand)]
        cmd: ServiceCmd,
    },
    /// Show the end of the agent's log.
    Logs {
        /// How many lines.
        #[arg(long, default_value_t = 50)]
        lines: usize,
    },
    /// Show the setup's version.
    Version,
}

#[derive(Subcommand, Debug)]
enum WhitelistCmd {
    /// List the local whitelist.
    List,
    /// Add a program to the local whitelist.
    Add {
        /// Program name, e.g. `notepad.exe`.
        name: String,
    },
    /// Remove a program from the local whitelist.
    Remove {
        /// Program name.
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum ServiceCmd {
    /// Start the GuardianAgent service.
    Start,
    /// Stop the GuardianAgent service.
    Stop,
    /// Restart the GuardianAgent service.
    Restart,
}

fn read_code(c: CodeArgs, stdin: &mut dyn Read) -> Result<Secret, String> {
    match (c.code, c.code_file) {
        (Some(code), None) => Ok(Secret::new(code.trim())),
        (None, Some(f)) if f == "-" => {
            let mut s = String::new();
            stdin
                .read_to_string(&mut s)
                .map_err(|e| format!("cannot read the code from standard input: {e}"))?;
            Ok(Secret::new(s.trim()))
        }
        (None, Some(f)) => std::fs::read_to_string(&f)
            .map(|s| Secret::new(s.trim()))
            .map_err(|e| format!("cannot read {f}: {e}")),
        _ => Err("give --code or --code-file".into()),
    }
}

/// Turns the arguments into an operation; `Err` is a usage error.
fn to_op(cmd: Cmd, stdin: &mut dyn Read) -> Result<(Op, bool), String> {
    Ok(match cmd {
        Cmd::Status { check } => (Op::Status { check }, check),
        Cmd::Install {
            server,
            code,
            cert_sha256,
            local_port,
            remove_legacy,
            no_start,
        } => (
            Op::Install(InstallArgs {
                server,
                code: read_code(code, stdin)?,
                cert_sha256,
                local_port,
                remove_legacy,
                no_start,
            }),
            false,
        ),
        Cmd::Update => (Op::Update, false),
        Cmd::Uninstall { yes, purge } => {
            if !yes {
                return Err("uninstall removes the agent; add --yes to confirm".into());
            }
            (Op::Uninstall { purge }, false)
        }
        Cmd::Reenroll {
            code,
            server,
            cert_sha256,
        } => (
            Op::Reenroll {
                server,
                code: read_code(code, stdin)?,
                cert_sha256,
            },
            false,
        ),
        Cmd::Whitelist { cmd } => (
            match cmd {
                WhitelistCmd::List => Op::WhitelistList,
                WhitelistCmd::Add { name } => Op::WhitelistAdd(name),
                WhitelistCmd::Remove { name } => Op::WhitelistRemove(name),
            },
            false,
        ),
        Cmd::Processes => (Op::Processes, false),
        Cmd::Service { cmd } => (
            match cmd {
                ServiceCmd::Start => Op::ServiceStart,
                ServiceCmd::Stop => Op::ServiceStop,
                ServiceCmd::Restart => Op::ServiceRestart,
            },
            false,
        ),
        Cmd::Logs { lines } => (Op::Logs { lines }, false),
        Cmd::Version => (Op::Version, false),
    })
}

fn exit_for(e: &OpError) -> i32 {
    match e {
        OpError::NeedsAdmin => EXIT_ADMIN,
        _ => EXIT_FAILED,
    }
}

/// Runs the command line `args` (program name first) and returns the exit code.
pub fn main_cli(
    h: &dyn Host,
    args: Vec<OsString>,
    out: &mut dyn Write,
    stdin: &mut dyn Read,
) -> i32 {
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let _ = write!(out, "{e}");
            return match e.kind() {
                clap::error::ErrorKind::DisplayHelp
                | clap::error::ErrorKind::DisplayVersion
                | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => EXIT_OK,
                _ => EXIT_USAGE,
            };
        }
    };
    let json = cli.json;
    let (op, check) = match to_op(cli.cmd, stdin) {
        Ok(x) => x,
        Err(m) => {
            let _ = writeln!(out, "error: {m}");
            return EXIT_USAGE;
        }
    };
    let name = op.name();
    let progress = |m: &str| {
        if !json {
            eprintln!("{m}");
        }
        tracing::info!("{m}");
    };
    let r = run_op(h, op, &progress);
    let code = match &r {
        Ok(Output::Status(s)) if check && !s.problems.is_empty() => EXIT_UNHEALTHY,
        Ok(_) => EXIT_OK,
        Err(e) => exit_for(e),
    };
    if json {
        let v = match &r {
            Ok(o) => json!({"ok": true, "op": name, "result": o, "error": null}),
            Err(e) => json!({"ok": false, "op": name, "result": null, "error": e.to_string()}),
        };
        let _ = writeln!(out, "{v}");
    } else {
        match &r {
            Ok(o) => {
                let _ = write!(out, "{}", o.text());
            }
            Err(e) => {
                let _ = writeln!(out, "error: {e}");
            }
        }
    }
    code
}
