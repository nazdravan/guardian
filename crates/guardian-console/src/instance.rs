//! One console at a time: a lock file next to the settings.

use std::fs::{File, TryLockError};
use std::path::Path;

/// Held while the console runs.
#[derive(Debug)]
pub struct InstanceLock(#[allow(dead_code)] File);

/// Why the lock was not taken.
#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    /// Another console holds it.
    #[error("Guardian Console is already running.")]
    AlreadyRunning,
    /// The lock file could not be opened.
    #[error("cannot open the lock file: {0}")]
    Io(#[from] std::io::Error),
}

impl InstanceLock {
    /// Takes `dir/console.lock`.
    pub fn acquire(dir: &Path) -> Result<InstanceLock, InstanceError> {
        std::fs::create_dir_all(dir)?;
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("console.lock"))?;
        match f.try_lock() {
            Ok(()) => Ok(InstanceLock(f)),
            Err(TryLockError::WouldBlock) => Err(InstanceError::AlreadyRunning),
            Err(TryLockError::Error(e)) => Err(InstanceError::Io(e)),
        }
    }
}
