//! The window asks for administrator rights by starting itself again through
//! UAC, at most once (`--elevated-relaunch` marks the second start).

use std::ffi::OsString;
use std::path::Path;

/// Marks the elevated second start.
pub const RELAUNCH_FLAG: &str = "--elevated-relaunch";

/// Whether to start again elevated: not elevated and not already relaunched.
pub fn should_relaunch(elevated: bool, args: &[OsString]) -> bool {
    !elevated && !args.iter().any(|a| a == RELAUNCH_FLAG)
}

/// PowerShell arguments that start `exe` elevated with the marker.
pub fn relaunch_args(exe: &Path) -> Vec<String> {
    let quoted = exe.display().to_string().replace('\'', "''");
    vec![
        "-NoProfile".into(),
        "-Command".into(),
        format!("Start-Process -Verb RunAs -FilePath '{quoted}' -ArgumentList '{RELAUNCH_FLAG}'"),
    ]
}

/// What the non-elevated window does after asking for rights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterRelaunch {
    /// The elevated copy runs: this one ends.
    Exit,
    /// Rights were refused: open the read-only window.
    Window,
}

/// `started`: the relaunch command's result (`Some(success)`), `None` when it
/// could not run.
/// (PowerShell's `Start-Process` fails, and PowerShell exits non-zero, when
/// the UAC prompt is refused.)
pub fn after_relaunch(started: Option<bool>) -> AfterRelaunch {
    match started {
        Some(true) => AfterRelaunch::Exit,
        Some(false) | None => AfterRelaunch::Window,
    }
}
