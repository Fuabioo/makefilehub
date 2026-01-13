//! Command execution module
//!
//! Provides async command execution with:
//! - Timeout support
//! - Output capture and truncation
//! - Environment variable injection
//! - Working directory control

pub mod runner;

pub use runner::*;
