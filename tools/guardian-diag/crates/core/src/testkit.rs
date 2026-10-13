//! Snapshots from fixture folders, for tests in this crate and downstream crates.

use std::path::Path;

use chrono::{DateTime, Utc};

use crate::parse;
use crate::read::{Read, SourceFile};
use crate::snapshot::*;

fn file<T>(dir: &Path, name: &str, mtime: DateTime<Utc>, f: impl FnOnce(&[u8]) -> T) -> Read<SourceFile<T>> {
    match std::fs::read(dir.join(name)) {
        Ok(b) => Read::Ok(SourceFile::new(b, Some(mtime), f)),
        Err(_) => Read::Missing,
    }
}

pub fn fixtures_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub fn snapshot_from_dir(dir: &Path, now: DateTime<Utc>) -> Snapshot {
    let fx = fixtures_dir();
    let tasklist = std::fs::read_to_string(fx.join("tasklist.csv")).unwrap_or_default();
    let events = std::fs::read_to_string(fx.join("wevtutil.xml")).unwrap_or_default();
    let recent = now - chrono::Duration::seconds(12);
    Snapshot {
        taken_at: now,
        machine: MachineInfo { hostname: "LAB-A-05".into(), os: "Windows 10 Pro 19045".into(), arch: "x64".into() },
        install: InstallDir { path: dir.to_path_buf(), source: DirSource::Service, exists: true },
        elevated: true,
        service: Read::Ok(ServiceInfo {
            state: ServiceState::Running,
            start_type: StartType::Auto,
            image_path: dir.join("procsentinel-agent64.exe").display().to_string(),
            exe_exists: true,
        }),
        agent_version: Read::Ok("3.0.0".into()),
        env: file(dir, crate::agent::ENV_FILE, recent - chrono::Duration::days(3), parse::env),
        credentials: match std::fs::read(dir.join(crate::agent::CREDENTIALS_FILE)) {
            Ok(b) => parse::credentials(&b).map_or_else(Read::Failed, Read::Ok),
            Err(_) => Read::Missing,
        },
        sync: match std::fs::read(dir.join(crate::agent::SYNC_FILE)) {
            Ok(b) => match parse::sync(&b) {
                Ok(s) => Read::Ok(SourceFile::new(b, Some(recent), |_| s)),
                Err(e) => Read::Failed(e),
            },
            Err(_) => Read::Missing,
        },
        whitelist: file(dir, crate::agent::WHITELIST_FILE, recent - chrono::Duration::days(3), parse::whitelist),
        processes: Read::Ok(parse::tasklist(&tasklist)),
        events: parse::wevtutil(&events).map_or_else(Read::Failed, Read::Ok),
        health: None,
    }
}
