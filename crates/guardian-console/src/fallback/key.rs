//! The console's ed25519 key (`console.key`, the 32-byte seed, 0600).

use std::path::Path;

use guardian_core::keys::KeyPair;

use super::FallbackError;
use crate::settings::write_private;

/// Reads the key, or creates and saves a new one.
pub fn load_or_create(path: &Path) -> Result<KeyPair, FallbackError> {
    let io = |source| FallbackError::Io {
        path: path.to_path_buf(),
        source,
    };
    match std::fs::read(path) {
        Ok(bytes) => {
            let seed: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                FallbackError::Format(format!(
                    "{}: expected a 32-byte key, found {} bytes",
                    path.display(),
                    bytes.len()
                ))
            })?;
            Ok(KeyPair::from_seed(&seed))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let k = KeyPair::generate();
            write_private(path, k.seed().as_slice()).map_err(io)?;
            Ok(k)
        }
        Err(e) => Err(io(e)),
    }
}

/// Writes `k` to `path` (0600), replacing any older key.
pub fn save(path: &Path, k: &KeyPair) -> Result<(), FallbackError> {
    write_private(path, k.seed().as_slice()).map_err(|source| FallbackError::Io {
        path: path.to_path_buf(),
        source,
    })
}
