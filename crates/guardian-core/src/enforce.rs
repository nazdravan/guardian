//! The kill/allow decision for one process under one profile.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::policy::{AppName, Mode, Profile};
use crate::protected::is_protected;

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
    /// Full executable path; `None` for Linux kernel threads.
    pub exe_path: Option<PathBuf>,
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
/// Protected processes (and Linux kernel threads) are never killed; in whitelist mode
/// the computer's `local_whitelist` counts as allowed too.
pub fn decide(
    os: Os,
    profile: &Profile,
    local_whitelist: &BTreeSet<AppName>,
    p: &ProcessInfo,
) -> Verdict {
    let name = p.name.trim().to_lowercase();
    let stem = name.strip_suffix(".exe");
    let protected = is_protected(os, &name)
        || stem.is_some_and(|s| is_protected(os, s))
        || (os == Os::Linux && p.exe_path.is_none());
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
            exe_path: Some("/x".into()),
        }
    }

    #[test]
    fn free_kills_nothing() {
        assert_eq!(
            decide(
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
            decide(Os::Windows, &p, &Default::default(), &proc_("Chrome.exe")),
            Verdict::Kill
        );
        assert_eq!(
            decide(
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
            decide(
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
            decide(Os::Windows, &p, &local, &proc_("chrome.exe")),
            Verdict::Kill
        );
        assert_eq!(
            decide(Os::Windows, &p, &local, &proc_("WORD.EXE")),
            Verdict::Allow
        );
        assert_eq!(
            decide(Os::Windows, &p, &local, &proc_("printer-tool.exe")),
            Verdict::Allow
        );
        assert_eq!(
            decide(Os::Windows, &p, &local, &proc_("svchost.exe")),
            Verdict::Allow
        );
        assert_eq!(
            decide(Os::Windows, &p, &local, &proc_("guardian-agent.exe")),
            Verdict::Allow
        );
    }

    #[test]
    fn linux_kernel_threads_are_never_killed() {
        let k = ProcessInfo {
            pid: 2,
            name: "kthreadd".into(),
            exe_path: None,
        };
        assert_eq!(
            decide(
                Os::Linux,
                &prof(Mode::Whitelist, &[]),
                &Default::default(),
                &k
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn linux_desktop_survives_whitelist() {
        assert_eq!(
            decide(
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
}
