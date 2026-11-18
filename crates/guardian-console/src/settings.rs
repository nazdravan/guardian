//! `console.toml`: the console's mode (remote server or embedded), the last
//! login and the session token; plus where the console keeps its files.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The default server port.
pub const DEFAULT_PORT: u16 = 8443;

/// How the console reaches a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// A server elsewhere.
    Remote {
        /// `https://host:port`.
        server: String,
        /// Pinned certificate SHA-256 (hex).
        cert_sha256: Option<String>,
    },
    /// The server runs inside the console while it is open.
    Embedded {
        /// Listening port.
        port: u16,
    },
}

/// A value that must not reach logs or the screen.
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

/// The console's settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConsoleSettings {
    /// `None` until the first-run choice.
    pub mode: Option<Mode>,
    /// Last login name.
    pub login: Option<String>,
    token: Option<Secret>,
}

/// Reading or writing `console.toml` failed.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// Filesystem.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The error.
        source: std::io::Error,
    },
    /// Not valid TOML or an unknown mode.
    #[error("{path}: {message}")]
    Format {
        /// The file.
        path: PathBuf,
        /// What is wrong.
        message: String,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cert_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

impl ConsoleSettings {
    /// The session token.
    pub fn token(&self) -> Option<&str> {
        self.token.as_ref().map(Secret::expose)
    }

    /// Replaces the session token.
    pub fn set_token(&mut self, t: Option<String>) {
        self.token = t.map(Secret);
    }

    /// Reads `path`; a missing file is the default.
    pub fn load(path: &Path) -> Result<ConsoleSettings, SettingsError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ConsoleSettings::default());
            }
            Err(source) => {
                return Err(SettingsError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        let format = |message: String| SettingsError::Format {
            path: path.to_path_buf(),
            message,
        };
        let st: Stored = toml::from_str(&text).map_err(|e| format(e.to_string()))?;
        let mode = match st.mode.as_deref() {
            None => None,
            Some("remote") => {
                let server = st
                    .server
                    .ok_or_else(|| format("mode = \"remote\" needs `server`".into()))?;
                Some(Mode::Remote {
                    server,
                    cert_sha256: st.cert_sha256,
                })
            }
            Some("embedded") => Some(Mode::Embedded {
                port: st.port.unwrap_or(DEFAULT_PORT),
            }),
            Some(other) => return Err(format(format!("unknown mode {other:?}"))),
        };
        Ok(ConsoleSettings {
            mode,
            login: st.login,
            token: st.token.map(Secret),
        })
    }

    /// Writes `path` atomically, readable only by the user.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let mut st = Stored {
            login: self.login.clone(),
            token: self.token.as_ref().map(|t| t.0.clone()),
            ..Default::default()
        };
        match &self.mode {
            None => {}
            Some(Mode::Remote {
                server,
                cert_sha256,
            }) => {
                st.mode = Some("remote".into());
                st.server = Some(server.clone());
                st.cert_sha256 = cert_sha256.clone();
            }
            Some(Mode::Embedded { port }) => {
                st.mode = Some("embedded".into());
                st.port = Some(*port);
            }
        }
        let text = toml::to_string(&st).map_err(|e| SettingsError::Format {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        write_private(path, text.as_bytes()).map_err(|source| SettingsError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Temp file in the same directory, `sync_all`, rename; 0600 on Unix.
fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tmp{}", std::process::id()));
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

fn env_dir(k: &str) -> Option<PathBuf> {
    std::env::var_os(k)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// `%APPDATA%\Guardian\console.toml`, or `<config dir>/guardian/console.toml`.
pub fn settings_path() -> PathBuf {
    let base = guardian_client::user_config_dir().unwrap_or_else(|| PathBuf::from("."));
    if cfg!(windows) {
        base.join("Guardian").join("console.toml")
    } else {
        base.join("guardian").join("console.toml")
    }
}

/// `<ProgramData>\Guardian\server`. Not `Guardian Server`: that folder belongs
/// to the 2.x Go server, whose `guardian.db` has a different schema.
pub fn embedded_data_dir_from(program_data: &Path) -> PathBuf {
    program_data.join("Guardian").join("server")
}

/// `%ProgramData%\Guardian\server`, or a per-user data directory elsewhere
/// (development only).
pub fn embedded_data_dir() -> PathBuf {
    if cfg!(windows) {
        embedded_data_dir_from(
            &env_dir("ProgramData").unwrap_or_else(|| PathBuf::from(r"C:\ProgramData")),
        )
    } else if cfg!(target_os = "macos") {
        env_dir("HOME")
            .map(|h| h.join("Library").join("Application Support"))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("guardian-server")
    } else {
        env_dir("XDG_DATA_HOME")
            .or_else(|| env_dir("HOME").map(|h| h.join(".local").join("share")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("guardian-server")
    }
}

/// `%LOCALAPPDATA%\Guardian\console.log`, or the user's cache directory.
pub fn log_path() -> PathBuf {
    if cfg!(windows) {
        env_dir("LOCALAPPDATA")
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Guardian")
            .join("console.log")
    } else if cfg!(target_os = "macos") {
        env_dir("HOME")
            .map(|h| h.join("Library").join("Caches"))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("guardian")
            .join("console.log")
    } else {
        env_dir("XDG_CACHE_HOME")
            .or_else(|| env_dir("HOME").map(|h| h.join(".cache")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("guardian")
            .join("console.log")
    }
}

/// Checks a server address typed by the user: `https://host[:port]`, port 8443
/// when missing.
pub fn validate_server_url(s: &str) -> Result<String, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("Enter the server address, e.g. https://server.school:8443".into());
    }
    if s.chars().any(char::is_whitespace) {
        return Err("The address cannot contain spaces.".into());
    }
    let Some(rest) = s.strip_prefix("https://") else {
        if s.starts_with("http://") {
            return Err("Use https:// — the console only talks to servers over TLS.".into());
        }
        return Err("The address must start with https://".into());
    };
    let host = rest.strip_suffix('/').unwrap_or(rest);
    if host.is_empty() || host.contains('/') {
        return Err("Enter only https://host:port, without a path.".into());
    }
    let has_port = match host.rfind(']') {
        Some(i) => host[i..].contains(':'),
        None => host.contains(':'),
    };
    if has_port {
        let port = host.rsplit(':').next().unwrap_or_default();
        if port.parse::<u16>().ok().filter(|p| *p != 0).is_none() {
            return Err(format!("{port:?} is not a valid port."));
        }
        Ok(format!("https://{host}"))
    } else {
        Ok(format!("https://{host}:{DEFAULT_PORT}"))
    }
}

/// Normalizes a certificate SHA-256 typed or pasted by the user: `:` and spaces
/// are ignored, case does not matter; empty means "no pin".
pub fn normalize_pin(s: &str) -> Result<Option<String>, String> {
    let hex: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':')
        .collect::<String>()
        .to_lowercase();
    if hex.is_empty() {
        return Ok(None);
    }
    if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(Some(hex))
    } else {
        Err("The certificate SHA-256 has 64 hexadecimal digits.".into())
    }
}
