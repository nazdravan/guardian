//! The agent's configuration file and where its files live.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::store::{StoreError, atomic_write};

/// `agent.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    /// `wss://host:port` (or `ws://` for tests).
    pub server: String,
    /// Pinned SHA-256 of the server certificate.
    pub cert_sha256: Option<String>,
    /// Set by `enroll`.
    pub computer_id: Option<u64>,
    /// TCP and UDP port of the local fallback listener; missing =
    /// [`guardian_core::local::DEFAULT_LOCAL_PORT`], `0` = off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_port: Option<u16>,
}

impl AgentConfig {
    /// The local fallback port (`0` = off).
    pub fn local_port(&self) -> u16 {
        self.local_port
            .unwrap_or(guardian_core::local::DEFAULT_LOCAL_PORT)
    }

    /// Reads the file.
    pub fn load(path: &Path) -> Result<AgentConfig, StoreError> {
        let text = std::fs::read_to_string(path).map_err(|e| StoreError::io(path, e))?;
        toml::from_str(&text).map_err(|e| StoreError::Format(format!("{}: {e}", path.display())))
    }

    /// Writes the file atomically, readable only by the owner.
    pub fn save(&self, path: &Path) -> Result<(), StoreError> {
        let text = toml::to_string(self).map_err(|e| StoreError::Format(e.to_string()))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::io(dir, e))?;
        }
        atomic_write(path, text.as_bytes(), true).map_err(|e| StoreError::io(path, e))
    }
}

/// Config file and data directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPaths {
    /// `agent.toml`.
    pub config: PathBuf,
    /// Directory with key, state, seen ids, local whitelist, log.
    pub data: PathBuf,
}

impl AgentPaths {
    /// Linux: `/etc/guardian-agent/agent.toml` + `/var/lib/guardian-agent`;
    /// Windows: both under `%ProgramData%\Guardian Agent`.
    pub fn system_default() -> AgentPaths {
        if cfg!(windows) {
            let base = std::env::var_os("ProgramData")
                .filter(|v| !v.is_empty())
                .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from)
                .join("Guardian Agent");
            AgentPaths {
                config: base.join("agent.toml"),
                data: base,
            }
        } else {
            AgentPaths {
                config: PathBuf::from("/etc/guardian-agent/agent.toml"),
                data: PathBuf::from("/var/lib/guardian-agent"),
            }
        }
    }

    /// `dir/agent.toml` and `dir/data/` (tests, development).
    pub fn under(dir: &Path) -> AgentPaths {
        AgentPaths {
            config: dir.join("agent.toml"),
            data: dir.join("data"),
        }
    }

    /// The ed25519 seed.
    pub fn key(&self) -> PathBuf {
        self.data.join("agent.key")
    }
    /// The last applied state.
    pub fn state(&self) -> PathBuf {
        self.data.join("state.pb")
    }
    /// Recently seen command ids.
    pub fn seen(&self) -> PathBuf {
        self.data.join("seen.txt")
    }
    /// The local fallback switch and journal.
    pub fn local(&self) -> PathBuf {
        self.data.join("local.pb")
    }
    /// The local whitelist.
    pub fn whitelist(&self) -> PathBuf {
        self.data.join("whitelist.txt")
    }
    /// The service log (Windows).
    pub fn log(&self) -> PathBuf {
        self.data.join("agent.log")
    }
}
