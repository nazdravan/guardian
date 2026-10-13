//! Everything that touches the machine. `apply` is portable; collection is Windows-first
//! with a folder-only fallback elsewhere (for `report --dir` and development).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod apply;
pub mod collect;
pub mod files;
pub mod health;
pub mod text;
#[cfg(windows)]
pub mod win;

pub use collect::{CollectOptions, collect, elevated, io_path, relaunch_elevated, restart_service};
