//! MCP Server module
//!
//! Provides MCP tools for build system interaction:
//! - `run_task` - Run a task/target in a project
//! - `rebuild_service` - Build service with dependency handling
//! - `list_tasks` - List available tasks/targets
//! - `detect_runner` - Detect which build system a project uses
//! - `get_project_config` - Get resolved configuration

pub mod server;

pub use server::MakefilehubServer;
