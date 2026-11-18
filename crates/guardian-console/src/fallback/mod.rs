//! Local fallback mode (§11 of the 3.0 design): the console's key, its offline
//! cache, and (in `client`) the channel to the agents on the school network.

pub mod cache;
pub mod key;

use std::path::PathBuf;

/// Something in fallback mode failed.
#[derive(Debug, thiserror::Error)]
pub enum FallbackError {
    /// A file.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The error.
        source: std::io::Error,
    },
    /// A file holds something else.
    #[error("{0}")]
    Format(String),
    /// Hashing a password failed.
    #[error("cannot remember the password")]
    Hash,
}
