//! Checks and the one-line verdict (spec R6, R7).

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::parse::{Level, Mode};
use crate::read::Read;
use crate::snapshot::{ServiceState, Snapshot, StartType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    Ok,
    Warn,
    Error,
    /// Could not check (no access, not probed). Never counts as a failure.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CheckId {
    Service,
    ServiceExe,
    AgentVersion,
    ServerAddress,
    Health,
    Enrollment,
    LeftoverToken,
    Sync,
    PendingRestart,
    AgentErrors,
    ServerRefusal,
    Clock,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Check {
    pub id: CheckId,
    pub title: &'static str,
    pub status: Status,
    pub fact: String,
    pub todo: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum VerdictKind {
    On,
    Stale,
    NotEnrolled,
    Stopped,
    NotInstalled,
    Free,
    NeverSynced,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Verdict {
    pub kind: VerdictKind,
    pub text: String,
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnosis {
    pub verdict: Verdict,
    pub checks: Vec<Check>,
}

impl Diagnosis {
    pub fn worst(&self) -> Status {
        if self.checks.iter().any(|c| c.status == Status::Error) {
            Status::Error
        } else if self.checks.iter().any(|c| c.status == Status::Warn) {
            Status::Warn
        } else {
            Status::Ok
        }
    }

    /// Colour of the verdict line: green only for On.
    pub fn worst_for_verdict(&self) -> Status {
        match self.verdict.kind {
            VerdictKind::On => Status::Ok,
            VerdictKind::Stale | VerdictKind::Free | VerdictKind::Unknown => Status::Warn,
            _ => Status::Error,
        }
    }
}

fn check(id: CheckId, title: &'static str, status: Status, fact: impl Into<String>, todo: impl Into<String>) -> Check {
    Check { id, title, status, fact: fact.into(), todo: todo.into() }
}

pub fn age(now: DateTime<Utc>, t: DateTime<Utc>) -> String {
    let s = (now - t).num_seconds().max(0);
    match s {
        0..=119 => format!("{s} s"),
        120..=7199 => format!("{} min", s / 60),
        7200..=172_799 => format!("{} h", s / 3600),
        _ => format!("{} days", s / 86_400),
    }
}

fn unknown<T>(r: &Read<T>) -> String {
    r.problem().unwrap_or_default()
}

/// Needles of the agent's refusal log lines (agent.go tryEnroll / sync, client.go errPlanLimit);
/// tests/agent_consistency.rs checks each one still exists in the Go source.
pub const REFUSALS: [(&str, &str); 3] = [
    ("token is no longer valid", "The installer's binding token was revoked or expired. Get a new installer token in the cabinet."),
    ("no longer accepts this computer's token", "The computer was unenrolled in the cabinet. Enrol it again with a new binding token."),
    ("plan limit reached", "The plan's computer limit is reached. Upgrade the plan or unenrol an unused computer."),
];

pub fn diagnose(s: &Snapshot) -> Diagnosis {
    let now = s.taken_at;
    let mut checks = Vec::new();

    // Service
    checks.push(match &s.service {
        Read::Ok(svc) => {
            let st = match (svc.state, svc.start_type) {
                (ServiceState::Running, StartType::Auto) => Status::Ok,
                (ServiceState::Running, _) => Status::Warn,
                _ => Status::Error,
            };
            check(
                CheckId::Service,
                "Service",
                st,
                format!("{:?}, start type {:?}", svc.state, svc.start_type),
                match st {
                    Status::Ok => "",
                    Status::Warn => "Set the service's start type to Automatic so protection survives a reboot.",
                    _ => "Start ProcSentinelAgent (services.msc) or restart it from the Configuration tab.",
                },
            )
        }
        Read::Missing => check(
            CheckId::Service,
            "Service",
            Status::Error,
            "ProcSentinelAgent is not registered",
            "Install the agent with the installer from the cabinet.",
        ),
        other => check(CheckId::Service, "Service", Status::Unknown, unknown(other), "Run as administrator to check the service."),
    });
    checks.push(match &s.service {
        Read::Ok(svc) if svc.exe_exists => check(CheckId::ServiceExe, "Agent executable", Status::Ok, svc.image_path.clone(), ""),
        Read::Ok(svc) => {
            check(CheckId::ServiceExe, "Agent executable", Status::Error, format!("{} does not exist", svc.image_path), "Reinstall the agent.")
        }
        _ => check(CheckId::ServiceExe, "Agent executable", Status::Unknown, "service not readable", ""),
    });
    checks.push(match &s.agent_version {
        Read::Ok(v) => check(CheckId::AgentVersion, "Agent version", Status::Ok, v.clone(), ""),
        other => check(CheckId::AgentVersion, "Agent version", Status::Warn, format!("could not determine: {}", unknown(other)), ""),
    });

    // Config
    let env = s.env.as_ok().map(|f| &f.parsed);
    checks.push(match (&s.env, env.and_then(|e| e.server_address.clone())) {
        (Read::Ok(_), Some(a)) => check(CheckId::ServerAddress, "SERVER_ADDRESS", Status::Ok, a, ""),
        (Read::Ok(_), None) | (Read::Missing, _) => {
            check(CheckId::ServerAddress, "SERVER_ADDRESS", Status::Error, "not set", "Set SERVER_ADDRESS on the Configuration tab.")
        }
        (other, _) => check(CheckId::ServerAddress, "SERVER_ADDRESS", Status::Unknown, unknown(other), "Run as administrator to read .env."),
    });
    checks.push(match &s.health {
        None => check(CheckId::Health, "Server responds", Status::Unknown, "not checked", ""),
        Some(Read::Ok(h)) if h.status == Some(200) => {
            check(CheckId::Health, "Server responds", Status::Ok, format!("200 in {} ms", h.latency_ms), "")
        }
        Some(Read::Ok(h)) => check(
            CheckId::Health,
            "Server responds",
            Status::Error,
            h.error.clone().unwrap_or_else(|| format!("HTTP {}", h.status.unwrap_or(0))),
            "Check the network and SERVER_ADDRESS. The agent keeps enforcing the last policy meanwhile.",
        ),
        Some(other) => check(CheckId::Health, "Server responds", Status::Unknown, unknown(other), ""),
    });

    // Enrollment
    let enrolled_here = match (&s.credentials, env.and_then(|e| e.server_address.as_deref())) {
        (Read::Ok(c), Some(addr)) if c.server_address.is_empty() || c.server_address == addr => Some(true),
        (Read::Ok(_), _) if matches!(s.env, Read::Ok(_) | Read::Missing) => Some(false),
        (Read::Ok(_), _) => None,
        (Read::Missing, _) => Some(false),
        _ => None,
    };
    checks.push(match (&s.credentials, enrolled_here) {
        (Read::Ok(c), Some(true)) => {
            check(CheckId::Enrollment, "Enrollment", Status::Ok, format!("enrolled since {} (machine {})", c.enrolled_at, c.machine_guid), "")
        }
        (Read::Ok(c), Some(false)) => check(
            CheckId::Enrollment,
            "Enrollment",
            Status::Error,
            format!(
                "credentials were issued by {}, but SERVER_ADDRESS is different; the agent treats this computer as not enrolled",
                c.server_address
            ),
            "Restore SERVER_ADDRESS, or enrol again with a new binding token.",
        ),
        (Read::Missing, _) => check(
            CheckId::Enrollment,
            "Enrollment",
            Status::Error,
            "not enrolled (no agent_credentials.json)",
            "Put a binding token from the cabinet into BINDING_TOKEN and restart the service.",
        ),
        (other, _) => check(CheckId::Enrollment, "Enrollment", Status::Unknown, unknown(other), "Run as administrator to read the credentials."),
    });
    checks.push(match (&s.env, env.and_then(|e| e.binding_token.clone()), enrolled_here) {
        (Read::Ok(_), Some(t), Some(true)) => check(
            CheckId::LeftoverToken,
            "Leftover BINDING_TOKEN",
            Status::Warn,
            format!("present (…{}) although enrolled; the agent ignores it", t.0),
            "Remove it on the Configuration tab.",
        ),
        (Read::Ok(_), Some(t), Some(false)) => {
            check(CheckId::LeftoverToken, "Leftover BINDING_TOKEN", Status::Ok, format!("present (…{}), waiting for enrollment", t.0), "")
        }
        (Read::Ok(_), Some(t), None) => check(CheckId::LeftoverToken, "Leftover BINDING_TOKEN", Status::Unknown, format!("present (…{})", t.0), ""),
        (Read::Ok(_) | Read::Missing, None, _) | (Read::Missing, _, _) => {
            check(CheckId::LeftoverToken, "Leftover BINDING_TOKEN", Status::Ok, "none", "")
        }
        (other, _, _) => {
            check(CheckId::LeftoverToken, "Leftover BINDING_TOKEN", Status::Unknown, unknown(other), "Run as administrator to read .env.")
        }
    });

    // Sync
    let interval = env.map(|e| e.effective_check_interval()).unwrap_or(crate::agent::DEFAULT_SERVICE_INTERVAL_SECS);
    let last_sync = s.sync.as_ok().and_then(|f| f.mtime);
    let stale = last_sync.map(|t| (now - t).num_seconds() > 3 * i64::from(interval));
    checks.push(match (&s.sync, last_sync, stale) {
        (Read::Ok(_), Some(t), Some(false)) => check(CheckId::Sync, "Last sync", Status::Ok, format!("{} ago", age(now, t)), ""),
        (Read::Ok(_), Some(t), _) => check(
            CheckId::Sync,
            "Last sync",
            Status::Warn,
            format!("{} ago (expected every {interval} s)", age(now, t)),
            "See Server responds and the Log tab for the reason.",
        ),
        (Read::Missing, _, _) => check(CheckId::Sync, "Last sync", Status::Error, "never synced (no sync.json)", "See Enrollment and the Log tab."),
        (Read::Ok(_), None, _) => check(CheckId::Sync, "Last sync", Status::Unknown, "time unknown", ""),
        (other, _, _) => {
            check(CheckId::Sync, "Last sync", if matches!(other, Read::Failed(_)) { Status::Warn } else { Status::Unknown }, unknown(other), "")
        }
    });

    // Pending restart (R7, R10)
    let started = s.service_started_at();
    let changed_after: Vec<&str> =
        [(crate::agent::ENV_FILE, s.env.as_ok().and_then(|f| f.mtime)), (crate::agent::WHITELIST_FILE, s.whitelist.as_ok().and_then(|f| f.mtime))]
            .into_iter()
            .filter(|(_, m)| matches!((m, started), (Some(m), Some(st)) if *m > st))
            .map(|(n, _)| n)
            .collect();
    checks.push(match started {
        None => {
            check(CheckId::PendingRestart, "Changes applied", Status::Unknown, "service start time unknown (no start record in the Event Log)", "")
        }
        Some(_) if changed_after.is_empty() => {
            check(CheckId::PendingRestart, "Changes applied", Status::Ok, "files unchanged since the service started", "")
        }
        Some(st) => check(
            CheckId::PendingRestart,
            "Changes applied",
            Status::Warn,
            format!("{} changed after the service started ({} ago)", changed_after.join(", "), age(now, st)),
            "Changes apply after the service restarts.",
        ),
    });

    // Event Log
    let day_ago = now - chrono::Duration::hours(24);
    let errors: Vec<String> = s
        .events
        .as_ok()
        .map(|ev| ev.iter().filter(|e| e.level == Level::Error && e.time >= day_ago).take(3).map(|e| e.message.clone()).collect())
        .unwrap_or_default();
    checks.push(match &s.events {
        Read::Ok(_) if errors.is_empty() => check(CheckId::AgentErrors, "Agent errors (24 h)", Status::Ok, "none", ""),
        Read::Ok(_) => check(CheckId::AgentErrors, "Agent errors (24 h)", Status::Error, errors.join(" | "), "See the Log tab."),
        other => check(CheckId::AgentErrors, "Agent errors (24 h)", Status::Unknown, unknown(other), ""),
    });
    let refusal = s
        .events
        .as_ok()
        .and_then(|ev| ev.iter().filter(|e| e.time >= day_ago).find_map(|e| REFUSALS.iter().find(|(needle, _)| e.message.contains(needle))));
    checks.push(match refusal {
        Some((needle, todo)) => check(CheckId::ServerRefusal, "Server refusal", Status::Error, format!("the server answered: {needle}"), *todo),
        None => match &s.events {
            Read::Ok(_) => check(CheckId::ServerRefusal, "Server refusal", Status::Ok, "none", ""),
            other => check(CheckId::ServerRefusal, "Server refusal", Status::Unknown, unknown(other), ""),
        },
    });

    // Clock
    checks.push(match s.health.as_ref().and_then(|h| h.as_ok()).and_then(|h| h.server_date) {
        Some(d) if (now - d).num_minutes().abs() > 5 => check(
            CheckId::Clock,
            "Clock",
            Status::Warn,
            format!("this computer is {} off the server", age(now.max(d), now.min(d))),
            "Fix the date and time; TLS and logs depend on them.",
        ),
        Some(_) => check(CheckId::Clock, "Clock", Status::Ok, "in sync with the server", ""),
        None => check(CheckId::Clock, "Clock", Status::Unknown, "not checked", ""),
    });

    let verdict = verdict(s, &checks, enrolled_here, last_sync, stale);
    Diagnosis { verdict, checks }
}

fn verdict(s: &Snapshot, _checks: &[Check], enrolled_here: Option<bool>, last_sync: Option<DateTime<Utc>>, stale: Option<bool>) -> Verdict {
    let v = |kind, text: String, hint: &str| Verdict { kind, text, hint: hint.to_string() };
    if matches!(s.service, Read::Missing) {
        if !s.install.exists {
            return v(
                VerdictKind::NotInstalled,
                "Not installed".into(),
                "Service not found, default folder is empty. Choose a folder to inspect a copy.",
            );
        }
        let hint = if s.is_manual_dir() {
            format!("The files in {} were inspected, but this computer has no ProcSentinelAgent service.", s.install.path.display())
        } else {
            format!(
                "The service ProcSentinelAgent is not registered; the agent's files are still in {}. Reinstall with the installer from the cabinet.",
                s.install.path.display()
            )
        };
        return v(VerdictKind::NotInstalled, "Not installed".into(), &hint);
    }
    if let Read::Ok(svc) = &s.service
        && !svc.exe_exists
    {
        return v(VerdictKind::NotInstalled, "Not installed".into(), "The service points at a missing executable. Reinstall the agent.");
    }
    if let Read::Ok(svc) = &s.service
        && svc.state != ServiceState::Running
    {
        return v(VerdictKind::Stopped, "Service stopped".into(), "Nothing is enforced while the service is stopped.");
    }
    if enrolled_here == Some(false) {
        return v(VerdictKind::NotEnrolled, "Not enrolled".into(), "The agent enforces the last saved policy but gets no updates.");
    }
    if matches!(s.sync, Read::Missing) {
        return v(VerdictKind::NeverSynced, "Never synced".into(), "See Enrollment and the Log tab.");
    }
    if let Some(f) = s.sync.as_ok()
        && f.parsed.mode == Mode::Free
    {
        return v(
            VerdictKind::Free,
            "Free mode: nothing is closed".into(),
            "The computer is not in a room, the room's protection is off, or the room was deleted. Check the cabinet.",
        );
    }
    if stale == Some(true) {
        let ago = last_sync.map(|t| age(s.taken_at, t)).unwrap_or_default();
        return v(
            VerdictKind::Stale,
            format!("Protection is on with a stale policy (no contact with the server for {ago})"),
            "See Server responds and the Log tab.",
        );
    }
    if stale == Some(false) {
        return v(VerdictKind::On, "Protection is on".into(), "");
    }
    v(VerdictKind::Unknown, "Could not check".into(), "Run as administrator to read everything.")
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::Read;
    use crate::snapshot::*;
    use crate::testkit::{fixtures_dir, snapshot_from_dir};

    fn now() -> chrono::DateTime<chrono::Utc> {
        "2026-10-10T11:15:00Z".parse().unwrap()
    }
    fn healthy() -> Snapshot {
        snapshot_from_dir(&fixtures_dir().join("healthy"), now())
    }
    fn status(d: &Diagnosis, id: CheckId) -> Status {
        d.checks.iter().find(|c| c.id == id).unwrap().status
    }

    #[test]
    fn diagnose_healthy() {
        let d = diagnose(&healthy());
        assert_eq!(d.verdict.kind, VerdictKind::On);
        assert_eq!(status(&d, CheckId::Sync), Status::Ok);
        assert_eq!(status(&d, CheckId::Health), Status::Unknown); // not probed
    }

    #[test]
    fn diagnose_not_installed() {
        let mut s = healthy();
        s.service = Read::Missing;
        s.install.exists = false;
        s.install.source = DirSource::Default;
        s.env = Read::Missing;
        s.sync = Read::Missing;
        s.credentials = Read::Missing;
        let d = diagnose(&s);
        assert_eq!(d.verdict.kind, VerdictKind::NotInstalled);
    }

    #[test]
    fn diagnose_denied_is_unknown() {
        let mut s = healthy();
        s.env = Read::Denied;
        s.credentials = Read::Denied;
        let d = diagnose(&s);
        assert_eq!(status(&d, CheckId::ServerAddress), Status::Unknown);
        assert_eq!(status(&d, CheckId::Enrollment), Status::Unknown);
    }

    #[test]
    fn diagnose_stopped_and_stale_and_free() {
        let mut s = healthy();
        if let Read::Ok(svc) = &mut s.service {
            svc.state = ServiceState::Stopped;
        }
        assert_eq!(diagnose(&s).verdict.kind, VerdictKind::Stopped);

        let mut s = healthy();
        if let Read::Ok(f) = &mut s.sync {
            f.mtime = Some(now() - chrono::Duration::hours(30));
        }
        let d = diagnose(&s);
        assert_eq!(d.verdict.kind, VerdictKind::Stale);
        assert!(d.verdict.text.contains("30 h"));

        let mut s = healthy();
        if let Read::Ok(f) = &mut s.sync {
            f.parsed.mode = crate::parse::Mode::Free;
        }
        assert_eq!(diagnose(&s).verdict.kind, VerdictKind::Free);
    }

    #[test]
    fn diagnose_server_mismatch_means_not_enrolled() {
        let mut s = healthy();
        if let Read::Ok(e) = &mut s.env {
            e.parsed.server_address = Some("https://other.example".into());
        }
        let d = diagnose(&s);
        assert_eq!(status(&d, CheckId::Enrollment), Status::Error);
        assert_eq!(d.verdict.kind, VerdictKind::NotEnrolled);
    }

    #[test]
    fn diagnose_pending_restart() {
        let mut s = healthy();
        if let Read::Ok(w) = &mut s.whitelist {
            w.mtime = Some(now() - chrono::Duration::minutes(1)); // after the 11:10 start
        }
        assert_eq!(status(&diagnose(&s), CheckId::PendingRestart), Status::Warn);
    }

    #[test]
    fn diagnose_server_refusal_from_log() {
        let d = diagnose(&healthy()); // fixture has "no longer accepts this computer's token"
        assert_eq!(status(&d, CheckId::ServerRefusal), Status::Error);
        assert_eq!(status(&d, CheckId::AgentErrors), Status::Error);
    }

    #[test]
    fn diagnose_clock_skew() {
        let mut s = healthy();
        s.health =
            Some(Read::Ok(HealthProbe { status: Some(200), latency_ms: 40, server_date: Some(now() - chrono::Duration::minutes(9)), error: None }));
        assert_eq!(status(&diagnose(&s), CheckId::Clock), Status::Warn);
        assert_eq!(status(&diagnose(&s), CheckId::Health), Status::Ok);
    }

    #[test]
    fn diagnose_leftover_token() {
        let mut s = healthy();
        if let Read::Ok(e) = &mut s.env {
            e.parsed.binding_token = Some(crate::read::SecretTail::of("t"));
        }
        assert_eq!(status(&diagnose(&s), CheckId::LeftoverToken), Status::Warn);
    }

    fn check_of(d: &Diagnosis, id: CheckId) -> &Check {
        d.checks.iter().find(|c| c.id == id).unwrap()
    }
    fn set_events(s: &mut Snapshot, f: impl Fn(&mut crate::parse::AgentEvent)) {
        if let Read::Ok(ev) = &mut s.events {
            ev.iter_mut().for_each(f);
        }
    }

    #[test]
    fn verdict_service_missing_folder_exists() {
        let mut s = healthy();
        s.service = Read::Missing;
        let d = diagnose(&s);
        assert_eq!(d.verdict.kind, VerdictKind::NotInstalled);
        assert!(d.verdict.hint.contains("not registered"));
        assert!(d.verdict.hint.contains("Reinstall"));
        s.install.source = DirSource::Manual;
        let d = diagnose(&s);
        assert_eq!(d.verdict.kind, VerdictKind::NotInstalled);
        assert!(d.verdict.hint.contains("no ProcSentinelAgent service"));
        assert_eq!(status(&d, CheckId::Service), Status::Error);
    }

    #[test]
    fn verdict_running_with_missing_exe() {
        let mut s = healthy();
        if let Read::Ok(svc) = &mut s.service {
            svc.exe_exists = false;
        }
        let d = diagnose(&s);
        assert_eq!(d.verdict.kind, VerdictKind::NotInstalled);
        assert!(d.verdict.hint.contains("missing executable"));
        assert_eq!(status(&d, CheckId::ServiceExe), Status::Error);
    }

    #[test]
    fn service_not_auto_is_warn() {
        let mut s = healthy();
        if let Read::Ok(svc) = &mut s.service {
            svc.start_type = StartType::Manual;
        }
        assert_eq!(status(&diagnose(&s), CheckId::Service), Status::Warn);
    }

    #[test]
    fn token_present_before_enrollment_is_ok() {
        let mut s = healthy();
        s.credentials = Read::Missing;
        if let Read::Ok(e) = &mut s.env {
            e.parsed.binding_token = Some(crate::read::SecretTail::of("t"));
        }
        let c = diagnose(&s);
        let c = check_of(&c, CheckId::LeftoverToken);
        assert_eq!(c.status, Status::Ok);
        assert!(c.fact.contains("waiting for enrollment"));
    }

    #[test]
    fn token_with_unknown_enrollment_is_unknown() {
        let mut s = healthy();
        s.credentials = Read::Denied;
        if let Read::Ok(e) = &mut s.env {
            e.parsed.binding_token = Some(crate::read::SecretTail::of("t"));
        }
        let d = diagnose(&s);
        let c = check_of(&d, CheckId::LeftoverToken);
        assert_eq!(c.status, Status::Unknown);
        assert!(c.fact.starts_with("present"));
    }

    #[test]
    fn token_with_unreadable_env_and_absent_token() {
        let mut s = healthy();
        s.env = Read::Denied;
        let d = diagnose(&s);
        let c = check_of(&d, CheckId::LeftoverToken);
        assert_eq!(c.status, Status::Unknown);
        assert_eq!(c.fact, "no access");
        // credentials are readable but .env is not: enrollment is unknown, not "not enrolled"
        assert_eq!(status(&d, CheckId::Enrollment), Status::Unknown);
        let d = diagnose(&healthy());
        let c = check_of(&d, CheckId::LeftoverToken);
        assert_eq!((c.status, c.fact.as_str()), (Status::Ok, "none"));
    }

    #[test]
    fn refusal_with_unreadable_events_is_unknown() {
        let mut s = healthy();
        s.events = Read::Denied;
        let d = diagnose(&s);
        let c = check_of(&d, CheckId::ServerRefusal);
        assert_eq!((c.status, c.fact.as_str()), (Status::Unknown, "no access"));
        assert_eq!(status(&d, CheckId::AgentErrors), Status::Unknown);
    }

    #[test]
    fn old_errors_and_refusals_are_ignored() {
        let mut s = healthy();
        set_events(&mut s, |e| e.time -= chrono::Duration::hours(30));
        let d = diagnose(&s);
        assert_eq!(status(&d, CheckId::AgentErrors), Status::Ok);
        assert_eq!(status(&d, CheckId::ServerRefusal), Status::Ok);
    }

    #[test]
    fn never_synced() {
        let mut s = healthy();
        s.sync = Read::Missing;
        let d = diagnose(&s);
        assert_eq!(status(&d, CheckId::Sync), Status::Error);
        assert_eq!(d.verdict.kind, VerdictKind::NeverSynced);
    }

    #[test]
    fn verdict_unknown_when_sync_denied() {
        let mut s = healthy();
        s.sync = Read::Denied;
        let d = diagnose(&s);
        assert_eq!(d.verdict.kind, VerdictKind::Unknown);
        assert_eq!(status(&d, CheckId::Sync), Status::Unknown);
    }

    #[test]
    fn health_failures_and_clock_ok() {
        let mut s = healthy();
        s.health = Some(Read::Ok(HealthProbe { status: Some(503), latency_ms: 10, server_date: None, error: None }));
        assert_eq!(status(&diagnose(&s), CheckId::Health), Status::Error);
        s.health = Some(Read::Ok(HealthProbe { status: None, latency_ms: 0, server_date: None, error: Some("timeout".into()) }));
        let d = diagnose(&s);
        assert_eq!(status(&d, CheckId::Health), Status::Error);
        assert_eq!(check_of(&d, CheckId::Health).fact, "timeout");
        s.health =
            Some(Read::Ok(HealthProbe { status: Some(200), latency_ms: 5, server_date: Some(now() - chrono::Duration::minutes(1)), error: None }));
        assert_eq!(status(&diagnose(&s), CheckId::Clock), Status::Ok);
    }

    #[test]
    fn pending_restart_unknown_without_start_record() {
        let mut s = healthy();
        if let Read::Ok(ev) = &mut s.events {
            ev.retain(|e| e.kind != crate::parse::EventKind::Start);
        }
        assert_eq!(status(&diagnose(&s), CheckId::PendingRestart), Status::Unknown);
    }

    #[test]
    fn stale_boundary_is_three_intervals() {
        let interval = healthy().env.as_ok().unwrap().parsed.effective_check_interval() as i64;
        for (offset, stale) in [(3 * interval, false), (3 * interval + 1, true)] {
            let mut s = healthy();
            if let Read::Ok(f) = &mut s.sync {
                f.mtime = Some(now() - chrono::Duration::seconds(offset));
            }
            let d = diagnose(&s);
            assert_eq!(d.verdict.kind == VerdictKind::Stale, stale, "offset {offset}");
            assert_eq!(status(&d, CheckId::Sync), if stale { Status::Warn } else { Status::Ok });
        }
    }
    #[test]
    fn plan_limit_refusal_is_recognised() {
        let mut s = healthy();
        set_events(&mut s, |e| {
            e.message = "Enrollment refused: plan limit reached: This account has reached its computer limit. Upgrade the plan in the cabinet; the agent retries on its own every 1m0s.".into()
        });
        let d = diagnose(&s);
        let c = check_of(&d, CheckId::ServerRefusal);
        assert_eq!((c.status, c.fact.as_str()), (Status::Error, "the server answered: plan limit reached"));
    }
}
