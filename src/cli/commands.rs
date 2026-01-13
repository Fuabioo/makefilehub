//! CLI command definitions using clap
//!
//! Defines all CLI subcommands and their arguments.

use clap::{Parser, Subcommand, ValueEnum};
use std::collections::HashMap;

/// General-purpose build system runner and MCP server.
///
/// Auto-detects and runs tasks across Makefile, justfile, and custom scripts.
/// Can be used as a standalone CLI or as an MCP server for Claude Code.
#[derive(Parser, Debug)]
#[command(name = "makefilehub")]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Config file path (overrides default XDG paths)
    #[arg(short, long, global = true)]
    pub config: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

/// Available CLI subcommands
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start MCP server over stdio (for Claude Code integration)
    Mcp,

    /// Run a task/target in a project
    Run(RunArgs),

    /// List available tasks/targets in a project
    List(ListArgs),

    /// Detect which build system a project uses
    Detect(DetectArgs),

    /// Show resolved configuration for a project
    Config(ConfigArgs),

    /// Rebuild a service with dependency handling
    Rebuild(RebuildArgs),
}

/// Arguments for the `run` subcommand
#[derive(Parser, Debug)]
pub struct RunArgs {
    /// Task name to run (e.g., build, test, up)
    #[arg(required = true)]
    pub task: String,

    /// Project path or name (defaults to current directory)
    #[arg(short, long)]
    pub project: Option<String>,

    /// Force specific runner (make, just, or script name)
    #[arg(short, long)]
    pub runner: Option<String>,

    /// Named arguments in KEY=VALUE format
    #[arg(short = 'a', long = "arg", value_parser = parse_key_value)]
    pub args: Vec<(String, String)>,

    /// Positional arguments passed after task
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub positional: Vec<String>,

    /// Timeout in seconds (0 for no timeout)
    #[arg(short, long, default_value = "300")]
    pub timeout: u64,

    /// Don't capture output, stream directly
    #[arg(long)]
    pub stream: bool,
}

impl RunArgs {
    /// Convert args to a HashMap
    pub fn args_as_map(&self) -> HashMap<String, String> {
        self.args.iter().cloned().collect()
    }
}

/// Parse KEY=VALUE argument
fn parse_key_value(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid argument '{}': expected KEY=VALUE format", s))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

/// Arguments for the `list` subcommand
#[derive(Parser, Debug)]
pub struct ListArgs {
    /// Project path or name (defaults to current directory)
    #[arg(short, long)]
    pub project: Option<String>,

    /// Force specific runner
    #[arg(short, long)]
    pub runner: Option<String>,

    /// Output format
    #[arg(short, long, value_enum, default_value = "table")]
    pub format: OutputFormat,
}

/// Output format options
#[derive(Debug, Clone, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable table format
    Table,
    /// JSON output
    Json,
    /// Plain text (one task per line)
    Plain,
}

/// Arguments for the `detect` subcommand
#[derive(Parser, Debug)]
pub struct DetectArgs {
    /// Project path (defaults to current directory)
    #[arg(short, long)]
    pub project: Option<String>,

    /// Output format
    #[arg(short, long, value_enum, default_value = "table")]
    pub format: OutputFormat,
}

/// Arguments for the `config` subcommand
#[derive(Parser, Debug)]
pub struct ConfigArgs {
    /// Project name or path (required)
    pub project: String,

    /// Output format
    #[arg(short, long, value_enum, default_value = "table")]
    pub format: OutputFormat,

    /// Show raw config without interpolation
    #[arg(long)]
    pub raw: bool,
}

/// Arguments for the `rebuild` subcommand
#[derive(Parser, Debug)]
pub struct RebuildArgs {
    /// Service to rebuild
    #[arg(required = true)]
    pub service: String,

    /// Additional services to rebuild
    #[arg(short = 's', long)]
    pub services: Vec<String>,

    /// Skip dependency restart
    #[arg(long)]
    pub skip_deps: bool,

    /// Skip force-recreate of containers
    #[arg(long)]
    pub skip_recreate: bool,

    /// Timeout in seconds
    #[arg(short, long, default_value = "600")]
    pub timeout: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli_parse_mcp() {
        let cli = Cli::parse_from(["makefilehub", "mcp"]);
        assert!(matches!(cli.command, Commands::Mcp));
        assert!(!cli.verbose);
    }

