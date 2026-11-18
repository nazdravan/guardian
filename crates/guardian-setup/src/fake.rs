//! A fake Windows for tests: a temporary directory stands for the drives,
//! services and firewall rules live in memory, and `guardian-agent.exe` is
//! simulated (enroll writes `agent.toml`, `service install` registers it).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use guardian_agent::config::AgentConfig;
use guardian_core::enforce::ProcessInfo;
use guardian_core::trusted::TrustedRoots;

use crate::host::{Host, HostError, RunOutput, ServiceState};
use crate::model::{self, LEGACY_SERVICE, SERVICE, WinPath};

/// Which call to make fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Method {
    /// `service`.
    Service,
    /// `start_service`.
    StartService,
    /// `stop_service`.
    StopService,
    /// `delete_service`.
    DeleteService,
    /// `run`.
    Run,
    /// `processes`.
    Processes,
    /// `set_firewall_rule`.
    SetFirewall,
    /// `delete_firewall_rule`.
    DeleteFirewall,
}

#[derive(Default)]
struct State {
    services: BTreeMap<String, ServiceState>,
    firewall: BTreeMap<String, u16>,
    calls: Vec<String>,
    fail: BTreeMap<Method, HostError>,
    enroll_error: Option<String>,
    bad_agent: Option<String>,
    elevated: bool,
    processes: Vec<ProcessInfo>,
    next_id: u64,
    last_code: Option<String>,
    restarts_on_failure: bool,
    slow_start: bool,
    slow_polls: u32,
}

