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
