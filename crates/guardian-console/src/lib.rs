//! Guardian 3.0 teacher console: talks to a remote server or runs the embedded
//! one, shows rooms and computers live, switches room profiles and administers
//! the school.

#![forbid(unsafe_code)]

/// The console's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod backend;
pub mod connect_info;
pub mod embedded;
pub mod firewall;
pub mod fleet;
pub mod instance;
pub mod settings;
pub mod ui;

#[cfg(feature = "testing")]
pub mod testing;
