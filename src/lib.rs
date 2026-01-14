//! makefilehub - General-Purpose Build System MCP Server
//!
//! Provides a unified interface for running tasks across different build systems:
//! - **Makefile** - Standard make
//! - **justfile** - just command runner
//! - **Custom scripts** - run.sh, build.sh, etc.
//!
//! ## Features
//!
//! - Auto-detection of build systems by priority
//! - XDG-compliant layered configuration
//! - Environment variable and shell command interpolation
//! - Service dependency management for complex rebuild orchestration
//! - MCP tools for Claude Code integration
//!
//! ## MCP Tools
//!
//! - `run_task` - Run a task/target in a project
//! - `rebuild_service` - Build service with dependency handling
//! - `list_tasks` - List available tasks/targets
//! - `detect_runner` - Detect which build system a project uses
//! - `get_project_config` - Get resolved configuration

pub mod cli;
pub mod config;
pub mod error;
pub mod executor;
pub mod mcp;
pub mod runner;

pub use cli::{Cli, Commands};
pub use config::Config;
pub use error::{ErrorInfo, TaskError};
pub use executor::{
    exec_command, exec_command_sync, exec_shell_command, ExecOptions, ExecResult, TaskExecutor,
};
pub use mcp::MakefilehubServer;
pub use runner::{
    detect_runner, DetectionResult, FilesFound, JustfileRunner, MakefileRunner, RunnerType,
    ScriptRunner,
};
