//! Guardian Diag core: everything that can be decided without an operating system.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod agent;

pub mod parse;
pub mod paths;
pub mod read;
