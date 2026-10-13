//! The edit model (spec R11–R13): pure, no I/O. The UI holds an `EnvEdit` and a
//! `WhitelistEdit` as its draft; `plan` (plan.rs) turns them into bytes to write.

pub mod plan;
pub mod render;

use crate::agent;
use crate::snapshot::Snapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenEdit {
    Keep,
    Set(String),
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvEdit {
    pub server_address: String,
    /// Raw field text; empty = remove the key (agent default applies).
    pub check_interval: String,
    pub binding_token: TokenEdit,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WhitelistEdit {
    pub entries: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    ServerAddress,
    CheckInterval,
    BindingToken,
    Whitelist,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub field: Field,
    pub message: String,
}

fn err(field: Field, m: impl Into<String>) -> Issue {
    Issue { severity: Severity::Error, field, message: m.into() }
}
fn warn(field: Field, m: impl Into<String>) -> Issue {
    Issue { severity: Severity::Warning, field, message: m.into() }
}

impl EnvEdit {
    pub fn from_snapshot(s: &Snapshot) -> Self {
        let e = s.env.as_ok().map(|f| &f.parsed);
        EnvEdit {
            server_address: e.and_then(|e| e.server_address.clone()).unwrap_or_default(),
            check_interval: e.and_then(|e| e.check_interval_raw.clone()).unwrap_or_default(),
            binding_token: TokenEdit::Keep,
        }
    }
}

impl WhitelistEdit {
    pub fn from_snapshot(s: &Snapshot) -> Self {
        let mut entries: Vec<String> = Vec::new();
        for e in s.whitelist.as_ok().map(|f| f.parsed.clone()).unwrap_or_default() {
            if !entries.contains(&e) {
                entries.push(e);
            }
        }
        WhitelistEdit { entries }
    }
}

/// tools/whitelist-gui normalizeName: trim, lower case, strip quotes, keep only the file name.
pub fn normalize_name(s: &str) -> String {
    let s = s.trim().to_lowercase();
    let s = s.trim_matches(|c| c == '"' || c == '\'');
    match s.rfind(['\\', '/']) {
        Some(i) => s[i + 1..].to_string(),
        None => s.to_string(),
    }
}

pub fn check_new_entry(current: &[String], raw: &str) -> Result<(String, Vec<Issue>), Issue> {
    if raw.contains(['\n', '\r']) {
        return Err(err(Field::Whitelist, "A name cannot contain a line break."));
    }
    let name = normalize_name(raw);
    if name.is_empty() {
        return Err(err(Field::Whitelist, "Type a program name, e.g. winword.exe."));
    }
    if name.starts_with('#') {
        return Err(err(Field::Whitelist, "A name starting with # would be read as a comment."));
    }
    if current.iter().any(|c| c.eq_ignore_ascii_case(&name)) {
        return Err(err(Field::Whitelist, format!("{name} is already in the list.")));
    }
    let mut w = Vec::new();
    if !name.ends_with(".exe") {
        w.push(warn(Field::Whitelist, "tasklist reports names with .exe; without it this entry may never match."));
    }
    if agent::is_protected(&name) {
        w.push(warn(Field::Whitelist, format!("{name} is already protected by the agent; the entry changes nothing.")));
    }
    Ok((name, w))
}

pub fn canonical_server_address(v: &str) -> String {
    v.trim().trim_end_matches('/').to_string()
}

/// R13: anything the agent's TrimSpace or comment rule would alter is refused.
fn env_value_problem(v: &str) -> Option<&'static str> {
    if v.contains(['\n', '\r']) {
        Some("cannot contain a line break")
    } else if v.starts_with('#') {
        Some("cannot start with #")
    } else if v != v.trim() {
        Some("cannot start or end with a space")
    } else {
        None
    }
}

fn server_address_problem(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return Some("SERVER_ADDRESS is required.".into());
    }
    if let Some(p) = env_value_problem(raw) {
        return Some(format!("SERVER_ADDRESS {p}."));
    }
    let v = canonical_server_address(raw);
    let rest = v.strip_prefix("https://").or_else(|| v.strip_prefix("http://"));
    let Some(rest) = rest else { return Some("SERVER_ADDRESS must start with http:// or https://.".into()) };
    if rest.contains(['/', '?', '#', ' ', '\t']) {
        return Some("SERVER_ADDRESS is a server, not a page: no path, query or spaces.".into());
    }
    // IPv6 literals are out of scope: schools use host names or IPv4.
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (rest, None),
    };
    if host.is_empty() {
        return Some("SERVER_ADDRESS has no host.".into());
    }
    if let Some(p) = port
        && !matches!(p.parse::<u32>(), Ok(1..=65535))
    {
        return Some("The port must be a number from 1 to 65535.".into());
    }
    None
}

