//! Facts mirrored from the agent's Go source (`agent/*.go`). `tests/agent_consistency.rs`
//! re-reads the source and fails if any of these drift.

pub const SERVICE_NAME: &str = "ProcSentinelAgent";
pub const DEFAULT_INSTALL_DIR: &str = r"C:\Windows\System32\ProcSentinel\agent";

pub const ENV_FILE: &str = ".env";
pub const CREDENTIALS_FILE: &str = "agent_credentials.json";
pub const SYNC_FILE: &str = "sync.json";
pub const WHITELIST_FILE: &str = "whitelist.txt";

pub const KEY_SERVER_ADDRESS: &str = "SERVER_ADDRESS";
pub const KEY_BINDING_TOKEN: &str = "BINDING_TOKEN";
pub const KEY_CHECK_INTERVAL: &str = "CHECK_INTERVAL";
pub const KNOWN_KEYS: [&str; 3] = [KEY_SERVER_ADDRESS, KEY_BINDING_TOKEN, KEY_CHECK_INTERVAL];

/// What the agent uses in service mode when CHECK_INTERVAL is absent or invalid.
pub const DEFAULT_SERVICE_INTERVAL_SECS: u32 = 20;

/// Event Log message prefixes (`Infof("Killed process: %s")` etc.).
pub const KILL_BLACKLIST_PREFIX: &str = "Killed process: ";
pub const KILL_WHITELIST_PREFIX: &str = "Killed non-whitelisted process: ";
/// Logged once per `runAgent`, i.e. once per service start; used as the start time.
pub const START_MARKER: &str = "Server address: ";
/// Logged by `tryEnroll` right after it rewrote `.env` to drop BINDING_TOKEN.
pub const ENROLLED_MARKER: &str = "Enrolled with ";
/// How long after the "Enrolled with" record a `.env` change still counts as the agent's own.
pub const ENROLL_ENV_REWRITE_SLACK_SECS: i64 = 10;

/// `isSystemProcess`'s hardcoded list, lower case, in the agent's order.
#[rustfmt::skip]
pub const PROTECTED: &[&str] = &[
    "system", "system idle process", "secure system", "registry",
    "smss.exe", "csrss.exe", "wininit.exe", "winlogon.exe",
    "services.exe", "lsass.exe", "lsaiso.exe",
    "svchost.exe", "dwm.exe", "fontdrvhost.exe", "conhost.exe",
    "wudfhost.exe", "wmiprvse.exe", "dllhost.exe", "spoolsv.exe",
    "audiodg.exe", "dashost.exe", "searchindexer.exe",
    "searchprotocolhost.exe", "searchfilterhost.exe",
    "explorer.exe", "sihost.exe", "taskhostw.exe",
    "runtimebroker.exe", "applicationframehost.exe",
    "shellexperiencehost.exe", "startmenuexperiencehost.exe",
    "searchhost.exe", "searchui.exe", "searchapp.exe",
    "textinputhost.exe", "lockapp.exe", "widgets.exe",
    "widgetservice.exe", "comppkgsrv.exe",
    "userinit.exe", "logonui.exe",
    "securityhealthservice.exe", "securityhealthsystray.exe",
    "securityhealthhost.exe", "smartscreen.exe",
    "msmpeng.exe", "nissrv.exe", "mpcmdrun.exe",
    "ctfmon.exe", "tabletinputservice.exe",
    "lsm.exe", "networkservice.exe", "localservice.exe",
    "tasklist.exe", "taskmgr.exe", "cmd.exe", "powershell.exe", "pwsh.exe",
    "windowsterminal.exe", "wt.exe", "openssh.exe", "sshd.exe",
    "procsentinel-agent64.exe", "procsentinel-agent32.exe",
    "guardian.exe",
];

pub fn is_protected(name: &str) -> bool {
    let lower = name.to_lowercase();
    PROTECTED.iter().any(|p| *p == lower)
}
