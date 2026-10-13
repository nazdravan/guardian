use std::path::{Path, PathBuf};

use chrono::Utc;
use guardian_core::agent;
use guardian_core::parse;
use guardian_core::read::Read;
use guardian_core::snapshot::{DirSource, InstallDir, MachineInfo, ServiceInfo, Snapshot};

use crate::files::{read_source, read_source_fallible};

pub struct CollectOptions {
    pub manual_dir: Option<PathBuf>,
    pub probe_health: bool,
}

#[cfg(windows)]
pub fn io_path(p: &Path) -> PathBuf {
    crate::win::io_path(p)
}
#[cfg(not(windows))]
pub fn io_path(p: &Path) -> PathBuf {
    p.to_path_buf()
}

type Platform = (Read<ServiceInfo>, Option<String>, Read<Vec<String>>, Read<Vec<parse::AgentEvent>>, MachineInfo, bool);

#[cfg(windows)]
fn platform() -> Platform {
    let svc = crate::win::service_info();
    (svc, crate::win::image_path(), crate::win::tasklist(), crate::win::events(), crate::win::machine(), crate::win::is_elevated())
}
#[cfg(not(windows))]
fn platform() -> Platform {
    let na = || "only available on Windows".to_string();
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".into());
    (
        Read::Failed(na()),
        None,
        Read::Failed(na()),
        Read::Failed(na()),
        MachineInfo { hostname: host, os: std::env::consts::OS.into(), arch: std::env::consts::ARCH.into() },
        false,
    )
}

pub fn collect(opts: &CollectOptions) -> Snapshot {
    let (service, image, processes, events, machine, elevated) = platform();
    let (path, source) = match (&opts.manual_dir, &image) {
        (Some(d), _) => (d.clone(), DirSource::Manual),
        (None, Some(img)) if !img.is_empty() => (Path::new(img).parent().map(Path::to_path_buf).unwrap_or_default(), DirSource::Service),
        _ => (PathBuf::from(agent::DEFAULT_INSTALL_DIR), DirSource::Default),
    };
    let exists = io_path(&path).is_dir();
    let f = |name: &str| io_path(&path.join(name));

    #[cfg(windows)]
    let agent_version = crate::win::agent_version(image.as_deref().unwrap_or(""));
    #[cfg(not(windows))]
    let agent_version = Read::Failed("only available on Windows".into());

    let env = read_source(&f(agent::ENV_FILE), parse::env);
    let credentials = match crate::files::read_capped(&f(agent::CREDENTIALS_FILE)) {
        Read::Ok((b, _)) => parse::credentials(&b).map_or_else(Read::Failed, Read::Ok),
        Read::Missing => Read::Missing,
        Read::Denied => Read::Denied,
        Read::Failed(e) => Read::Failed(e),
    };
    let sync = read_source_fallible(&f(agent::SYNC_FILE), parse::sync);
    let whitelist = read_source(&f(agent::WHITELIST_FILE), parse::whitelist);

    let server = env.as_ok().and_then(|e| e.parsed.server_address.clone());
    let health = match (opts.probe_health, server) {
        (true, Some(s)) => Some(Read::Ok(crate::health::probe(&s, std::time::Duration::from_secs(5)))),
        _ => None,
    };

    Snapshot {
        taken_at: Utc::now(),
        machine,
        install: InstallDir { path, source, exists },
        elevated,
        service,
        agent_version,
        env,
        credentials,
        sync,
        whitelist,
        processes,
        events,
        health,
    }
}

#[cfg(not(windows))]
pub fn elevated() -> bool {
    false
}
#[cfg(not(windows))]
pub fn relaunch_elevated(_args: &[String]) -> Result<(), String> {
    Err("only available on Windows".into())
}
#[cfg(not(windows))]
pub fn restart_service(_progress: &dyn Fn(&str)) -> Result<(), String> {
    Err("only available on Windows".into())
}
#[cfg(windows)]
pub use crate::win::{is_elevated as elevated, relaunch_elevated, restart_service};

#[cfg(test)]
mod tests {
    use super::*;
    use guardian_core::snapshot::DirSource;

    #[test]
    fn manual_dir_reads_the_agent_files() {
        let dir = guardian_core::testkit::fixtures_dir().join("healthy");
        let s = collect(&CollectOptions { manual_dir: Some(dir.clone()), probe_health: false });
        assert_eq!(s.install.source, DirSource::Manual);
        assert!(s.install.exists);
        assert_eq!(s.env.as_ok().unwrap().parsed.server_address.as_deref(), Some("https://guardian.example.com"));
        assert_eq!(s.whitelist.as_ok().unwrap().parsed, vec!["notepad.exe", "calc.exe"]);
        assert!(s.sync.as_ok().unwrap().mtime.is_some());
        assert!(s.credentials.as_ok().is_some());
        assert!(s.health.is_none());
    }
}
