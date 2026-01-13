//! CLI module for makefilehub
//!
//! Provides command-line interface with the following subcommands:
//! - `mcp` - Start MCP server over stdio
//! - `run` - Run a task in a project
//! - `list` - List available tasks
//! - `detect` - Detect build system
//! - `config` - Show configuration
//! - `rebuild` - Rebuild a service with dependencies

pub mod commands;
pub mod mcp;

pub use commands::{Cli, Commands};
pub use mcp::run_mcp_server;