    #[test]
    fn test_cli_parse_run_simple() {
        let cli = Cli::parse_from(["makefilehub", "run", "build"]);
        if let Commands::Run(args) = cli.command {
            assert_eq!(args.task, "build");
            assert!(args.project.is_none());
            assert!(args.runner.is_none());
        } else {
            panic!("Expected Run command");
        }
    }

    #[test]
    fn test_cli_parse_run_with_project() {
        let cli = Cli::parse_from(["makefilehub", "run", "test", "-p", "/tmp/myproject"]);
        if let Commands::Run(args) = cli.command {
            assert_eq!(args.task, "test");
            assert_eq!(args.project, Some("/tmp/myproject".to_string()));
        } else {
            panic!("Expected Run command");
        }
    }

    #[test]
    fn test_cli_parse_run_with_args() {
        let cli = Cli::parse_from([
            "makefilehub",
            "run",
            "build",
            "-a",
            "TARGET=release",
            "-a",
            "DEBUG=0",
        ]);
        if let Commands::Run(args) = cli.command {
            assert_eq!(args.task, "build");
            let args_map = args.args_as_map();
            assert_eq!(args_map.get("TARGET"), Some(&"release".to_string()));
            assert_eq!(args_map.get("DEBUG"), Some(&"0".to_string()));
        } else {
            panic!("Expected Run command");
        }
    }

    #[test]
    fn test_cli_parse_run_with_runner() {
        let cli = Cli::parse_from(["makefilehub", "run", "build", "-r", "just"]);
        if let Commands::Run(args) = cli.command {
            assert_eq!(args.runner, Some("just".to_string()));
        } else {
            panic!("Expected Run command");
        }
    }

    #[test]
    fn test_cli_parse_list() {
        let cli = Cli::parse_from(["makefilehub", "list", "-p", "/tmp/project"]);
        if let Commands::List(args) = cli.command {
            assert_eq!(args.project, Some("/tmp/project".to_string()));
            assert!(matches!(args.format, OutputFormat::Table));
        } else {
            panic!("Expected List command");
        }
    }

    #[test]
    fn test_cli_parse_list_json() {
        let cli = Cli::parse_from(["makefilehub", "list", "-f", "json"]);
        if let Commands::List(args) = cli.command {
            assert!(matches!(args.format, OutputFormat::Json));
        } else {
            panic!("Expected List command");
        }
    }

    #[test]
    fn test_cli_parse_detect() {
        let cli = Cli::parse_from(["makefilehub", "detect"]);
        assert!(matches!(cli.command, Commands::Detect(_)));
    }

    #[test]
    fn test_cli_parse_config() {
        let cli = Cli::parse_from(["makefilehub", "config", "myservice"]);
        if let Commands::Config(args) = cli.command {
            assert_eq!(args.project, "myservice");
        } else {
            panic!("Expected Config command");
        }
    }

    #[test]
    fn test_cli_parse_rebuild() {
        let cli = Cli::parse_from([
            "makefilehub",
            "rebuild",
            "web-api",
            "-s",
            "web-frontend",
            "--skip-deps",
        ]);
        if let Commands::Rebuild(args) = cli.command {
            assert_eq!(args.service, "web-api");
            assert_eq!(args.services, vec!["web-frontend".to_string()]);
            assert!(args.skip_deps);
            assert!(!args.skip_recreate);
        } else {
            panic!("Expected Rebuild command");
        }
    }

    #[test]
    fn test_cli_verbose_flag() {
        let cli = Cli::parse_from(["makefilehub", "-v", "mcp"]);
        assert!(cli.verbose);
    }

    #[test]
    fn test_cli_config_flag() {
        let cli = Cli::parse_from(["makefilehub", "-c", "/path/to/config.toml", "mcp"]);
        assert_eq!(cli.config, Some("/path/to/config.toml".to_string()));
    }

    #[test]
    fn test_parse_key_value_valid() {
        let result = parse_key_value("FOO=bar");
        assert_eq!(result, Ok(("FOO".to_string(), "bar".to_string())));
    }

    #[test]
    fn test_parse_key_value_empty_value() {
        let result = parse_key_value("FOO=");
        assert_eq!(result, Ok(("FOO".to_string(), "".to_string())));
    }

    #[test]
    fn test_parse_key_value_with_equals() {
        let result = parse_key_value("FOO=bar=baz");
        assert_eq!(result, Ok(("FOO".to_string(), "bar=baz".to_string())));
    }

    #[test]
    fn test_parse_key_value_invalid() {
        let result = parse_key_value("INVALID");
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_verify() {
        // Verify CLI structure is valid
        Cli::command().debug_assert();
    }
}
