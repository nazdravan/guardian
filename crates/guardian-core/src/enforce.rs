//! The kill/allow decision for one process under one profile.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::policy::{AppName, Mode, Profile};
use crate::protected::is_protected;
use crate::trusted::TrustedRoots;

/// The operating system the agent runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    /// Microsoft Windows.
    Windows,
    /// Linux.
    Linux,
}

/// One running process as the agent sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    /// Process id.
    pub pid: u32,
    /// Image name (e.g. `chrome.exe`, `firefox`).
    pub name: String,
    /// Full executable path; `None` for Linux kernel threads and for Windows
    /// processes that cannot be opened.
    pub exe_path: Option<PathBuf>,
    /// Identifies this process instance across PID reuse (Linux `starttime`,
    /// Windows creation time); 0 = unknown.
    pub start_marker: u64,
}

/// What to do with a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Leave it running.
    Allow,
    /// Terminate it.
    Kill,
}

/// `true` if a list entry equals `name` or `name` without its `.exe` suffix.
fn matches(list: &BTreeSet<AppName>, name: &str, stem: Option<&str>) -> bool {
    list.iter()
        .any(|e| e.as_str() == name || Some(e.as_str()) == stem)
}

/// Decides whether `p` must be killed under `profile`.
///
/// A protected system name protects a process only if its executable is under
/// `trusted` (or, on Windows, cannot be read at all — pseudo-processes and protected
/// processes). A process without a readable executable is protected only by
/// name (the Linux platform never reports kernel threads). In whitelist
/// mode the computer's `local_whitelist` counts as allowed too.
pub fn decide(
    os: Os,
    profile: &Profile,
    local_whitelist: &BTreeSet<AppName>,
    trusted: &TrustedRoots,
    p: &ProcessInfo,
) -> Verdict {
    let name = p.name.trim().to_lowercase();
    let stem = name.strip_suffix(".exe");
    let protected_name = is_protected(os, &name) || stem.is_some_and(|s| is_protected(os, s));
    let protected = match (&p.exe_path, os) {
        (None, _) => protected_name,
        (Some(path), _) => protected_name && trusted.contains(os, path),
    };
    if protected {
        return Verdict::Allow;
    }
    let kill = match profile.mode {
        Mode::Free => false,
        Mode::Blacklist => matches(&profile.apps, &name, stem),
        Mode::Whitelist => {
            !matches(&profile.apps, &name, stem) && !matches(local_whitelist, &name, stem)
        }
    };
    if kill { Verdict::Kill } else { Verdict::Allow }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{AppName, Mode, Profile, ProfileId};
    use crate::protected;
    use crate::trusted::TrustedRoots;
    use std::collections::BTreeSet;

    fn prof(mode: Mode, apps: &[&str]) -> Profile {
        Profile {
            id: ProfileId(1),
            name: "t".into(),
            mode,
            apps: apps.iter().map(|a| AppName::parse(a).unwrap()).collect(),
        }
    }

    fn proc_(name: &str) -> ProcessInfo {
        ProcessInfo {
            pid: 100,
            name: name.into(),
            exe_path: Some(format!("/x/{name}").into()),
            start_marker: 0,
        }
    }

    fn roots() -> TrustedRoots {
        TrustedRoots {
            recursive: vec!["/x".into()],
            ..TrustedRoots::none()
        }
    }

    fn win_roots() -> TrustedRoots {
        TrustedRoots {
            recursive: vec![r"C:\Windows\System32".into()],
            direct_files: vec![r"C:\Windows".into()],
            excluded: vec![r"C:\Windows\Temp".into()],
        }
    }

    /// The old 4-argument form, with test roots that trust `/x`.
    fn dec(os: Os, p: &Profile, local: &BTreeSet<AppName>, pr: &ProcessInfo) -> Verdict {
        decide(os, p, local, &roots(), pr)
    }

    #[test]
    fn free_kills_nothing() {
        assert_eq!(
            dec(
                Os::Windows,
                &prof(Mode::Free, &[]),
                &Default::default(),
                &proc_("chrome.exe")
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn blacklist_matches_with_or_without_exe() {
        let p = prof(Mode::Blacklist, &["chrome"]);
        assert_eq!(
            dec(Os::Windows, &p, &Default::default(), &proc_("Chrome.exe")),
            Verdict::Kill
        );
        assert_eq!(
            dec(
                Os::Windows,
                &p,
                &Default::default(),
                &proc_("chromedriver.exe")
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn blacklist_never_kills_protected() {
        assert_eq!(
            dec(
                Os::Windows,
                &prof(Mode::Blacklist, &["explorer"]),
                &Default::default(),
                &proc_("explorer.exe")
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn whitelist_kills_unlisted_but_not_protected_or_local() {
        let p = prof(Mode::Whitelist, &["word.exe"]);
        let local: BTreeSet<_> = [AppName::parse("printer-tool.exe").unwrap()].into();
        assert_eq!(
            dec(Os::Windows, &p, &local, &proc_("chrome.exe")),
            Verdict::Kill
        );
        assert_eq!(
            dec(Os::Windows, &p, &local, &proc_("WORD.EXE")),
            Verdict::Allow
        );
        assert_eq!(
            dec(Os::Windows, &p, &local, &proc_("printer-tool.exe")),
            Verdict::Allow
        );
        assert_eq!(
            dec(Os::Windows, &p, &local, &proc_("svchost.exe")),
            Verdict::Allow
        );
        assert_eq!(
            dec(Os::Windows, &p, &local, &proc_("guardian-agent.exe")),
            Verdict::Allow
        );
    }

    #[test]
    fn linux_process_without_readable_exe_is_judged_by_name() {
        // The platform drops kernel threads; a user process whose exe cannot be
        // read must not become exempt (an agent without CAP_SYS_PTRACE would
        // otherwise kill nothing in Exam).
        let p = |name: &str| ProcessInfo {
            pid: 4242,
            name: name.into(),
            exe_path: None,
            start_marker: 0,
        };
        let exam = prof(Mode::Whitelist, &[]);
        assert_eq!(
            dec(Os::Linux, &exam, &Default::default(), &p("game")),
            Verdict::Kill
        );
        assert_eq!(
            dec(Os::Linux, &exam, &Default::default(), &p("systemd")),
            Verdict::Allow
        );
    }

    #[test]
    fn linux_desktop_survives_whitelist() {
        assert_eq!(
            dec(
                Os::Linux,
                &prof(Mode::Whitelist, &[]),
                &Default::default(),
                &proc_("gnome-shell")
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn protected_lists_are_lowercase() {
        for n in protected::WINDOWS.iter().chain(protected::LINUX) {
            assert_eq!(*n, n.to_lowercase());
        }
    }

    #[test]
    fn renamed_binary_outside_trusted_roots_is_killed_in_exam() {
        let p = prof(Mode::Whitelist, &[]);
        let fake = ProcessInfo {
            pid: 9,
            name: "svchost.exe".into(),
            exe_path: Some(r"C:\Users\elev\Downloads\svchost.exe".into()),
            start_marker: 1,
        };
        assert_eq!(
            decide(Os::Windows, &p, &Default::default(), &win_roots(), &fake),
            Verdict::Kill
        );
        let real = ProcessInfo {
            exe_path: Some(r"C:\Windows\System32\svchost.exe".into()),
            ..fake.clone()
        };
        assert_eq!(
            decide(Os::Windows, &p, &Default::default(), &win_roots(), &real),
            Verdict::Allow
        );
    }

    #[test]
    fn excluded_writable_dir_is_not_trusted() {
        let p = prof(Mode::Whitelist, &[]);
        let t = ProcessInfo {
            pid: 9,
            name: "explorer.exe".into(),
            exe_path: Some("c:/windows/temp/explorer.exe".into()),
            start_marker: 1,
        };
        assert_eq!(
            decide(Os::Windows, &p, &Default::default(), &win_roots(), &t),
            Verdict::Kill
        );
        let real = ProcessInfo {
            exe_path: Some(r"C:\WINDOWS\explorer.exe".into()),
            ..t
        };
        assert_eq!(
            decide(Os::Windows, &p, &Default::default(), &win_roots(), &real),
            Verdict::Allow
        );
    }

    #[test]
    fn linux_home_bash_is_killed_system_bash_is_not() {
        let p = prof(Mode::Whitelist, &[]);
        let roots = TrustedRoots {
            recursive: vec!["/usr".into(), "/bin".into()],
            ..TrustedRoots::none()
        };
        let home = ProcessInfo {
            pid: 7,
            name: "bash".into(),
            exe_path: Some("/home/elev/bash".into()),
            start_marker: 1,
        };
        assert_eq!(
            decide(Os::Linux, &p, &Default::default(), &roots, &home),
            Verdict::Kill
        );
        let sys = ProcessInfo {
            exe_path: Some("/usr/bin/bash".into()),
            ..home.clone()
        };
        assert_eq!(
            decide(Os::Linux, &p, &Default::default(), &roots, &sys),
            Verdict::Allow
        );
        let dotdot = ProcessInfo {
            exe_path: Some("/usr/../home/elev/bash".into()),
            ..home
        };
        assert_eq!(
            decide(Os::Linux, &p, &Default::default(), &roots, &dotdot),
            Verdict::Kill
        );
    }

    #[test]
    fn windows_pseudo_process_without_path_is_protected() {
        let sys = ProcessInfo {
            pid: 4,
            name: "System".into(),
            exe_path: None,
            start_marker: 0,
        };
        assert_eq!(
            decide(
                Os::Windows,
                &prof(Mode::Whitelist, &[]),
                &Default::default(),
                &TrustedRoots::none(),
                &sys
            ),
            Verdict::Allow
        );
        let unknown = ProcessInfo {
            name: "game.exe".into(),
            ..sys
        };
        assert_eq!(
            decide(
                Os::Windows,
                &prof(Mode::Whitelist, &[]),
                &Default::default(),
                &TrustedRoots::none(),
                &unknown
            ),
            Verdict::Kill
        );
    }

    #[test]
    fn direct_files_are_not_recursive() {
        let t = win_roots();
        assert!(t.contains(
            Os::Windows,
            std::path::Path::new(r"C:\Windows\explorer.exe")
        ));
        assert!(!t.contains(
            Os::Windows,
            std::path::Path::new(r"C:\Windows\Fonts\explorer.exe")
        ));
        assert!(t.contains(
            Os::Windows,
            std::path::Path::new(r"c:\windows\system32\drivers\x.exe")
        ));
    }
}
