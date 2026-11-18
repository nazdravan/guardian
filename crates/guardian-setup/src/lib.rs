//! Guardian 3.0 setup: installs, checks, updates and removes the agent on
//! Windows. Layers: `model` (pure), `host` (what touches the OS, faked in
//! tests), `lifecycle` and `ops` (the operations), the window and the binary.

#![forbid(unsafe_code)]

pub mod cli;
#[cfg(feature = "testing")]
pub mod fake;
pub mod host;
pub mod lifecycle;
pub mod model;
pub mod ops;
#[cfg(windows)]
pub mod winhost;