pub fn validate(s: &Snapshot, env: &EnvEdit, _wl: &WhitelistEdit) -> Vec<Issue> {
    let mut out = Vec::new();
    if let Some(m) = server_address_problem(&env.server_address) {
        out.push(err(Field::ServerAddress, m));
    }
    let ci = env.check_interval.trim();
    if env.check_interval != ci {
        out.push(err(Field::CheckInterval, "CHECK_INTERVAL cannot start or end with a space."));
    } else if !ci.is_empty() {
        match ci.parse::<u32>() {
            Ok(n @ 1..=86_400) => {
                if n < 5 {
                    out.push(warn(Field::CheckInterval, "A short interval adds load on the server."));
                }
            }
            _ => out.push(err(Field::CheckInterval, "CHECK_INTERVAL must be a whole number of seconds from 1 to 86400.")),
        }
    }
    if let TokenEdit::Set(t) = &env.binding_token {
        if t.is_empty() {
            out.push(err(Field::BindingToken, "The new binding token is empty."));
        } else if let Some(p) = env_value_problem(t) {
            out.push(err(Field::BindingToken, format!("BINDING_TOKEN {p}.")));
        }
    }

    let new_addr = canonical_server_address(&env.server_address);
    if let Some(c) = s.credentials.as_ok() {
        let enrolled_to_new = c.server_address.is_empty() || c.server_address == new_addr;
        let token_set = matches!(env.binding_token, TokenEdit::Set(_));
        if !enrolled_to_new && !token_set {
            out.push(warn(Field::ServerAddress, "This computer will drop out of the fleet until it gets a new binding token."));
        }
        if enrolled_to_new && token_set {
            out.push(warn(
                Field::BindingToken,
                "The agent ignores a binding token while this computer is enrolled. Unenrol it in the cabinet first, then save the token again.",
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{fixtures_dir, snapshot_from_dir};

    fn snap() -> Snapshot {
        snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap())
    }
    fn errors(v: &[Issue]) -> Vec<Field> {
        v.iter().filter(|i| i.severity == Severity::Error).map(|i| i.field).collect()
    }
    fn warns(v: &[Issue]) -> usize {
        v.iter().filter(|i| i.severity == Severity::Warning).count()
    }

    #[test]
    fn normalize_cases() {
        assert_eq!(normalize_name(r#"  "C:\Program Files\Microsoft Office\WINWORD.EXE"  "#), "winword.exe");
        assert_eq!(normalize_name("'Notepad.exe'"), "notepad.exe");
        assert_eq!(normalize_name("/usr/bin/foo"), "foo");
    }

    #[test]
    fn check_new_entry_cases() {
        let cur = vec!["notepad.exe".to_string()];
        assert_eq!(check_new_entry(&cur, "WINWORD.EXE").unwrap(), ("winword.exe".to_string(), vec![]));
        let (n, w) = check_new_entry(&cur, "notepad2").unwrap();
        assert_eq!(n, "notepad2");
        assert_eq!(w.len(), 1); // no .exe
        let (_, w) = check_new_entry(&cur, "explorer.exe").unwrap();
        assert!(w[0].message.contains("already protected"));
        assert!(check_new_entry(&cur, "Notepad.EXE").is_err()); // duplicate
        assert!(check_new_entry(&cur, "   ").is_err());
        assert!(check_new_entry(&cur, "#x.exe").is_err());
        assert!(check_new_entry(&cur, "a\nb.exe").is_err());
    }

    #[test]
    fn from_snapshot_roundtrips_current_values() {
        let s = snap();
        let e = EnvEdit::from_snapshot(&s);
        assert_eq!(e.server_address, "https://guardian.example.com");
        assert_eq!(e.check_interval, "30");
        assert_eq!(e.binding_token, TokenEdit::Keep);
        assert!(validate(&s, &e, &WhitelistEdit::from_snapshot(&s)).is_empty());
    }

    #[test]
    fn server_address_rules() {
        let s = snap();
        let wl = WhitelistEdit::from_snapshot(&s);
        let with = |a: &str| EnvEdit { server_address: a.into(), ..EnvEdit::from_snapshot(&s) };
        for bad in [
            "",
            "guardian.example.com",
            "ftp://x",
            "https://",
            "https://x/api",
            "https://x?y",
            "https://x:99999",
            "https://x:ab",
            " https://x",
            "https://x y",
            "#https://x",
        ] {
            assert_eq!(errors(&validate(&s, &with(bad), &wl)), vec![Field::ServerAddress], "{bad:?}");
        }
        for good in ["https://guardian.example.com/", "http://10.0.0.5:8080", "https://guardian.example.com"] {
            assert!(errors(&validate(&s, &with(good), &wl)).is_empty(), "{good:?}");
        }
        assert_eq!(canonical_server_address("https://a.example//"), "https://a.example");
    }

    #[test]
    fn server_change_without_token_warns_drop_out() {
        let s = snap();
        let e = EnvEdit { server_address: "https://other.example".into(), ..EnvEdit::from_snapshot(&s) };
        let v = validate(&s, &e, &WhitelistEdit::from_snapshot(&s));
        assert!(v.iter().any(|i| i.severity == Severity::Warning && i.message.contains("drop out of the fleet")));
        let e = EnvEdit { binding_token: TokenEdit::Set("tok".into()), ..e };
        assert!(!validate(&s, &e, &WhitelistEdit::from_snapshot(&s)).iter().any(|i| i.message.contains("drop out")));
    }

    #[test]
    fn token_while_enrolled_warns_ignored() {
        let s = snap();
        let e = EnvEdit { binding_token: TokenEdit::Set("tok".into()), ..EnvEdit::from_snapshot(&s) };
        let v = validate(&s, &e, &WhitelistEdit::from_snapshot(&s));
        assert!(v.iter().any(|i| i.field == Field::BindingToken && i.message.contains("Unenrol it in the cabinet first")));
        let e = EnvEdit { binding_token: TokenEdit::Set(" tok".into()), ..EnvEdit::from_snapshot(&s) };
        assert_eq!(errors(&validate(&s, &e, &WhitelistEdit::from_snapshot(&s))), vec![Field::BindingToken]);
    }

    #[test]
    fn check_interval_rules() {
        let s = snap();
        let wl = WhitelistEdit::from_snapshot(&s);
        let with = |v: &str| EnvEdit { check_interval: v.into(), ..EnvEdit::from_snapshot(&s) };
        assert!(validate(&s, &with(""), &wl).is_empty());
        assert_eq!(errors(&validate(&s, &with("0"), &wl)), vec![Field::CheckInterval]);
        assert_eq!(errors(&validate(&s, &with("86401"), &wl)), vec![Field::CheckInterval]);
        assert_eq!(errors(&validate(&s, &with("ten"), &wl)), vec![Field::CheckInterval]);
        assert_eq!(warns(&validate(&s, &with("3"), &wl)), 1);
    }
}
