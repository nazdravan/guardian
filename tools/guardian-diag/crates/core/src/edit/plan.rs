//! Turns a validated draft into exactly the bytes to write (spec R14).

use std::path::PathBuf;

use super::render::{EnvChange, render_env, render_whitelist};
use super::{EnvEdit, Field, Issue, Severity, TokenEdit, WhitelistEdit, canonical_server_address, validate};
use crate::agent;
use crate::read::Read;
use crate::snapshot::Snapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Env,
    Whitelist,
}

impl FileKind {
    pub fn file_name(self) -> &'static str {
        match self {
            FileKind::Env => agent::ENV_FILE,
            FileKind::Whitelist => agent::WHITELIST_FILE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    Absent,
    Hash(String),
}

#[derive(Clone, PartialEq, Eq)]
pub struct FileWrite {
    pub kind: FileKind,
    /// Canonical path; winhost maps it to an I/O path.
    pub path: PathBuf,
    pub expected: Expected,
    pub new_bytes: Vec<u8>,
    pub summary: Vec<String>,
}

impl std::fmt::Debug for FileWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWrite")
            .field("kind", &self.kind)
            .field("path", &self.path)
            .field("expected", &self.expected)
            .field("len", &self.new_bytes.len())
            .field("summary", &self.summary)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WritePlan {
    pub files: Vec<FileWrite>,
}

impl WritePlan {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

pub fn env_changes(s: &Snapshot, e: &EnvEdit) -> Vec<EnvChange> {
    env_diff(s, e).0
}

/// The `.env` changes this draft makes relative to what the agent currently reads.
fn env_diff(s: &Snapshot, e: &EnvEdit) -> (Vec<EnvChange>, Vec<String>) {
    let cur = s.env.as_ok().map(|f| f.parsed.clone()).unwrap_or_default();
    let mut ch = Vec::new();
    let mut summary = Vec::new();
    let new_addr = canonical_server_address(&e.server_address);
    if cur.server_address.as_deref() != Some(new_addr.as_str()) {
        summary.push(format!("SERVER_ADDRESS: {} → {}", cur.server_address.as_deref().unwrap_or("(none)"), new_addr));
        ch.push(EnvChange::Set(agent::KEY_SERVER_ADDRESS, new_addr));
    }
    let new_ci = e.check_interval.trim();
    if cur.check_interval_raw.as_deref().unwrap_or("") != new_ci {
        let old = cur.check_interval_raw.as_deref().unwrap_or("(default)");
        if new_ci.is_empty() {
            summary.push(format!("CHECK_INTERVAL: {old} → (default)"));
            ch.push(EnvChange::Remove(agent::KEY_CHECK_INTERVAL));
        } else {
            summary.push(format!("CHECK_INTERVAL: {old} → {new_ci}"));
            ch.push(EnvChange::Set(agent::KEY_CHECK_INTERVAL, new_ci.to_string()));
        }
    }
    match &e.binding_token {
        TokenEdit::Keep => {}
        TokenEdit::Set(t) => {
            summary.push("BINDING_TOKEN: set".into());
            ch.push(EnvChange::Set(agent::KEY_BINDING_TOKEN, t.clone()));
        }
        TokenEdit::Remove => {
            summary.push("BINDING_TOKEN: removed".into());
            ch.push(EnvChange::Remove(agent::KEY_BINDING_TOKEN));
        }
    }
    (ch, summary)
}

fn source<T>(r: &Read<crate::read::SourceFile<T>>, field: Field) -> Result<(Vec<u8>, Expected), Issue> {
    match r {
        Read::Ok(f) => Ok((f.bytes.clone(), Expected::Hash(f.hash.clone()))),
        Read::Missing => Ok((Vec::new(), Expected::Absent)),
        other => Err(Issue { severity: Severity::Error, field, message: format!("Cannot edit: {}", other.problem().unwrap_or_default()) }),
    }
}

pub fn plan(s: &Snapshot, env: &EnvEdit, wl: &WhitelistEdit) -> Result<WritePlan, Vec<Issue>> {
    let mut errors: Vec<Issue> = validate(s, env, wl).into_iter().filter(|i| i.severity == Severity::Error).collect();
    // validate ignores entries; re-check what render_whitelist would write.
    for e in &wl.entries {
        if e.is_empty() || e.contains(['\n', '\r']) || e.starts_with('#') {
            errors.push(Issue {
                severity: Severity::Error,
                field: Field::Whitelist,
                message: format!("Invalid whitelist entry {e:?}: it cannot be empty, contain a line break or start with #."),
            });
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut files = Vec::new();

    let (changes, summary) = env_diff(s, env);
    if !changes.is_empty() {
        let (bytes, expected) = source(&s.env, Field::ServerAddress).map_err(|e| vec![e])?;
        let new_bytes = render_env(&bytes, &changes)
            .map_err(|e| vec![Issue { severity: Severity::Error, field: Field::ServerAddress, message: e.to_string() }])?;
        if new_bytes != bytes {
            files.push(FileWrite { kind: FileKind::Env, path: s.install.path.join(agent::ENV_FILE), expected, new_bytes, summary });
        }
    }

    let current = super::WhitelistEdit::from_snapshot(s).entries;
    if current != wl.entries || (matches!(s.whitelist, Read::Missing) && !wl.entries.is_empty()) {
        let (bytes, expected) = source(&s.whitelist, Field::Whitelist).map_err(|e| vec![e])?;
        let new_bytes = render_whitelist(&bytes, &wl.entries)
            .map_err(|e| vec![Issue { severity: Severity::Error, field: Field::Whitelist, message: e.to_string() }])?;
        let mut summary: Vec<String> = wl.entries.iter().filter(|e| !current.contains(e)).map(|e| format!("+ {e}")).collect();
        summary.extend(current.iter().filter(|c| !wl.entries.contains(c)).map(|c| format!("− {c}")));
        // A reorder or remove+re-add renders the same bytes: nothing to write, no needless .bak.
        if new_bytes != bytes {
            files.push(FileWrite { kind: FileKind::Whitelist, path: s.install.path.join(agent::WHITELIST_FILE), expected, new_bytes, summary });
        }
    }
    Ok(WritePlan { files })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{EnvEdit, TokenEdit, WhitelistEdit};
    use crate::read::Read;
    use crate::testkit::{fixtures_dir, snapshot_from_dir};

    fn snap() -> Snapshot {
        snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap())
    }

    #[test]
    fn unchanged_draft_writes_nothing() {
        let s = snap();
        let p = plan(&s, &EnvEdit::from_snapshot(&s), &WhitelistEdit::from_snapshot(&s)).unwrap();
        assert!(p.is_empty());
    }

    #[test]
    fn trailing_slash_only_is_not_a_change() {
        let s = snap();
        let e = EnvEdit { server_address: "https://guardian.example.com/".into(), ..EnvEdit::from_snapshot(&s) };
        assert!(plan(&s, &e, &WhitelistEdit::from_snapshot(&s)).unwrap().is_empty());
    }

    #[test]
    fn env_and_whitelist_changes() {
        let s = snap();
        let e = EnvEdit { check_interval: String::new(), binding_token: TokenEdit::Set("bt_new".into()), ..EnvEdit::from_snapshot(&s) };
        let mut wl = WhitelistEdit::from_snapshot(&s);
        wl.entries.retain(|x| x != "calc.exe");
        wl.entries.push("winword.exe".into());
        let p = plan(&s, &e, &wl).unwrap();
        assert_eq!(p.files.len(), 2);
        let env = &p.files[0];
        assert_eq!(env.kind, FileKind::Env);
        assert_eq!(env.expected, Expected::Hash(s.env.as_ok().unwrap().hash.clone()));
        assert_eq!(String::from_utf8(env.new_bytes.clone()).unwrap(), "SERVER_ADDRESS=https://guardian.example.com\r\nBINDING_TOKEN=bt_new\r\n");
        assert_eq!(env.summary, vec!["CHECK_INTERVAL: 30 → (default)".to_string(), "BINDING_TOKEN: set".to_string()]);
        assert!(!format!("{:?}", env.summary).contains("bt_new"));
        assert_eq!(p.files[1].summary, vec!["+ winword.exe".to_string(), "− calc.exe".to_string()]);
        assert_eq!(p.files[1].path, fixtures_dir().join("healthy").join("whitelist.txt"));
    }

    #[test]
    fn reorder_or_remove_and_readd_writes_nothing() {
        let s = snap();
        let mut wl = WhitelistEdit::from_snapshot(&s);
        wl.entries.reverse();
        assert!(plan(&s, &EnvEdit::from_snapshot(&s), &wl).unwrap().is_empty());
        let mut wl = WhitelistEdit::from_snapshot(&s);
        let first = wl.entries.remove(0);
        wl.entries.push(first);
        assert!(plan(&s, &EnvEdit::from_snapshot(&s), &wl).unwrap().is_empty());
    }

    #[test]
    fn missing_files_expect_absent() {
        let mut s = snap();
        s.whitelist = Read::Missing;
        let wl = WhitelistEdit { entries: vec!["a.exe".into()] };
        let p = plan(&s, &EnvEdit::from_snapshot(&s), &wl).unwrap();
        assert_eq!(p.files[0].expected, Expected::Absent);
    }

    #[test]
    fn errors_block_the_plan_and_unreadable_files_refuse() {
        let s = snap();
        let e = EnvEdit { server_address: "nope".into(), ..EnvEdit::from_snapshot(&s) };
        assert!(plan(&s, &e, &WhitelistEdit::from_snapshot(&s)).is_err());
        let mut s = snap();
        s.env = Read::Denied;
        assert!(plan(&s, &EnvEdit { check_interval: "9".into(), ..EnvEdit::from_snapshot(&snap()) }, &WhitelistEdit::from_snapshot(&s)).is_err());
    }

    #[test]
    fn bad_whitelist_entries_are_refused() {
        let s = snap();
        for bad in ["", "a\nb.exe", "x\r", "#c.exe"] {
            let mut wl = WhitelistEdit::from_snapshot(&s);
            wl.entries.push(bad.into());
            let err = plan(&s, &EnvEdit::from_snapshot(&s), &wl).unwrap_err();
            assert!(err.iter().any(|i| i.field == Field::Whitelist && i.severity == Severity::Error), "{bad:?}");
        }
    }
}