/// The fake host.
pub struct FakeHost {
    root: tempfile::TempDir,
    env: BTreeMap<String, String>,
    state: Mutex<State>,
    on_wait: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl std::fmt::Debug for FakeHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeHost")
            .field("root", &self.root.path())
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// What the bundled agent binaries contain (the installed copy is compared
/// with it to tell versions apart).
pub const NEW_AGENT: &str = "guardian-agent 3.0.0 (bundled)";

impl FakeHost {
    /// 64-bit Windows, elevated, with the archive's `bin\x64` and `bin\x86`.
    #[allow(clippy::expect_used)]
    pub fn new() -> FakeHost {
        let root = tempfile::tempdir().expect("tempdir");
        let env: BTreeMap<String, String> = [
            ("PROCESSOR_ARCHITECTURE", "x86"),
            ("PROCESSOR_ARCHITEW6432", "AMD64"),
            ("ProgramFiles", r"C:\Program Files (x86)"),
            ("ProgramW6432", r"C:\Program Files"),
            ("ProgramData", r"C:\ProgramData"),
            ("SystemRoot", r"C:\Windows"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let h = FakeHost {
            root,
            env,
            state: Mutex::new(State {
                elevated: true,
                next_id: 41,
                ..State::default()
            }),
            on_wait: Mutex::new(None),
        };
        for arch in ["x64", "x86"] {
            let bin = h.setup_dir().join("bin").join(arch);
            let _ = std::fs::create_dir_all(&bin);
            let _ = std::fs::write(bin.join(model::AGENT_EXE), format!("{NEW_AGENT} {arch}"));
            let _ = std::fs::write(bin.join(model::WORKER_EXE), format!("worker {arch}"));
        }
        h
    }

    /// The layout this fake's environment gives.
    pub fn layout(&self) -> model::Layout {
        model::layout(&|k| self.env(k))
    }

    /// Not elevated.
    pub fn not_elevated(&self) {
        lock(&self.state).elevated = false;
    }

    /// Removes the bundled binaries (setup started outside the archive).
    pub fn without_bundle(&self) {
        let _ = std::fs::remove_dir_all(self.setup_dir().join("bin"));
    }

    /// Guardian 2.x: its service runs and its folder has files.
    pub fn with_legacy(&self) {
        lock(&self.state)
            .services
            .insert(LEGACY_SERVICE.into(), ServiceState::Running);
        let dir = self.io_path(&self.layout().legacy_dir);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("procsentinel-agent64.exe"), "old");
    }

    /// The next call of `m` fails (and every later one, until cleared).
    pub fn fail_on(&self, m: Method, e: HostError) {
        lock(&self.state).fail.insert(m, e);
    }

    /// Clears a failure.
    pub fn clear(&self, m: Method) {
        lock(&self.state).fail.remove(&m);
    }

    /// `enroll` fails with `message` (`None`: succeeds again).
    pub fn fail_enroll(&self, message: Option<&str>) {
        lock(&self.state).enroll_error = message.map(str::to_string);
    }

    /// The service does not start while the installed agent contains `marker`.
    pub fn agent_wont_start_if(&self, marker: &str) {
        lock(&self.state).bad_agent = Some(marker.to_string());
    }

    /// The processes `processes()` returns.
    pub fn set_processes(&self, p: Vec<ProcessInfo>) {
        lock(&self.state).processes = p;
    }

    /// The code the last `enroll` got (through `GUARDIAN_AGENT_CODE`).
    pub fn last_enroll_code(&self) -> Option<String> {
        lock(&self.state).last_code.clone()
    }

    /// Like the agent's recovery actions: a service whose start failed is
    /// started again by the service manager (and runs).
    pub fn restarts_on_failure(&self) {
        lock(&self.state).restarts_on_failure = true;
    }

    /// The agent's `service install` leaves the service starting; it runs after
    /// two more state queries, and a stop while it starts fails (Windows 1061).
    pub fn starts_slowly(&self) {
        lock(&self.state).slow_start = true;
    }

    /// Runs `f` on every `wait` (e.g. to release a "locked" file).
    pub fn on_wait(&self, f: Box<dyn Fn() + Send>) {
        *lock(&self.on_wait) = Some(f);
    }

    /// Every host call so far, as `method arg…`.
    pub fn calls(&self) -> Vec<String> {
        lock(&self.state).calls.clone()
    }

    /// A service's state as the fake knows it.
    pub fn service_state(&self, name: &str) -> Option<ServiceState> {
        lock(&self.state).services.get(name).copied()
    }

    /// The firewall rule's port.
    pub fn firewall_port(&self, name: &str) -> Option<u16> {
        lock(&self.state).firewall.get(name).copied()
    }

    fn record(&self, call: String) {
        lock(&self.state).calls.push(call);
    }

    fn failing(&self, m: Method) -> Result<(), HostError> {
        match lock(&self.state).fail.get(&m) {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    fn agent_is_bad(&self) -> bool {
        let marker = lock(&self.state).bad_agent.clone();
        let Some(marker) = marker else { return false };
        std::fs::read_to_string(self.io_path(&self.layout().agent_exe))
            .is_ok_and(|s| s.contains(&marker))
    }

    fn arg(args: &[String], flag: &str) -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    }

    fn agent(&self, args: &[String], env: &[(String, String)]) -> RunOutput {
        let ok = |out: &str| RunOutput {
            code: 0,
            stdout: out.to_string(),
            stderr: String::new(),
        };
        let fail = |err: &str| RunOutput {
            code: 1,
            stdout: String::new(),
            stderr: err.to_string(),
        };
        let config = self.io_path(&self.layout().config);
        match args.first().map(String::as_str) {
            Some("--version") => ok("guardian-agent 3.0.0\n"),
            Some("enroll") => {
                lock(&self.state).last_code = env
                    .iter()
                    .find(|(k, _)| k == "GUARDIAN_AGENT_CODE")
                    .map(|(_, v)| v.clone())
                    .or_else(|| FakeHost::arg(args, "--code"));
                if let Some(e) = lock(&self.state).enroll_error.clone() {
                    return fail(&format!("error: {e}"));
                }
                if AgentConfig::load(&config).is_ok_and(|c| c.computer_id.is_some()) {
                    return fail("error: already enrolled");
                }
                let id = {
                    let mut s = lock(&self.state);
                    s.next_id += 1;
                    s.next_id
                };
                let c = AgentConfig {
                    server: FakeHost::arg(args, "--server").unwrap_or_default(),
                    cert_sha256: FakeHost::arg(args, "--cert-sha256"),
                    computer_id: Some(id),
                    local_port: None,
                };
                match c.save(&config) {
                    Ok(()) => ok(&format!("enrolled as computer {id}\n")),
                    Err(e) => fail(&e.to_string()),
                }
            }
            Some("service") => match args.get(1).map(String::as_str) {
                Some("install") => {
                    if !AgentConfig::load(&config).is_ok_and(|c| c.computer_id.is_some()) {
                        return fail("not enrolled: run `guardian-agent enroll` first");
                    }
                    let bad = self.agent_is_bad();
                    let slow = lock(&self.state).slow_start;
                    lock(&self.state).services.insert(
                        SERVICE.into(),
                        if bad {
                            ServiceState::Stopped
                        } else if slow {
                            ServiceState::Starting
                        } else {
                            ServiceState::Running
                        },
                    );
                    if bad {
                        fail("start service: the service did not respond")
                    } else {
                        ok("Service GuardianAgent installed and started.\n")
                    }
                }
                Some("uninstall") => {
                    lock(&self.state).services.remove(SERVICE);
                    ok("Service GuardianAgent removed.\n")
                }
                _ => fail("usage"),
            },
            _ => fail("usage"),
        }
    }
}

impl Default for FakeHost {
    fn default() -> FakeHost {
        FakeHost::new()
    }
}

impl Host for FakeHost {
    fn env(&self, var: &str) -> Option<String> {
        self.env.get(var).cloned()
    }

    fn is_elevated(&self) -> bool {
        lock(&self.state).elevated
    }

    fn setup_dir(&self) -> PathBuf {
        self.root.path().join("archive").join("agent-windows")
    }

    fn io_path(&self, canonical: &WinPath) -> PathBuf {
        let mut p = self.root.path().join("drives");
        for part in canonical.as_str().split('\\').filter(|s| !s.is_empty()) {
            p.push(part.trim_end_matches(':'));
        }
        p
    }

    fn service(&self, name: &str) -> Result<Option<ServiceState>, HostError> {
        self.failing(Method::Service)?;
        let mut s = lock(&self.state);
        if s.services.get(name) == Some(&ServiceState::Starting) {
            s.slow_polls += 1;
            if s.slow_polls >= 2 {
                s.services.insert(name.into(), ServiceState::Running);
            }
        }
        Ok(s.services.get(name).copied())
    }

    fn start_service(&self, name: &str, _timeout: Duration) -> Result<(), HostError> {
        self.record(format!("start_service {name}"));
        self.failing(Method::StartService)?;
        if name == SERVICE && self.agent_is_bad() {
            self.record(format!("start_service {name} (failed)"));
            let mut s = lock(&self.state);
            if s.restarts_on_failure {
                s.services.insert(name.into(), ServiceState::Running);
            }
            return Err(HostError::Failed("the service did not start".into()));
        }
        let mut s = lock(&self.state);
        match s.services.get_mut(name) {
            Some(st) => {
                *st = ServiceState::Running;
                Ok(())
            }
            None => Err(HostError::Failed(format!("{name} is not installed"))),
        }
    }

    fn stop_service(&self, name: &str, _timeout: Duration) -> Result<(), HostError> {
        self.record(format!("stop_service {name}"));
        self.failing(Method::StopService)?;
        let mut s = lock(&self.state);
        match s.services.get_mut(name) {
            Some(ServiceState::Starting) => Err(HostError::Failed(
                "the service cannot accept control messages at this time (1061)".into(),
            )),
            Some(st) => {
                *st = ServiceState::Stopped;
                Ok(())
            }
            None => Err(HostError::Failed(format!("{name} is not installed"))),
        }
    }

    fn delete_service(&self, name: &str) -> Result<(), HostError> {
        self.record(format!("delete_service {name}"));
        self.failing(Method::DeleteService)?;
        lock(&self.state).services.remove(name);
        Ok(())
    }

    fn run(
        &self,
        exe: &Path,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<RunOutput, HostError> {
        let name = exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let shown: Vec<String> = args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if i > 0 && args.get(i - 1).is_some_and(|p| p == "--code") {
                    "***".to_string()
                } else {
                    a.clone()
                }
            })
            .collect();
        let env_keys: Vec<String> = env.iter().map(|(k, _)| format!(" env {k}")).collect();
        self.record(format!(
            "run {name} {}{}",
            shown.join(" "),
            env_keys.concat()
        ));
        self.failing(Method::Run)?;
        if !exe.is_file() {
            return Err(HostError::Failed(format!("{} not found", exe.display())));
        }
        if name == model::AGENT_EXE {
            Ok(self.agent(args, env))
        } else {
            Err(HostError::Failed(format!("unknown program {name}")))
        }
    }

    fn processes(&self) -> Result<Vec<ProcessInfo>, HostError> {
        self.failing(Method::Processes)?;
        Ok(lock(&self.state).processes.clone())
    }

    fn trusted_roots(&self) -> TrustedRoots {
        TrustedRoots {
            recursive: vec![PathBuf::from(r"C:\Windows")],
            ..TrustedRoots::default()
        }
    }

    fn firewall_rule(&self, name: &str) -> Result<bool, HostError> {
        Ok(lock(&self.state).firewall.contains_key(name))
    }

    fn set_firewall_rule(&self, name: &str, port: u16) -> Result<(), HostError> {
        self.record(format!("set_firewall_rule {name} {port}"));
        self.failing(Method::SetFirewall)?;
        lock(&self.state).firewall.insert(name.into(), port);
        Ok(())
    }

    fn delete_firewall_rule(&self, name: &str) -> Result<(), HostError> {
        self.record(format!("delete_firewall_rule {name}"));
        self.failing(Method::DeleteFirewall)?;
        lock(&self.state).firewall.remove(name);
        Ok(())
    }

    fn wait(&self, _d: Duration) {
        if let Some(f) = lock(&self.on_wait).as_ref() {
            f();
        }
    }
}
