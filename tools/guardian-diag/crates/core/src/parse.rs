//! Parsers for every source the agent leaves behind. Each mirrors the agent's own reading
//! (see agent/client.go readEnvFile, agent/main.go loadWhitelist, extractProcessName).

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::agent;
use crate::read::{Encoding, LineEnding, SecretTail, detect_encoding, detect_line_ending};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvFile {
    /// As the agent sees it: last occurrence, trimmed, trailing '/' removed.
    pub server_address: Option<String>,
    /// Raw last value; the agent falls back to its default when it is not an int > 0.
    pub check_interval_raw: Option<String>,
    pub binding_token: Option<SecretTail>,
    /// Keys defined more than once, with the count.
    pub duplicates: Vec<(String, usize)>,
    /// Keys the agent ignores, with values masked when they look like secrets.
    pub unknown: Vec<(String, String)>,
    pub encoding: Encoding,
    pub line_ending: LineEnding,
}

impl EnvFile {
    pub fn effective_check_interval(&self) -> u32 {
        self.check_interval_raw
            .as_deref()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|v| *v > 0)
            .map(|v| v.min(u32::MAX as i64) as u32)
            .unwrap_or(agent::DEFAULT_SERVICE_INTERVAL_SECS)
    }
}

/// `KEY=value` lines exactly as the agent's readEnvFile splits them. Returns (key, value) pairs in file order.
pub fn env_pairs(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

fn looks_secret(key: &str) -> bool {
    let k = key.to_uppercase();
    ["TOKEN", "SECRET", "PASSWORD", "KEY"].iter().any(|s| k.contains(s))
}

pub fn env(bytes: &[u8]) -> EnvFile {
    let encoding = detect_encoding(bytes);
    let text = String::from_utf8_lossy(bytes);
    let pairs = env_pairs(&text);
    let mut e = EnvFile { encoding, line_ending: detect_line_ending(&text), ..Default::default() };
    let mut order: Vec<String> = Vec::new();
    let mut counts: std::collections::HashMap<String, usize> = Default::default();
    let mut last: std::collections::HashMap<String, String> = Default::default();
    for (k, v) in pairs {
        if !counts.contains_key(&k) {
            order.push(k.clone());
        }
        *counts.entry(k.clone()).or_default() += 1;
        last.insert(k, v);
    }
    for k in &order {
        let v = last.get(k).cloned().unwrap_or_default();
        let n = counts.get(k).copied().unwrap_or(0);
        if n > 1 {
            e.duplicates.push((k.clone(), n));
        }
        match k.as_str() {
            agent::KEY_SERVER_ADDRESS => e.server_address = Some(v.trim_end_matches('/').to_string()).filter(|s| !s.is_empty()),
            agent::KEY_CHECK_INTERVAL => e.check_interval_raw = Some(v),
            agent::KEY_BINDING_TOKEN => e.binding_token = Some(SecretTail::of(&v)).filter(|_| !v.is_empty()),
            _ => e.unknown.push((k.clone(), if looks_secret(k) { "••••".into() } else { v })),
        }
    }
    e
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub machine_guid: String,
    pub server_address: String,
    pub enrolled_at: String,
    pub token: Option<SecretTail>,
}

pub fn credentials(bytes: &[u8]) -> Result<Credentials, String> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        agent_token: String,
        #[serde(default)]
        machine_guid: String,
        #[serde(default)]
        server_address: String,
        #[serde(default)]
        enrolled_at: String,
    }
    let r: Raw = serde_json::from_slice(bytes).map_err(|e| format!("not valid JSON: {e}"))?;
    Ok(Credentials {
        machine_guid: r.machine_guid,
        server_address: r.server_address,
        enrolled_at: r.enrolled_at,
        token: Some(SecretTail::of(&r.agent_token)).filter(|_| !r.agent_token.is_empty()),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Blacklist,
    Whitelist,
    Free,
    Other(String),
}

impl Mode {
    pub fn as_str(&self) -> &str {
        match self {
            Mode::Blacklist => "blacklist",
            Mode::Whitelist => "whitelist",
            Mode::Free => "free",
            Mode::Other(s) => s,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct App {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncState {
    pub mode: Mode,
    pub applications: Vec<App>,
}

pub fn sync(bytes: &[u8]) -> Result<SyncState, String> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        applications: Option<Vec<App>>,
        #[serde(default)]
        mode: String,
    }
    let r: Raw = serde_json::from_slice(bytes).map_err(|e| format!("not valid JSON: {e}"))?;
    let mode = match r.mode.as_str() {
        "blacklist" => Mode::Blacklist,
        "whitelist" => Mode::Whitelist,
        "free" => Mode::Free,
        other => Mode::Other(other.to_string()),
    };
    Ok(SyncState { mode, applications: r.applications.unwrap_or_default() })
}

/// Entries as the agent's loadWhitelist sees them: trimmed, lower case, comments skipped.
pub fn whitelist(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes).split('\n').map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_lowercase).collect()
}

/// First quoted CSV field of each `tasklist /FO CSV /NH` line (agent's extractProcessName).
pub fn tasklist(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix('"')?;
            let end = rest.find('"')?;
            Some(rest[..end].to_string()).filter(|s| !s.is_empty())
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Level {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum KillReason {
    Blacklist,
    Whitelist,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum EventKind {
    Kill { process: String, reason: KillReason },
    Sync,
    Start,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AgentEvent {
    pub time: DateTime<Utc>,
    pub level: Level,
    pub message: String,
    pub kind: EventKind,
}

pub fn classify(message: &str) -> EventKind {
    if let Some(p) = message.strip_prefix(agent::KILL_WHITELIST_PREFIX) {
        EventKind::Kill { process: p.trim().to_string(), reason: KillReason::Whitelist }
    } else if let Some(p) = message.strip_prefix(agent::KILL_BLACKLIST_PREFIX) {
        EventKind::Kill { process: p.trim().to_string(), reason: KillReason::Blacklist }
    } else if message.starts_with("Synced:") {
        EventKind::Sync
    } else if message.starts_with(agent::START_MARKER) {
        EventKind::Start
    } else {
        EventKind::Other
    }
}

/// `wevtutil qe … /f:xml` output: concatenated <Event> elements without a root.
pub fn wevtutil(xml: &str) -> Result<Vec<AgentEvent>, String> {
    if xml.trim().is_empty() {
        return Ok(Vec::new());
    }
    let wrapped = format!("<Events>{xml}</Events>");
    let doc = roxmltree::Document::parse(&wrapped).map_err(|e| format!("unreadable event log output: {e}"))?;
    let mut out = Vec::new();
    for ev in doc.root_element().children().filter(|n| n.has_tag_name("Event")) {
        let find = |name: &str| ev.descendants().find(|n| n.has_tag_name(name));
        let time = find("TimeCreated")
            .and_then(|n| n.attribute("SystemTime"))
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc));
        let Some(time) = time else { continue };
        let level = match find("Level").and_then(|n| n.text()).unwrap_or("4") {
            "1" | "2" => Level::Error,
            "3" => Level::Warning,
            _ => Level::Info,
        };
        let message: String = ev.descendants().filter(|n| n.has_tag_name("Data")).filter_map(|n| n.text()).collect::<Vec<_>>().join(" ");
        let message = message.trim().to_string();
        let kind = classify(&message);
        out.push(AgentEvent { time, level, message, kind });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    fn fx(p: &str) -> Vec<u8> {
        std::fs::read(format!("{FX}/{p}")).unwrap()
    }

    #[test]
    fn env_reads_like_the_agent() {
        let e = env(b"# c\r\nSERVER_ADDRESS = https://a.example/ \r\nSERVER_ADDRESS=https://b.example//\r\nCHECK_INTERVAL=abc\r\nBINDING_TOKEN=tok\r\nTOKEN=old\r\nnoequals\r\n");
        assert_eq!(e.server_address.as_deref(), Some("https://b.example")); // last wins, trailing / trimmed
        assert_eq!(e.check_interval_raw.as_deref(), Some("abc"));
        assert_eq!(e.effective_check_interval(), 20);
        assert_eq!(e.binding_token, Some(SecretTail::of("tok")));
        assert_eq!(e.duplicates, vec![("SERVER_ADDRESS".to_string(), 2)]);
        assert_eq!(e.unknown, vec![("TOKEN".to_string(), "••••".to_string())]);
        assert_eq!(e.encoding, Encoding::Utf8);
        assert_eq!(e.line_ending, LineEnding::Crlf);
    }

    #[test]
    fn env_unknown_values_shown_unless_secret_like() {
        let e = env(b"PROXY=http://p:3128\nAPI_KEY=x\nPASSWORD=y\n");
        assert_eq!(e.unknown, vec![("PROXY".into(), "http://p:3128".into()), ("API_KEY".into(), "••••".into()), ("PASSWORD".into(), "••••".into())]);
    }

    #[test]
    fn env_bom_is_not_part_of_the_first_key() {
        // The agent would see "\u{feff}SERVER_ADDRESS"; we report the BOM as a problem
        // but parse the key as the agent does, so diagnosis matches reality.
        let e = env(b"\xEF\xBB\xBFSERVER_ADDRESS=https://a\n");
        assert_eq!(e.encoding, Encoding::Utf8Bom);
        assert_eq!(e.server_address, None);
        assert_eq!(e.unknown.len(), 1);
    }

    #[test]
    fn credentials_drop_the_token() {
        let c = credentials(&fx("healthy/agent_credentials.json")).unwrap();
        assert_eq!(c.server_address, "https://guardian.example.com");
        assert_eq!(c.machine_guid, "4c4c4544-0042-3510-8052-b4c04f4d4e31");
        assert_eq!(c.token, Some(SecretTail::of("agt_secret_value_123")));
        assert!(!format!("{c:?}").contains("agt_secret"));
        assert!(credentials(b"{not json").is_err());
    }

    #[test]
    fn sync_state() {
        let s = sync(&fx("healthy/sync.json")).unwrap();
        assert_eq!(s.mode, Mode::Whitelist);
        assert_eq!(s.applications.len(), 2);
        assert_eq!(sync(br#"{"applications":null,"mode":"free"}"#).unwrap().mode, Mode::Free);
        assert_eq!(sync(br#"{"mode":"weird"}"#).unwrap().mode, Mode::Other("weird".into()));
        assert!(sync(b"{").is_err());
    }

    #[test]
    fn whitelist_like_the_agent() {
        assert_eq!(whitelist(&fx("healthy/whitelist.txt")), vec!["notepad.exe", "calc.exe"]);
        assert_eq!(whitelist(b"  A.exe  \n#x\n\n"), vec!["a.exe"]);
    }

    #[test]
    fn tasklist_names() {
        let t = tasklist(std::str::from_utf8(&fx("tasklist.csv")).unwrap());
        assert_eq!(t, vec!["System Idle Process", "explorer.exe", "WINWORD.EXE", "game.exe", "Guardian.exe"]);
    }

    #[test]
    fn wevtutil_events() {
        let ev = wevtutil(std::str::from_utf8(&fx("wevtutil.xml")).unwrap()).unwrap();
        assert_eq!(ev.len(), 6);
        assert_eq!(ev[0].kind, EventKind::Kill { process: "game.exe".into(), reason: KillReason::Whitelist });
        assert_eq!(ev[1].level, Level::Warning);
        assert_eq!(ev[1].kind, EventKind::Other);
        assert_eq!(ev[2].kind, EventKind::Sync);
        assert_eq!(ev[3].kind, EventKind::Kill { process: "steam.exe".into(), reason: KillReason::Blacklist });
        assert_eq!(ev[4].kind, EventKind::Start);
        assert_eq!(ev[5].level, Level::Error);
        assert_eq!(ev[0].time.to_rfc3339(), "2026-10-10T11:14:05.123456700+00:00");
        assert_eq!(wevtutil("").unwrap(), vec![]);
        assert!(wevtutil("<Event><broken").is_err());
    }
}
