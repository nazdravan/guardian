//! The server inside the console (§9.6): same code as `guardian-server`, started
//! when the console opens and stopped when it closes.

use std::path::Path;
use std::time::Instant;

use guardian_server::{ServerConfig, ServerHandle, StartError, TlsMode};

/// What the embedded server is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddedState {
    /// Not started (remote mode, or stopped).
    Stopped,
    /// Starting.
    Starting,
    /// Listening.
    Running {
        /// Port.
        port: u16,
        /// Its self-signed certificate's SHA-256, for the agents.
        cert_sha256: String,
        /// When it started (for the "no agent yet" hint).
        started_at: Instant,
    },
    /// It could not start; never shown as running.
    Failed {
        /// Why, for the teacher.
        reason: String,
    },
}

/// Starts the server on `0.0.0.0:<port>` with a self-signed certificate kept in
/// `data_dir`.
pub async fn start_embedded(data_dir: &Path, port: u16) -> Result<ServerHandle, String> {
    let mut cfg = ServerConfig::new(data_dir.to_path_buf(), TlsMode::SelfSigned);
    cfg.listen = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    guardian_server::start(cfg).await.map_err(|e| match &e {
        StartError::Io(io) if io.kind() == std::io::ErrorKind::AddrInUse => format!(
            "Port {port} is already in use — another Guardian server or program is using it. \
             Close it, or use Change mode… to pick another port."
        ),
        StartError::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
            format!("Cannot use {}: access denied.", data_dir.display())
        }
        other => format!("The server could not start: {other}"),
    })
}

/// The SID in `whoami /user /fo csv /nh` output (`"pc\user","S-1-5-…"`).
pub fn parse_whoami_sid(csv: &str) -> Option<String> {
    csv.split(',')
        .map(|f| f.trim().trim_matches('"'))
        .find(|f| f.starts_with("S-1-"))
        .map(str::to_string)
}

/// Creates the data directory, readable only by SYSTEM, Administrators and the
/// current user (it holds the TLS key and the database).
pub fn prepare_data_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("Cannot protect {}: {e}", dir.display()))?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new(crate::firewall::system_tool(
            &crate::firewall::system_root(),
            "whoami.exe",
        ))
        .args(["/user", "/fo", "csv", "/nh"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("whoami: {e}"))?;
        let sid = parse_whoami_sid(&String::from_utf8_lossy(&out.stdout))
            .ok_or_else(|| "Cannot find the current user's SID.".to_string())?;
        let me = format!("*{sid}:(OI)(CI)F");
        let out = std::process::Command::new(crate::firewall::system_tool(
            &crate::firewall::system_root(),
            "icacls.exe",
        ))
        .arg(dir)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "/grant:r",
            "*S-1-5-32-544:(OI)(CI)F",
            "/grant:r",
            &me,
            "/T",
            "/C",
            "/Q",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("icacls: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "Cannot protect {}: {}",
                dir.display(),
                String::from_utf8_lossy(&out.stdout).trim()
            ));
        }
    }
    Ok(())
}
