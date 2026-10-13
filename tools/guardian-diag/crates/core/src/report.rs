//! Text and JSON report (spec R24, R25). Built from parsed data only: secrets never reach it.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::diagnose::{Diagnosis, Status};
use crate::enforce::verdict_for;
use crate::parse::{AgentEvent, EventKind, KillReason};
use crate::snapshot::Snapshot;

pub struct Report {
    pub text: String,
    pub json: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillSummary {
    pub process: String,
    pub reason: KillReason,
    pub count: usize,
    pub first: DateTime<Utc>,
    pub last: DateTime<Utc>,
}

/// Kills grouped by (process, reason), most recent first.
pub fn kill_summary(events: &[AgentEvent]) -> Vec<KillSummary> {
    let mut map: BTreeMap<(String, String), KillSummary> = BTreeMap::new();
    for e in events {
        if let EventKind::Kill { process, reason } = &e.kind {
            let key = (process.to_lowercase(), format!("{reason:?}"));
            let k = map.entry(key).or_insert(KillSummary { process: process.clone(), reason: *reason, count: 0, first: e.time, last: e.time });
            k.count += 1;
            k.first = k.first.min(e.time);
            k.last = k.last.max(e.time);
        }
    }
    let mut v: Vec<KillSummary> = map.into_values().collect();
    v.sort_by_key(|k| std::cmp::Reverse(k.last));
    v
}

pub fn exit_code(d: &Diagnosis) -> i32 {
    match d.worst() {
        Status::Error => 11,
        Status::Warn => 10,
        _ => 0,
    }
}

pub fn report(s: &Snapshot, d: &Diagnosis) -> Report {
    let env = s.env.as_ok().map(|f| &f.parsed);
    let config = json!({
        "install_dir": s.install.path.display().to_string(),
        "install_dir_source": format!("{:?}", s.install.source),
        "server_address": env.and_then(|e| e.server_address.clone()),
        "check_interval": env.and_then(|e| e.check_interval_raw.clone()),
        "binding_token": env.map(|e| match &e.binding_token { Some(t) => format!("present (…{})", t.0), None => "absent".into() }),
        // Keys only: an unknown key's value could be anything, including a secret.
        "unknown_keys": env.map(|e| e.unknown.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>()),
        "credentials": s.credentials.as_ok().map(|c| json!({
            "machine_guid": c.machine_guid,
            "server_address": c.server_address,
            "enrolled_at": c.enrolled_at,
            "agent_token": match &c.token { Some(t) => format!("present (…{})", t.0), None => "absent".into() },
        })),
        "whitelist": s.whitelist.as_ok().map(|w| w.parsed.clone()),
    });
    let policy = s.sync.as_ok().map(|f| json!({"mode": f.parsed.mode.as_str(), "synced_at": f.mtime, "applications": f.parsed.applications.iter().map(|a| a.name.clone()).collect::<Vec<_>>()}));
    let processes: Vec<Value> =
        s.processes.as_ok().map(|p| p.iter().map(|n| json!({"name": n, "verdict": verdict_for(s, n).label()})).collect()).unwrap_or_default();
    let events: Vec<&AgentEvent> = s.events.as_ok().map(|e| e.iter().take(500).collect()).unwrap_or_default();
    let json = json!({
        "taken_at": s.taken_at,
        "machine": {"hostname": s.machine.hostname, "os": s.machine.os, "arch": s.machine.arch},
        "versions": {"agent": s.agent_version.as_ok(), "guardian": env!("CARGO_PKG_VERSION")},
        "verdict": d.verdict,
        "checks": d.checks,
        "config": config,
        "policy": policy,
        "processes": processes,
        "events": events,
    });

    let mut t = String::new();
    let mut line = |l: String| {
        t.push_str(&l);
        t.push_str("\r\n");
    };
    line(format!("Guardian report — {} — {}", s.machine.hostname, s.taken_at.to_rfc3339()));
    line(format!(
        "{} ({}) | agent {} | guardian {}",
        s.machine.os,
        s.machine.arch,
        s.agent_version.as_ok().map(String::as_str).unwrap_or("?"),
        env!("CARGO_PKG_VERSION")
    ));
    line(String::new());
    line(format!("VERDICT: {}", d.verdict.text));
    if !d.verdict.hint.is_empty() {
        line(format!("  {}", d.verdict.hint));
    }
    line(String::new());
    for c in &d.checks {
        line(format!("[{:?}] {}: {}", c.status, c.title, c.fact));
        if !c.todo.is_empty() && c.status != Status::Ok {
            line(format!("        -> {}", c.todo));
        }
    }
    line(String::new());
    line("CONFIGURATION".into());
    line(serde_json::to_string_pretty(&json["config"]).unwrap_or_default());
    line(String::new());
    line("POLICY".into());
    line(serde_json::to_string_pretty(&json["policy"]).unwrap_or_default());
    line(String::new());
    line("PROCESSES".into());
    for p in &processes {
        line(format!("  {:<32} {}", p["name"].as_str().unwrap_or(""), p["verdict"].as_str().unwrap_or("")));
    }
    line(String::new());
    line("EVENT LOG (newest first)".into());
    for e in &events {
        line(format!("  {}  {:?}  {}", e.time.to_rfc3339(), e.level, e.message));
    }
    Report { text: t, json }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnose::diagnose;
    use crate::read::Read;
    use crate::testkit::{fixtures_dir, snapshot_from_dir};

    #[test]
    fn report_has_no_secrets() {
        let mut s = snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap());
        // Put a binding token in .env too.
        s.env = Read::Ok(crate::read::SourceFile::new(
            b"SERVER_ADDRESS=https://guardian.example.com\nBINDING_TOKEN=bt_secret_xyz\n".to_vec(),
            None,
            crate::parse::env,
        ));
        let r = report(&s, &diagnose(&s));
        for secret in ["agt_secret_value_123", "bt_secret_xyz"] {
            assert!(!r.text.contains(secret));
            assert!(!r.json.to_string().contains(secret));
        }
        assert!(r.text.contains("LAB-A-05"));
        assert!(r.text.contains("game.exe"));
        assert_eq!(r.json["verdict"]["kind"], "On");
        assert_eq!(r.json["config"]["binding_token"], format!("present (…{})", crate::read::SecretTail::of("bt_secret_xyz").0));
    }

    #[test]
    fn exit_codes() {
        let s = snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap());
        let d = diagnose(&s);
        assert_eq!(exit_code(&d), 11); // fixture log has an error
    }

    #[test]
    fn kill_summary_groups() {
        let s = snapshot_from_dir(&fixtures_dir().join("healthy"), "2026-10-10T11:15:00Z".parse().unwrap());
        let k = kill_summary(s.events.as_ok().unwrap());
        assert_eq!(k.len(), 2);
        assert_eq!(k[0].process, "game.exe");
        assert_eq!(k[0].count, 1);
    }
}
