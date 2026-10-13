//! What the agent's `enforce` would do to a process right now (spec R16–R18).

use crate::agent;
use crate::parse::Mode;
use crate::read::Read;
use crate::snapshot::Snapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessVerdict {
    /// On the agent's hardcoded list (whitelist mode only).
    Protected,
    AllowedWhitelistFile,
    AllowedPolicy,
    /// `rule` is the blacklist rule that matches; `None` in whitelist mode.
    WillClose {
        rule: Option<String>,
    },
    /// `substring_rule`: a blacklist rule contained in this name — the agent tries
    /// `taskkill /IM <rule>` every second, which does not touch this process.
    NotAffected {
        substring_rule: Option<String>,
        /// A `*` rule that would glob-match this name; the agent never acts on wildcard rules.
        wildcard_rule: Option<String>,
    },
    Unknown(String),
}

impl ProcessVerdict {
    pub fn label(&self) -> String {
        match self {
            ProcessVerdict::Protected => "Protected by the system".into(),
            ProcessVerdict::AllowedWhitelistFile => "Allowed (whitelist.txt)".into(),
            ProcessVerdict::AllowedPolicy => "Allowed (policy)".into(),
            ProcessVerdict::WillClose { rule: Some(r) } => format!("Will be closed: blacklist rule `{r}`"),
            ProcessVerdict::WillClose { rule: None } => "Will be closed: whitelist mode, not in the room policy or whitelist.txt".into(),
            ProcessVerdict::NotAffected { wildcard_rule: Some(r), .. } => {
                format!("Not affected (rule `{r}` has a wildcard; the agent only acts on exact names, so it never closes anything)")
            }
            ProcessVerdict::NotAffected { substring_rule: Some(r), .. } => {
                format!("Not affected (substring match: rule `{r}` makes the agent try to close `{r}`, not this process)")
            }
            ProcessVerdict::NotAffected { .. } => "Not affected".into(),
            ProcessVerdict::Unknown(why) => format!("Unknown: {why}"),
        }
    }
    pub fn will_close(&self) -> bool {
        matches!(self, ProcessVerdict::WillClose { .. })
    }
}

/// taskkill /IM glob semantics: case-insensitive, `*` matches any run of characters.
/// Used only to explain a wildcard rule: the agent never reaches taskkill with one.
pub fn image_matches(rule: &str, name: &str) -> bool {
    let (rule, name) = (rule.to_lowercase(), name.to_lowercase());
    let parts: Vec<&str> = rule.split('*').collect();
    if parts.len() == 1 {
        return rule == name;
    }
    let mut rest = name.as_str();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else { return false };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            let Some(pos) = rest.find(part) else { return false };
            rest = &rest[pos + part.len()..];
        }
    }
    true
}

pub fn looks_locked(s: &Snapshot) -> bool {
    s.sync.as_ok().is_some_and(|f| f.parsed.mode == Mode::Whitelist && f.parsed.applications.is_empty())
}

