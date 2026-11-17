//! Guardian 3.0 agent: enrollment, the server connection, process enforcement,
//! the worker in the user session.

#![forbid(unsafe_code)]

mod agent;
pub mod config;
pub mod conn;
pub mod engine;
pub mod store;
pub mod worker;

pub use agent::{
    AGENT_VERSION, AgentError, AgentHandle, AgentOptions, AgentStatus, CAPABILITIES, enroll, run,
};
