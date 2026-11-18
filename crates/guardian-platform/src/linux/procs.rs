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

/// `PF_KTHREAD` in the `flags` field (field 9) of `/proc/<pid>/stat`.
const PF_KTHREAD: u64 = 0x0020_0000;

/// Whether the `stat` text belongs to a kernel thread.
pub fn is_kernel_thread(stat: &str) -> bool {
    let Some(close) = stat.rfind(')') else {
        return false;
    };
    stat.get(close + 1..)
        .and_then(|rest| rest.split_whitespace().nth(6))
        .and_then(|f| f.parse::<u64>().ok())
        .is_some_and(|f| f & PF_KTHREAD != 0)
}

/// Whether `real` (the file `/proc/<pid>/exe` really points at) is the file at
/// `reported` in the agent's own mount namespace. A process in its own mount
/// namespace can make the link text say `/usr/bin/bash` for any file, so a
/// trusted path counts only when the inode matches. A deleted executable
/// (`… (deleted)`, e.g. after a package upgrade) counts when the real file has
/// no links left and sits on the same filesystem as the reported directory.
pub fn exe_verified(real: &std::fs::Metadata, reported: &Path, deleted: bool) -> bool {
    use std::os::unix::fs::MetadataExt;
    if deleted {
        return real.nlink() == 0
            && reported
                .parent()
                .and_then(|d| std::fs::metadata(d).ok())
                .is_some_and(|d| d.dev() == real.dev());
    }
    std::fs::metadata(reported).is_ok_and(|m| m.dev() == real.dev() && m.ino() == real.ino())
}

/// The reported executable path (for the name) and the path used for trust:
/// the same path when verified, otherwise the untrusted `/proc/<pid>/exe`.
fn exe_of(pid: u32) -> Option<(PathBuf, PathBuf)> {
    let link = format!("/proc/{pid}/exe");
    let p = std::fs::read_link(&link).ok()?;
    let text = p.to_string_lossy();
    let (reported, deleted) = match text.strip_suffix(" (deleted)") {
        Some(t) => (PathBuf::from(t), true),
        None => (p.clone(), false),
    };
    let verified = std::fs::metadata(&link).is_ok_and(|m| exe_verified(&m, &reported, deleted));
    let trust = if verified {
        reported.clone()
    } else {
        PathBuf::from(link)
    };
    Some((reported, trust))
}

/// One process; `NotFound` if it is gone or is a kernel thread (the agent never
/// acts on those).
pub fn info(pid: u32) -> Result<ProcessInfo, PlatformError> {
    let stat =
        std::fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => PlatformError::NotFound,
            _ => PlatformError::Io(e),
        })?;
    if is_kernel_thread(&stat) {
        return Err(PlatformError::NotFound);
    }
    let (comm, start) =
        parse_stat(&stat).ok_or_else(|| PlatformError::Os(format!("bad /proc/{pid}/stat")))?;
    let exe = exe_of(pid);
    // The executable's file name is the full name; `comm` is cut at 15 bytes.
    let name = exe
        .as_ref()
        .and_then(|(reported, _)| reported.file_name())
        .map_or(comm, |n| n.to_string_lossy().into_owned());
    Ok(ProcessInfo {
        pid,
        name,
        exe_path: exe.map(|(_, trust)| trust),
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

    fn stat_with_flags(flags: u64) -> String {
        let mut fields: Vec<String> = (3..=22).map(|i| i.to_string()).collect();
        fields[6] = flags.to_string();
        format!("2 (kthreadd) {}", fields.join(" "))
    }

    #[test]
    fn kernel_threads_are_recognised_by_their_flags() {
        assert!(is_kernel_thread(&stat_with_flags(0x0020_8040)));
        assert!(!is_kernel_thread(&stat_with_flags(0x0040_0100)));
        assert!(!is_kernel_thread("garbage"));
    }

    #[test]
    fn exe_identity_needs_the_same_file() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let real = std::fs::metadata(&a).unwrap();
        assert!(exe_verified(&real, &a, false));
        // A namespace can make the link say /usr/bin/bash for another file.
        assert!(!exe_verified(&real, &b, false));
        assert!(!exe_verified(&real, &d.path().join("missing"), false));
    }

    #[test]
    fn deleted_exe_is_verified_by_link_count_and_device() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        std::fs::write(&a, b"a").unwrap();
        let f = std::fs::File::open(&a).unwrap();
        assert!(!exe_verified(&f.metadata().unwrap(), &a, true));
        std::fs::remove_file(&a).unwrap();
        assert!(exe_verified(&f.metadata().unwrap(), &a, true));
    }

    #[test]
    fn short_stat_is_rejected() {
        assert_eq!(parse_stat("1 (x) S 0"), None);
        assert_eq!(parse_stat("garbage"), None);
    }
}