pub fn verdict_for(s: &Snapshot, process: &str) -> ProcessVerdict {
    let policy = match &s.sync {
        Read::Ok(f) => &f.parsed,
        other => return ProcessVerdict::Unknown(format!("no policy ({})", other.problem().unwrap_or_default())),
    };
    let lower = process.trim().to_lowercase();
    match policy.mode {
        Mode::Whitelist => {
            if agent::is_protected(&lower) {
                ProcessVerdict::Protected
            } else if s.whitelist.as_ok().is_some_and(|w| w.parsed.contains(&lower)) {
                ProcessVerdict::AllowedWhitelistFile
            } else if policy.applications.iter().any(|a| a.name.to_lowercase() == lower) {
                ProcessVerdict::AllowedPolicy
            } else {
                ProcessVerdict::WillClose { rule: None }
            }
        }
        Mode::Blacklist => {
            let rules = policy.applications.iter().map(|a| a.name.as_str()).filter(|n| !n.is_empty());
            let mut substring = None;
            let mut wildcard = None;
            for r in rules {
                if r.contains('*') {
                    // Never a literal substring of the tasklist text, so the agent never tries it.
                    if wildcard.is_none() && image_matches(r, &lower) {
                        wildcard = Some(r.to_string());
                    }
                } else if r.to_lowercase() == lower {
                    return ProcessVerdict::WillClose { rule: Some(r.to_string()) };
                } else if substring.is_none() && lower.contains(&r.to_lowercase()) {
                    substring = Some(r.to_string());
                }
            }
            ProcessVerdict::NotAffected { substring_rule: substring, wildcard_rule: wildcard }
        }
        Mode::Free | Mode::Other(_) => ProcessVerdict::NotAffected { substring_rule: None, wildcard_rule: None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{App, Mode};
    use crate::read::Read;
    use crate::testkit::{fixtures_dir, snapshot_from_dir};

    fn snap(mode: Mode, apps: &[&str]) -> Snapshot {
        let mut s = snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap());
        if let Read::Ok(f) = &mut s.sync {
            f.parsed.mode = mode;
            f.parsed.applications = apps.iter().map(|n| App { name: n.to_string(), mode: String::new() }).collect();
        }
        s
    }

    #[test]
    fn whitelist_mode() {
        let s = snap(Mode::Whitelist, &["WinWord.exe"]);
        assert_eq!(verdict_for(&s, "explorer.exe"), ProcessVerdict::Protected);
        assert_eq!(verdict_for(&s, "Guardian.exe"), ProcessVerdict::Protected);
        assert_eq!(verdict_for(&s, "NOTEPAD.EXE"), ProcessVerdict::AllowedWhitelistFile);
        assert_eq!(verdict_for(&s, "winword.exe"), ProcessVerdict::AllowedPolicy);
        assert_eq!(verdict_for(&s, "game.exe"), ProcessVerdict::WillClose { rule: None });
    }

    #[test]
    fn blacklist_is_exact_image_name_with_substring_note() {
        let s = snap(Mode::Blacklist, &["code.exe", "steam*"]);
        assert_eq!(verdict_for(&s, "Code.exe"), ProcessVerdict::WillClose { rule: Some("code.exe".into()) });
        assert_eq!(verdict_for(&s, "vscode.exe"), ProcessVerdict::NotAffected { substring_rule: Some("code.exe".into()), wildcard_rule: None });
        assert_eq!(verdict_for(&s, "steamwebhelper.exe"), ProcessVerdict::NotAffected { substring_rule: None, wildcard_rule: Some("steam*".into()) });
        assert!(verdict_for(&s, "steamwebhelper.exe").label().contains("has a wildcard"));
        assert_eq!(verdict_for(&s, "notepad.exe"), ProcessVerdict::NotAffected { substring_rule: None, wildcard_rule: None });
        // Protected processes are not exempt in blacklist mode: the agent does not check isSystemProcess there.
        let s = snap(Mode::Blacklist, &["explorer.exe"]);
        assert_eq!(verdict_for(&s, "explorer.exe"), ProcessVerdict::WillClose { rule: Some("explorer.exe".into()) });
    }

    #[test]
    fn free_and_unknown() {
        assert_eq!(verdict_for(&snap(Mode::Free, &[]), "game.exe"), ProcessVerdict::NotAffected { substring_rule: None, wildcard_rule: None });
        let mut s = snap(Mode::Whitelist, &[]);
        assert!(looks_locked(&s));
        s.sync = Read::Missing;
        assert!(matches!(verdict_for(&s, "game.exe"), ProcessVerdict::Unknown(_)));
    }

    #[test]
    fn wildcards() {
        assert!(image_matches("STEAM*", "steam.exe"));
        assert!(image_matches("*.scr", "x.scr"));
        assert!(!image_matches("code.exe", "vscode.exe"));
        assert!(image_matches("a*b*c", "aXXbYYc"));
    }
}
