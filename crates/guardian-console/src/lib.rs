//! Guardian 3.0 teacher console: talks to a remote server or runs the embedded
//! one, shows rooms and computers live, switches room profiles and administers
//! the school.

#![forbid(unsafe_code)]

/// The console's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod settings;
