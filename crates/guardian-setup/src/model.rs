//! Pure logic: where things go on Windows, which agent fits this computer,
//! the local whitelist, and the secret type. No OS calls.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use guardian_core::enforce::{Os, ProcessInfo};
use guardian_core::policy::{AppName, AppNameError};
use guardian_core::protected::is_protected;
use guardian_core::trusted::TrustedRoots;
use serde::Serialize;

/// The agent build this computer needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    /// 64-bit (also used, emulated, on ARM64).
    X64,
    /// 32-bit.
    X86,
}

impl Arch {
    /// The folder name under `bin\`.
    pub fn dir(self) -> &'static str {
        match self {
            Arch::X64 => "x64",
            Arch::X86 => "x86",
        }
    }
}

/// The architecture Windows reports, not the one this process was built for:
/// `PROCESSOR_ARCHITEW6432` (set for a 32-bit process on 64-bit Windows), then
/// `PROCESSOR_ARCHITECTURE`. ARM64 gets the 64-bit agent (emulated).
pub fn os_arch(env: &dyn Fn(&str) -> Option<String>) -> Arch {
    let v = env("PROCESSOR_ARCHITEW6432")
        .filter(|s| !s.is_empty())
        .or_else(|| env("PROCESSOR_ARCHITECTURE"))
        .unwrap_or_default()
        .to_ascii_uppercase();
    match v.as_str() {
        "AMD64" | "IA64" | "ARM64" => Arch::X64,
        _ => Arch::X86,
    }
}

/// A Windows path kept as text, so that the model behaves the same on every
/// OS; `Host::io_path` turns it into a real path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct WinPath(String);

impl WinPath {
    /// From text (`/` becomes `\`, trailing `\` dropped).
    pub fn new(s: &str) -> WinPath {
        let s = s.replace('/', "\\");
        let trimmed = s.trim_end_matches('\\');
        WinPath(if trimmed.ends_with(':') {
            format!("{trimmed}\\")
        } else {
            trimmed.to_string()
        })
    }

    /// `self\part`.
    pub fn join(&self, part: &str) -> WinPath {
        if self.0.ends_with('\\') {
            WinPath(format!("{}{part}", self.0))
        } else {
            WinPath(format!("{}\\{part}", self.0))
        }
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WinPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where Guardian lives on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Layout {
    /// The agent's binaries (`Program Files\Guardian`).
    pub program_dir: WinPath,
    /// `guardian-agent.exe` there.
    pub agent_exe: WinPath,
    /// `guardian-worker.exe` there.
    pub worker_exe: WinPath,
    /// `agent.toml` (as `AgentPaths::system_default`).
    pub config: WinPath,
    /// The agent's data directory (key, state, whitelist, log).
    pub data_dir: WinPath,
    /// Where Guardian 2.x kept its agent.
    pub legacy_dir: WinPath,
    /// The setup's lock file.
    pub lock: WinPath,
}

/// Name of the agent service (from `guardian-agent`).
pub const SERVICE: &str = "GuardianAgent";
/// Name of the Guardian 2.x agent service.
pub const LEGACY_SERVICE: &str = "ProcSentinelAgent";
/// The firewall rule for local fallback mode.
pub const FIREWALL_RULE: &str = "Guardian local fallback";
/// The agent binary.
pub const AGENT_EXE: &str = "guardian-agent.exe";
/// The worker binary.
pub const WORKER_EXE: &str = "guardian-worker.exe";

fn var(env: &dyn Fn(&str) -> Option<String>, name: &str, default: &str) -> WinPath {
    WinPath::new(
        &env(name)
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| default.to_string()),
    )
}

/// The layout for this environment. 64-bit binaries go to `ProgramW6432`
/// (`C:\Program Files`), even from this 32-bit process whose `ProgramFiles` is
/// `Program Files (x86)`; 32-bit Windows has no `ProgramW6432`.
pub fn layout(env: &dyn Fn(&str) -> Option<String>) -> Layout {
    let program_files = match env("ProgramW6432").filter(|v| !v.trim().is_empty()) {
        Some(p) => WinPath::new(&p),
        None => var(env, "ProgramFiles", r"C:\Program Files"),
    };
    let program_data = var(env, "ProgramData", r"C:\ProgramData");
    let system_root = var(env, "SystemRoot", r"C:\Windows");
    let program_dir = program_files.join("Guardian");
    let data_dir = program_data.join("Guardian Agent");
    Layout {
        agent_exe: program_dir.join(AGENT_EXE),
        worker_exe: program_dir.join(WORKER_EXE),
        program_dir,
        config: data_dir.join("agent.toml"),
        data_dir,
        legacy_dir: system_root
            .join("System32")
            .join("ProcSentinel")
            .join("agent"),
        lock: program_data.join("Guardian").join("setup.lock"),
    }
}

/// The agent and worker shipped next to `guardian-setup.exe` for `arch`.
pub fn bundled_bins(setup_dir: &Path, arch: Arch) -> (PathBuf, PathBuf) {
    let bin = setup_dir.join("bin").join(arch.dir());
    (bin.join(AGENT_EXE), bin.join(WORKER_EXE))
}

/// `canonical` as this process must open it: a 32-bit process on 64-bit
/// Windows (`wow64`) sees `System32` redirected to `SysWOW64`, so paths under
/// `<SystemRoot>\System32` go through `Sysnative` instead.
pub fn sysnative(canonical: &WinPath, system_root: &WinPath, wow64: bool) -> WinPath {
    if !wow64 {
        return canonical.clone();
    }
    let prefix = system_root.join("System32");
    let c = canonical.as_str();
    let p = prefix.as_str();
    let matches = c.len() >= p.len()
        && c.get(..p.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(p))
        && (c.len() == p.len() || c.as_bytes().get(p.len()) == Some(&b'\\'));
    if !matches {
        return canonical.clone();
    }
    let rest = c.get(p.len()..).unwrap_or_default();
    WinPath::new(&format!("{}{rest}", system_root.join("Sysnative")))
}

/// `netsh` arguments that add the local fallback rule for one protocol.
pub fn firewall_args(name: &str, port: u16, proto: &str) -> Vec<String> {
    vec![
        "advfirewall".into(),
        "firewall".into(),
        "add".into(),
        "rule".into(),
        format!("name={name}"),
        "dir=in".into(),
        "action=allow".into(),
        format!("protocol={proto}"),
        format!("localport={port}"),
        "remoteip=localsubnet".into(),
        "profile=any".into(),
    ]
}

/// The agent's local `whitelist.txt`: names (one per line) and the comment
/// lines at the top, which are kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Whitelist {
    names: BTreeSet<AppName>,
    header: Vec<String>,
}

