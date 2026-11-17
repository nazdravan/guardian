//! `guardian-cli`: administer a Guardian 3.0 server and send classroom commands.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

mod commands;
mod session;

/// Guardian 3.0 command-line client.
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    /// Server URL, e.g. https://server.school:8443 (remembered after init/login).
    #[arg(long, env = "GUARDIAN_SERVER", global = true)]
    pub server: Option<String>,
    /// SHA-256 of the server certificate, printed by the server at start.
    #[arg(long, env = "GUARDIAN_CERT_SHA256", global = true)]
    pub cert_sha256: Option<String>,
    /// Session file (default: <config dir>/guardian/cli.toml).
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// JSON output for `list` and `show` commands.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// Top-level commands.
#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Create the first admin with the server's bootstrap code (password from stdin).
    Init {
        #[arg(long)]
        code: String,
        #[arg(long)]
        login: String,
    },
    /// Log in (password from stdin or prompt).
    Login {
        #[arg(long)]
        login: String,
    },
    /// End the current session.
    Logout,
    /// Show the logged-in user.
    Whoami,
    /// Change your password (old and new from stdin or prompt).
    Passwd,
    /// Manage users (admin).
    #[command(subcommand)]
    User(UserCmd),
    /// Manage rooms.
    #[command(subcommand)]
    Room(RoomCmd),
    /// Manage profiles (admin).
    #[command(subcommand)]
    Profile(ProfileCmd),
    /// Manage computers (admin).
    #[command(subcommand)]
    Computer(ComputerCmd),
    /// Create enrollment codes (admin).
    #[command(subcommand)]
    Code(CodeCmd),
    /// Manage trusted consoles (admin).
    #[command(subcommand)]
    Console(ConsoleCmd),
    /// Send a command to a room or to computers; prints the command id.
    Send(SendArgs),
    /// Inspect commands.
    #[command(subcommand)]
    Command(CommandCmd),
    /// Print live events until Ctrl+C.
    Watch,
    /// School settings (admin).
    #[command(subcommand)]
    Settings(SettingsCmd),
}

/// Roles.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RoleArg {
    /// Everything.
    Admin,
    /// Own rooms only.
    Teacher,
}

/// Profile modes.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ModeArg {
    /// Nothing is blocked.
    Free,
    /// Listed apps are closed.
    Blacklist,
    /// Only listed apps may run.
    Whitelist,
}

/// `user …`
#[derive(Debug, Subcommand)]
pub enum UserCmd {
    /// List users.
    List,
    /// Add a user (password from stdin or prompt).
    Add {
        login: String,
        #[arg(long, value_enum)]
        role: RoleArg,
    },
    /// Change a user's role.
    SetRole { login: String, role: RoleArg },
    /// Disable a user (ends their sessions).
    Disable { login: String },
    /// Enable a user.
    Enable { login: String },
    /// Set a new password (from stdin or prompt; ends their sessions).
    ResetPassword { login: String },
    /// Set exactly the rooms of a user (names or ids; none = no rooms).
    Rooms { login: String, rooms: Vec<String> },
}

/// `room …`
#[derive(Debug, Subcommand)]
pub enum RoomCmd {
    /// List rooms.
    List,
    /// Add a room (admin).
    Add { name: String },
    /// Rename a room (admin).
    Rename { room: String, name: String },
    /// Delete a room (admin).
    Rm { room: String },
    /// Switch the room's active profile.
    Profile { room: String, profile: String },
}

/// `profile …`
#[derive(Debug, Subcommand)]
pub enum ProfileCmd {
    /// List profiles.
    List,
    /// Add a profile.
    Add {
        name: String,
        #[arg(long, value_enum)]
        mode: ModeArg,
    },
    /// Change a profile's mode.
    Mode { profile: String, mode: ModeArg },
    /// Replace a profile's application list.
    Apps { profile: String, apps: Vec<String> },
    /// Add applications.
    AddApp {
        profile: String,
        #[arg(required = true)]
        apps: Vec<String>,
    },
    /// Remove applications.
    RmApp {
        profile: String,
        #[arg(required = true)]
        apps: Vec<String>,
    },
    /// Delete a profile.
    Rm { profile: String },
}

/// `computer …`
#[derive(Debug, Subcommand)]
pub enum ComputerCmd {
    /// List computers.
    List,
    /// Rename a computer.
    Rename { id: u64, name: String },
    /// Move to a room (`-` = no room).
    Move { id: u64, room: String },
    /// Set an exception profile (`-` = none).
    Except { id: u64, profile: String },
    /// Revoke a computer (it must enroll again).
    Revoke { id: u64 },
}

/// `code …`
#[derive(Debug, Subcommand)]
pub enum CodeCmd {
    /// Agent code: one computer for 15 min, or a room code with --room.
    Agent {
        #[arg(long)]
        room: Option<String>,
        /// Uses (room codes only, 1-500).
        #[arg(long)]
        uses: Option<u32>,
        /// Lifetime in seconds (room codes only).
        #[arg(long)]
        ttl: Option<u32>,
    },
    /// Console code (one use, 15 min).
    Console,
}

/// `console …`
#[derive(Debug, Subcommand)]
pub enum ConsoleCmd {
    /// List trusted consoles.
    List,
    /// Revoke a console.
    Revoke { id: i64 },
}

/// `send …`
#[derive(Debug, clap::Args)]
pub struct SendArgs {
    /// Target room (name or id).
    #[arg(
        long,
        conflicts_with = "computer",
        required_unless_present = "computer"
    )]
    pub room: Option<String>,
    /// Target computer ids.
    #[arg(long, num_args = 1..)]
    pub computer: Vec<u64>,
    /// Seconds before an undelivered command expires (default 600).
    #[arg(long)]
    pub ttl: Option<u32>,
    #[command(subcommand)]
    pub what: SendWhat,
}

/// What to send.
#[derive(Debug, Subcommand)]
pub enum SendWhat {
    /// Show a message.
    Message {
        #[arg(long)]
        text: String,
        #[arg(long, default_value = "")]
        title: String,
    },
    /// Start a program.
    Program {
        path: String,
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Open a web page.
    Site { url: String },
    /// Shut down.
    Shutdown {
        #[arg(long, default_value_t = 0)]
        countdown: u32,
    },
    /// Restart.
    Restart {
        #[arg(long, default_value_t = 0)]
        countdown: u32,
    },
}

/// `command …`
#[derive(Debug, Subcommand)]
pub enum CommandCmd {
    /// Show a command and its per-computer results.
    Show { id: String },
}

/// `settings …`
#[derive(Debug, Subcommand)]
pub enum SettingsCmd {
    /// Show settings.
    Show,
    /// Change settings.
    Set {
        #[arg(long)]
        default_profile: Option<String>,
        #[arg(long)]
        retention_days: Option<u32>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(commands::run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            match e.downcast_ref::<tonic::Status>() {
                Some(s) => eprintln!("error: {}", s.message()),
                None => eprintln!("error: {e:#}"),
            }
            ExitCode::FAILURE
        }
    }
}
