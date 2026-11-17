//! `/proc`: listing processes and killing one only if it is still the same one.

use std::path::{Path, PathBuf};

use guardian_core::enforce::ProcessInfo;
use nix::errno::Errno;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

use crate::PlatformError;

/// `comm` and `starttime` from the text of `/proc/<pid>/stat`.
///
/// `comm` is in parentheses and may itself contain spaces and `)`, so it ends at
/// the **last** `)`. `starttime` is field 22 (the 20th after `comm`).
pub fn parse_stat(stat: &str) -> Option<(String, u64)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    if close < open {
        return None;
    }
    let comm = stat.get(open + 1..close)?.to_string();
    let rest = stat.get(close + 1..)?;
    // Fields after comm start at field 3 (state); starttime is field 22.
    let start = rest.split_whitespace().nth(19)?.parse().ok()?;
    Some((comm, start))
}

fn exe_of(pid: u32) -> Option<PathBuf> {
    let p = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let s = p.to_string_lossy();
    Some(match s.strip_suffix(" (deleted)") {
        Some(t) => PathBuf::from(t),
        None => p,
    })
}

/// One process; `NotFound` if it is gone.
pub fn info(pid: u32) -> Result<ProcessInfo, PlatformError> {
    let stat =
        std::fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => PlatformError::NotFound,
            _ => PlatformError::Io(e),
        })?;
    let (comm, start) =
        parse_stat(&stat).ok_or_else(|| PlatformError::Os(format!("bad /proc/{pid}/stat")))?;
    let exe = exe_of(pid);
    // The executable's file name is the full name; `comm` is cut at 15 bytes.
    let name = exe
        .as_deref()
        .and_then(Path::file_name)
        .map_or(comm, |n| n.to_string_lossy().into_owned());
    Ok(ProcessInfo {
        pid,
        name,
        exe_path: exe,
        start_marker: start,
    })
}

/// Every process that can be read; processes that vanish meanwhile are skipped.
pub fn list() -> Result<Vec<ProcessInfo>, PlatformError> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if let Ok(p) = info(pid) {
            out.push(p);
        }
    }
    Ok(out)
}

/// SIGKILL to `p.pid`, only if name, executable and start time still match.
pub fn kill_checked(p: &ProcessInfo) -> Result<(), PlatformError> {
    let now = info(p.pid)?;
    if now.name != p.name || now.exe_path != p.exe_path || now.start_marker != p.start_marker {
        return Err(PlatformError::Changed);
    }
    let pid = i32::try_from(p.pid).map_err(|_| PlatformError::NotFound)?;
    kill(Pid::from_raw(pid), Signal::SIGKILL).map_err(|e| match e {
        Errno::ESRCH => PlatformError::NotFound,
        Errno::EPERM => PlatformError::AccessDenied,
        e => PlatformError::Os(e.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comm_with_parenthesis_is_parsed() {
        let fields: Vec<String> = (3..=22).map(|i| i.to_string()).collect();
        let stat = format!("123 (a) b)) {} 23 24", fields.join(" "));
        let (comm, start) = parse_stat(&stat).unwrap();
        assert_eq!(comm, "a) b)");
        assert_eq!(start, 22);
    }

    #[test]
    fn short_stat_is_rejected() {
        assert_eq!(parse_stat("1 (x) S 0"), None);
        assert_eq!(parse_stat("garbage"), None);
    }
}
