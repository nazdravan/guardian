//! The local fallback channel between a teacher console and an agent (§11 of
//! the 3.0 design): Noise IK over TCP with the existing ed25519 identities
//! converted to X25519, and signed UDP discovery on the LAN.

#![forbid(unsafe_code)]

pub mod channel;
pub mod discover;
