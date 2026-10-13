use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::parse::{AgentEvent, Credentials, EnvFile, SyncState};
use crate::read::{Read, SourceFile};

#[derive(Debug, Clone, PartialEq)]
pub struct MachineInfo {
    pub hostname: String,
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirSource {
    Service,
    Default,
    Manual,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstallDir {
    /// Canonical (System32) path, as shown to the user.
    pub path: PathBuf,
    pub source: DirSource,
    pub exists: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Running,
    Stopped,
    Pending,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartType {
    Auto,
    Manual,
    Disabled,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceInfo {
    pub state: ServiceState,
    pub start_type: StartType,
    pub image_path: String,
    pub exe_exists: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HealthProbe {
    pub status: Option<u16>,
    pub latency_ms: u64,
    pub server_date: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub taken_at: DateTime<Utc>,
    pub machine: MachineInfo,
    pub install: InstallDir,
    pub elevated: bool,
    pub service: Read<ServiceInfo>,
    pub agent_version: Read<String>,
    pub env: Read<SourceFile<EnvFile>>,
    pub credentials: Read<Credentials>,
    pub sync: Read<SourceFile<SyncState>>,
    pub whitelist: Read<SourceFile<Vec<String>>>,
    pub processes: Read<Vec<String>>,
    pub events: Read<Vec<AgentEvent>>,
    /// None = not probed (`report --offline`, or no SERVER_ADDRESS).
    pub health: Option<Read<HealthProbe>>,
}

impl Snapshot {
    /// Last service start, from the agent's start marker in the Event Log.
    pub fn service_started_at(&self) -> Option<DateTime<Utc>> {
        self.events.as_ok()?.iter().filter(|e| e.kind == crate::parse::EventKind::Start).map(|e| e.time).max()
    }
    /// Manual folder: files from there, service and log from this machine. No service restart offered.
    pub fn is_manual_dir(&self) -> bool {
        self.install.source == DirSource::Manual
    }
}