impl Whitelist {
    /// Reads the text the agent reads (BOM, CRLF, `#` comments); bad lines
    /// become warnings.
    pub fn parse(text: &str) -> (Whitelist, Vec<String>) {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut w = Whitelist::default();
        let mut warnings = Vec::new();
        let mut in_header = true;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('#') {
                if in_header {
                    w.header.push(line.to_string());
                }
                continue;
            }
            in_header = false;
            match AppName::parse(line) {
                Ok(a) => {
                    w.names.insert(a);
                }
                Err(e) => warnings.push(format!("skipping {line:?}: {e}")),
            }
        }
        (w, warnings)
    }

    /// Adds a program name; `Ok(false)` if it was there.
    pub fn add(&mut self, name: &str) -> Result<bool, AppNameError> {
        Ok(self.names.insert(AppName::parse(name)?))
    }

    /// Removes a program name; `false` if it was not there.
    pub fn remove(&mut self, name: &str) -> bool {
        match AppName::parse(name) {
            Ok(a) => self.names.remove(&a),
            Err(_) => false,
        }
    }

    /// The names, sorted.
    pub fn names(&self) -> Vec<String> {
        self.names.iter().map(|a| a.as_str().to_string()).collect()
    }

    /// The file's text: the header comments, then one name per line.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for h in &self.header {
            out.push_str(h);
            out.push('\n');
        }
        for n in &self.names {
            out.push_str(n.as_str());
            out.push('\n');
        }
        out
    }
}

/// The comment written at the top of a generated whitelist.
pub const GENERATED_HEADER: &str =
    "# Programs allowed in exam (whitelist) mode on this computer, written by guardian-setup.";

/// The whitelist written at install time from what runs then (§8.3): every
/// process except those started from a user profile (`C:\Users\…`) and those
/// the agent protects anyway.
pub fn generated_whitelist(procs: &[ProcessInfo], roots: &TrustedRoots) -> Whitelist {
    let mut w = Whitelist {
        header: vec![GENERATED_HEADER.to_string()],
        ..Whitelist::default()
    };
    for p in procs {
        let from_profile = p.exe_path.as_ref().is_some_and(|e| {
            let s = e.to_string_lossy().replace('/', "\\").to_lowercase();
            s.get(1..)
                .is_some_and(|rest| rest.starts_with(":\\users\\"))
        });
        if from_profile {
            continue;
        }
        let protected = is_protected(Os::Windows, &p.name)
            && p.exe_path
                .as_ref()
                .is_some_and(|e| roots.contains(Os::Windows, e));
        if protected {
            continue;
        }
        let _ = w.add(&p.name);
    }
    w
}

/// A value that must not reach logs, JSON or the screen (the enrollment code).
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret(String);

impl Secret {
    /// Wraps `s`.
    pub fn new(s: impl Into<String>) -> Secret {
        Secret(s.into())
    }

    /// The value.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

impl Serialize for Secret {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("***")
    }
}
