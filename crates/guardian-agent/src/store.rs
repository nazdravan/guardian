//! Files the agent keeps: written atomically, private where they hold secrets.

use std::collections::{BTreeSet, HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};

use guardian_core::keys::KeyPair;
use guardian_core::policy::AppName;
use guardian_proto::v1::{AgentState, Mode, Profile, StoredState};
use prost::Message;

/// How many command ids are remembered.
pub const SEEN_CAP: usize = 1000;
/// Name of the default profile used before any state is known.
pub const DEFAULT_PROFILE: &str = "Lecție";

/// A store operation failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// I/O on a path.
    #[error("{path}: {err}")]
    Io {
        /// The path.
        path: PathBuf,
        /// The error.
        err: std::io::Error,
    },
    /// Bad content.
    #[error("{0}")]
    Format(String),
}

impl StoreError {
    /// Wraps an I/O error with its path.
    pub fn io(path: &Path, err: std::io::Error) -> StoreError {
        StoreError::Io {
            path: path.to_path_buf(),
            err,
        }
    }
}

/// Writes `data` to a temporary file next to `path`, syncs it and renames it over
/// `path`. With `private`, the file is created `0600` on Unix.
pub fn atomic_write(path: &Path, data: &[u8], private: bool) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
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
            o.mode(if private { 0o600 } else { 0o644 });
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut f = o.open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Loads the 32-byte seed, or creates it if missing. A file of any other size is
/// an error and is left untouched: a new key would be a new identity.
pub fn load_or_create_key(path: &Path) -> Result<KeyPair, StoreError> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let seed: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                StoreError::Format(format!(
                    "{}: expected a 32-byte key, found {} bytes",
                    path.display(),
                    bytes.len()
                ))
            })?;
            Ok(KeyPair::from_seed(&seed))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| StoreError::io(dir, e))?;
            }
            let k = KeyPair::generate();
            atomic_write(path, k.seed().as_slice(), true).map_err(|e| StoreError::io(path, e))?;
            Ok(k)
        }
        Err(e) => Err(StoreError::io(path, e)),
    }
}

/// The saved state; `None` if missing or unreadable (a corrupt file is logged).
pub fn load_state(path: &Path) -> Option<StoredState> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!("cannot read {}: {e}", path.display());
            return None;
        }
    };
    match StoredState::decode(bytes.as_slice()) {
        Ok(s) if s.state.is_some() => Some(s),
        Ok(_) => {
            tracing::warn!("{} holds no state; ignoring it", path.display());
            None
        }
        Err(e) => {
            tracing::warn!(
                "{} is corrupt ({e}); starting from the default",
                path.display()
            );
            None
        }
    }
}

/// Saves the state atomically.
pub fn save_state(path: &Path, s: &StoredState) -> Result<(), StoreError> {
    atomic_write(path, &s.encode_to_vec(), true).map_err(|e| StoreError::io(path, e))
}

/// What applies before any state is known: "Lecție", blacklist, no apps (kills
/// nothing, but is not "free").
pub fn default_state() -> StoredState {
    StoredState {
        state: Some(AgentState {
            version: 0,
            effective_profile_id: 0,
            room_id: 0,
            profiles: vec![Profile {
                id: 0,
                name: DEFAULT_PROFILE.into(),
                mode: Mode::Blacklist as i32,
                apps: vec![],
            }],
        }),
        ..Default::default()
    }
}

/// The last [`SEEN_CAP`] command ids, oldest first.
#[derive(Debug, Default, Clone)]
pub struct SeenIds {
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl SeenIds {
    /// Reads `seen.txt` (one id per line); missing → empty.
    pub fn load(path: &Path) -> SeenIds {
        let mut s = SeenIds::default();
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                s.insert(line);
            }
        }
        s
    }

    /// Records `id`; `false` if it was already there.
    pub fn insert(&mut self, id: &str) -> bool {
        if self.set.contains(id) {
            return false;
        }
        self.order.push_back(id.to_string());
        self.set.insert(id.to_string());
        while self.order.len() > SEEN_CAP {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }

    /// How many ids are remembered.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// No ids remembered.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Writes `seen.txt`.
    pub fn save(&self, path: &Path) -> Result<(), StoreError> {
        let mut text = String::new();
        for id in &self.order {
            text.push_str(id);
            text.push('\n');
        }
        atomic_write(path, text.as_bytes(), false).map_err(|e| StoreError::io(path, e))
    }
}

/// The local whitelist: one app name per line, `#` comments; bad names skipped.
pub fn load_whitelist(path: &Path) -> BTreeSet<AppName> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let mut out = BTreeSet::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match AppName::parse(line) {
            Ok(a) => {
                out.insert(a);
            }
            Err(e) => tracing::warn!("{}: skipping {line:?}: {e}", path.display()),
        }
    }
    out
}
