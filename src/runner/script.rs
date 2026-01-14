//! Custom script runner implementation
//!
//! Provides task listing and execution for custom scripts (run.sh, build.sh, etc.)
//!
//! # Task Detection Methods
//!
//! 1. **Parse --help output** - Extract commands from help text
//! 2. **Parse case statements** - Look for subcommand patterns in shell scripts
//! 3. **Config-defined tasks** - Use tasks from configuration
//!
//! # Argument Handling
//!
//! Scripts typically use: `./run.sh command arg1 arg2 --flag value`

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use once_cell::sync::Lazy;
use regex::Regex;

use super::traits::{RunOptions, RunResult, Runner, RunnerResult, TaskInfo};
use crate::error::{suggest_fix, TaskError};

// Static regex patterns - compiled once at first use
/// Matches "Commands:" or "Command:" section headers (case-insensitive)
static CMD_SECTION_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)commands?:").unwrap());

/// Matches command lines in help output: "  name    description"
static CMD_LINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s{2,4}([a-zA-Z_][a-zA-Z0-9_-]*)\s+(.*)$").unwrap());

/// Alternative command line format: "  name - description" or "  name : description"
static ALT_CMD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s{2,4}([a-zA-Z_][a-zA-Z0-9_-]*)\s+[-:]?\s*(.*)$").unwrap());

/// Matches case statement patterns: "  name)" or "  'name')" or "  \"name\")"
static CASE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^\s*["']?([a-zA-Z_][a-zA-Z0-9_-]*)["']?\s*\)"#).unwrap());

/// Matches function definitions: "name() {" or "function name()"
static FUNC_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"^(?:function\s+)?([a-zA-Z_][a-zA-Z0-9_-]*)\s*\(\s*\)"#).unwrap());

/// Matches comment lines: "# description" (with optional leading whitespace)
static SCRIPT_COMMENT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*#\s*(.*)$").unwrap());

/// Script runner for custom shell scripts
pub struct ScriptRunner {
    /// Name of the script (e.g., "run.sh", "build.sh")
    script_name: String,
    /// Shell to use for execution (defaults to "bash")
    shell: String,
}

impl Default for ScriptRunner {
    fn default() -> Self {
        Self::new("./run.sh")
    }
}

impl ScriptRunner {
    /// Create a new script runner for the given script
    pub fn new(script_name: impl Into<String>) -> Self {
        Self {
            script_name: script_name.into(),
            shell: "bash".to_string(),
        }
    }

    /// Create a script runner with a custom shell
    pub fn with_shell(mut self, shell: impl Into<String>) -> Self {
        self.shell = shell.into();
        self
    }

    /// Get the script name
    pub fn script_name(&self) -> &str {
        &self.script_name
    }

    /// Find an executable script in a directory
    ///
    /// Checks the configured script and returns the path if it exists and is executable.
    pub fn find_script(&self, dir: &Path) -> Option<std::path::PathBuf> {
        // Handle both ./run.sh and run.sh formats
        let script_name = self
            .script_name
            .strip_prefix("./")
            .unwrap_or(&self.script_name);
        let path = dir.join(script_name);

        if !path.exists() || !path.is_file() {
            return None;
        }

        // Check if executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = path.metadata() {
                let permissions = metadata.permissions();
                if permissions.mode() & 0o111 == 0 {
                    tracing::debug!("Script {} exists but is not executable", script_name);
                    return None;
                }
            }
        }

        Some(path)
    }

    /// Try to list commands by running script with --help
    fn list_via_help(&self, dir: &Path) -> RunnerResult<Vec<TaskInfo>> {
        let script_path = self
            .find_script(dir)
            .ok_or_else(|| TaskError::NoRunnerDetected {
                path: dir.display().to_string(),
                available: vec![],
            })?;

        let output = Command::new(&self.shell)
            .current_dir(dir)
            .arg(&script_path)
            .arg("--help")
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .output()
            .map_err(|e| TaskError::SpawnFailed {
                command: format!("{} {} --help", self.shell, self.script_name),
                error: e.to_string(),
            })?;

        // Combine stdout and stderr (some scripts output help to stderr)
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}\n{}", stdout, stderr);

        self.parse_help_output(&combined)
    }

    /// Parse --help output for commands
    ///
    /// Looks for patterns like:
    /// - `Commands:` followed by indented command names
    /// - `Usage: script <command>` patterns
    /// - `  command    Description` patterns
    fn parse_help_output(&self, output: &str) -> RunnerResult<Vec<TaskInfo>> {
        let mut tasks = Vec::new();

        // Pattern 1: Look for "Commands:" section
        // Using static regexes for performance (compiled once at first use)
        let mut in_commands_section = false;
        for line in output.lines() {
            if CMD_SECTION_RE.is_match(line) {
                in_commands_section = true;
                continue;
            }

            if in_commands_section {
                // Empty line or new section ends the commands section
                if line.trim().is_empty() && !tasks.is_empty() {
                    in_commands_section = false;
                    continue;
                }

                if let Some(caps) = CMD_LINE_RE.captures(line) {
                    let name = caps[1].to_string();
                    let desc = caps.get(2).map(|m| m.as_str().trim().to_string());

                    tasks.push(TaskInfo {
                        name,
                        description: if desc.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
                            None
                        } else {
                            desc
                        },
                        arguments: vec![],
                    });
                }
            }
        }

        // Pattern 2: Look for individual command descriptions
        // Format: "  command - description" or "  command    description"
        if tasks.is_empty() {
            for line in output.lines() {
                // Skip lines that look like options (start with -)
                if line.trim().starts_with('-') {
                    continue;
                }

                if let Some(caps) = ALT_CMD_RE.captures(line) {
                    let name = caps[1].to_string();
                    // Skip if name looks like an option or common word
                    if is_common_word(&name) {
                        continue;
                    }

                    let desc = caps.get(2).map(|m| m.as_str().trim().to_string());

                    // Avoid duplicates
                    if !tasks.iter().any(|t| t.name == name) {
                        tasks.push(TaskInfo {
                            name,
                            description: if desc.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
                                None
                            } else {
                                desc
                            },
                            arguments: vec![],
                        });
                    }
                }
            }
        }

        tasks.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tasks)
    }

    /// Parse script directly for case statement commands
    fn list_via_parse(&self, dir: &Path) -> RunnerResult<Vec<TaskInfo>> {
        let script_path = self
            .find_script(dir)
            .ok_or_else(|| TaskError::NoRunnerDetected {
                path: dir.display().to_string(),
                available: vec![],
            })?;

        let file = std::fs::File::open(&script_path).map_err(TaskError::Io)?;
        let reader = BufReader::new(file);

        let mut tasks = Vec::new();

        // Using static regexes for performance (compiled once at first use)
        let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

        for (i, line) in lines.iter().enumerate() {
            // Try case pattern match
            if let Some(caps) = CASE_RE.captures(line) {
                let name = caps[1].to_string();

                // Skip special case patterns
                if name == "*" || name == "help" && !tasks.is_empty() {
                    continue;
                }

                // Look for comment in previous line
                let description = if i > 0 {
                    SCRIPT_COMMENT_RE
                        .captures(&lines[i - 1])
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    None
                };

                if !tasks.iter().any(|t: &TaskInfo| t.name == name) {
                    tasks.push(TaskInfo {
                        name,
                        description,
                        arguments: vec![],
                    });
                }
            }

            // Try function definition match
            if let Some(caps) = FUNC_RE.captures(line) {
                let name = caps[1].to_string();

                // Skip common internal function names
                if is_internal_function(&name) {
                    continue;
                }

                let description = if i > 0 {
                    SCRIPT_COMMENT_RE
                        .captures(&lines[i - 1])
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    None
                };

                if !tasks.iter().any(|t| t.name == name) {
                    tasks.push(TaskInfo {
                        name,
                        description,
                        arguments: vec![],
                    });
                }
            }
        }

        tasks.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tasks)
    }

    /// Execute a script command
    fn execute_script(
        &self,
        dir: &Path,
        task: &str,
        options: &RunOptions,
    ) -> RunnerResult<RunResult> {
        let script_path = self
            .find_script(dir)
            .ok_or_else(|| TaskError::NoRunnerDetected {
                path: dir.display().to_string(),
                available: vec![],
            })?;

        let start = Instant::now();

        let mut cmd = Command::new(&self.shell);
        cmd.current_dir(dir);
        cmd.arg(&script_path);
        cmd.arg(task);

        // Add positional arguments first
        for arg in &options.positional_args {
            cmd.arg(arg);
        }

        // Add named arguments as --key value or --key=value
        for (key, value) in &options.args {
            if value.is_empty() {
                cmd.arg(format!("--{}", key));
            } else {
                cmd.arg(format!("--{}={}", key, value));
            }
        }

        // Set environment variables
        for (key, value) in &options.env {
            cmd.env(key, value);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let command_str = self.build_command(task, options);

        tracing::debug!("Executing: {}", command_str);

        let output = cmd.output().map_err(|e| TaskError::SpawnFailed {
            command: command_str.clone(),
            error: e.to_string(),
        })?;

        let duration_ms = start.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(RunResult::success(command_str, stdout, duration_ms))
        } else {
            let exit_code = output.status.code();

            // Check for common error patterns
            if stderr.contains("Unknown command")
                || stderr.contains("not a valid command")
                || stderr.contains("Invalid command")
                || stderr.contains("unrecognized command")
            {
                let available = self.list_tasks(dir).unwrap_or_default();
                let available_names: Vec<String> =
                    available.iter().map(|t| t.name.clone()).collect();

                return Err(TaskError::TaskNotFound {
                    task: task.to_string(),
                    available: available_names,
                    suggestion: suggest_fix(&command_str, &stderr),
                });
            }

            Ok(RunResult::failed(
                command_str,
                exit_code,
                stdout,
                stderr,
                duration_ms,
            ))
        }
    }
}

impl Runner for ScriptRunner {
    fn name(&self) -> &str {
        &self.script_name
    }

    fn list_tasks(&self, dir: &Path) -> RunnerResult<Vec<TaskInfo>> {
        // Verify script exists first
        if self.find_script(dir).is_none() {
            return Err(TaskError::NoRunnerDetected {
                path: dir.display().to_string(),
                available: vec![],
            });
        }

        // Try --help first
        match self.list_via_help(dir) {
            Ok(tasks) if !tasks.is_empty() => return Ok(tasks),
            Ok(_) => {
                tracing::debug!("No commands found via --help, trying parse");
            }
            Err(e) => {
                tracing::debug!("--help failed: {}, trying parse", e);
            }
        }

        // Fallback to parsing script directly
        self.list_via_parse(dir)
    }

    fn run_task(&self, dir: &Path, task: &str, options: &RunOptions) -> RunnerResult<RunResult> {
        // Verify script exists
        if self.find_script(dir).is_none() {
            return Err(TaskError::NoRunnerDetected {
                path: dir.display().to_string(),
                available: vec![],
            });
        }

        self.execute_script(dir, task, options)
    }

    fn build_command(&self, task: &str, options: &RunOptions) -> String {
        let mut parts = vec![self.script_name.clone(), task.to_string()];

        // Add positional arguments
        for arg in &options.positional_args {
            parts.push(arg.clone());
        }

        // Add named arguments
        for (key, value) in &options.args {
            if value.is_empty() {
                parts.push(format!("--{}", key));
            } else {
                parts.push(format!("--{}={}", key, value));
            }
        }

        parts.join(" ")
    }
}

/// Check if a word is a common non-command word
fn is_common_word(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "into"
            | "usage"
            | "options"
            | "arguments"
            | "description"
            | "example"
            | "examples"
            | "note"
            | "notes"
            | "see"
            | "also"
            | "more"
            | "info"
    )
}

/// Check if a function name is likely internal
fn is_internal_function(name: &str) -> bool {
    name.starts_with('_')
        || matches!(
            name,
            "main"
                | "usage"
                | "help"
                | "error"
                | "log"
                | "debug"
                | "info"
                | "warn"
                | "die"
                | "abort"
                | "exit"
                | "cleanup"
                | "setup"
                | "init"
                | "check"
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_dir_with_script(content: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        let script_path = dir.path().join("run.sh");
        fs::write(&script_path, content).unwrap();

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).unwrap();
        }

        dir
    }

    #[test]
    fn test_find_script_exists() {
        let dir = create_test_dir_with_script("#!/bin/bash\necho hello");
        let runner = ScriptRunner::new("./run.sh");

        let found = runner.find_script(dir.path());
        assert!(found.is_some());
    }

    #[test]
    fn test_find_script_without_prefix() {
        let dir = create_test_dir_with_script("#!/bin/bash\necho hello");
        let runner = ScriptRunner::new("run.sh");

        let found = runner.find_script(dir.path());
        assert!(found.is_some());
    }

    #[test]
    fn test_find_script_not_exists() {
        let dir = TempDir::new().unwrap();
        let runner = ScriptRunner::new("./run.sh");

        let found = runner.find_script(dir.path());
        assert!(found.is_none());
    }

    #[test]
    fn test_find_script_not_executable() {
        let dir = TempDir::new().unwrap();
        let script_path = dir.path().join("run.sh");
        fs::write(&script_path, "#!/bin/bash\necho hello").unwrap();
        // Don't make it executable

        let runner = ScriptRunner::new("./run.sh");

        #[cfg(unix)]
        {
            let found = runner.find_script(dir.path());
            assert!(found.is_none());
        }
    }

    #[test]
    fn test_parse_help_output_commands_section() {
        let runner = ScriptRunner::new("./run.sh");
        let output = r#"
Usage: run.sh <command>

Commands:
  build    Build the project
  test     Run tests
  deploy   Deploy to production

Options:
  --help   Show this help
"#;

        let tasks = runner.parse_help_output(output).unwrap();

        assert_eq!(tasks.len(), 3);
        assert!(tasks.iter().any(|t| t.name == "build"));
        assert!(tasks.iter().any(|t| t.name == "test"));
        assert!(tasks.iter().any(|t| t.name == "deploy"));

        let build = tasks.iter().find(|t| t.name == "build").unwrap();
        assert_eq!(build.description, Some("Build the project".to_string()));
    }

    #[test]
    fn test_parse_help_output_alt_format() {
        let runner = ScriptRunner::new("./run.sh");
        let output = r#"
Usage: run.sh [command]

  up      Start the services
  down    Stop the services
"#;

        let tasks = runner.parse_help_output(output).unwrap();

        assert!(tasks.iter().any(|t| t.name == "up"));
        assert!(tasks.iter().any(|t| t.name == "down"));
    }

    #[test]
    fn test_parse_script_case_statements() {
        let script = r#"#!/bin/bash

case "$1" in
  # Build the project
  build)
    echo "Building..."
    ;;
  # Run tests
  test)
    echo "Testing..."
    ;;
  up)
    docker-compose up -d
    ;;
  *)
    echo "Unknown command"
    ;;
esac
"#;
        let dir = create_test_dir_with_script(script);
        let runner = ScriptRunner::new("./run.sh");

        let tasks = runner.list_via_parse(dir.path()).unwrap();

        assert!(tasks.iter().any(|t| t.name == "build"));
        assert!(tasks.iter().any(|t| t.name == "test"));
        assert!(tasks.iter().any(|t| t.name == "up"));

        let build = tasks.iter().find(|t| t.name == "build").unwrap();
        assert_eq!(build.description, Some("Build the project".to_string()));
    }

    #[test]
    fn test_parse_script_functions() {
        let script = r#"#!/bin/bash

# Build the project
build() {
    echo "Building..."
}

# Run all tests
test() {
    echo "Testing..."
}

# Internal helper
_setup() {
    echo "Setup..."
}

"$@"
"#;
        let dir = create_test_dir_with_script(script);
        let runner = ScriptRunner::new("./run.sh");

        let tasks = runner.list_via_parse(dir.path()).unwrap();

        // Should find build and test, but not _setup
        assert!(tasks.iter().any(|t| t.name == "build"));
        assert!(tasks.iter().any(|t| t.name == "test"));
        assert!(!tasks.iter().any(|t| t.name == "_setup"));
    }

    #[test]
    fn test_build_command_simple() {
        let runner = ScriptRunner::new("./run.sh");
        let options = RunOptions::default();

        let cmd = runner.build_command("build", &options);
        assert_eq!(cmd, "./run.sh build");
    }

    #[test]
    fn test_build_command_with_args() {
        let runner = ScriptRunner::new("./run.sh");
        let options = RunOptions::default()
            .with_positional("arg1")
            .with_arg("verbose", "true");

        let cmd = runner.build_command("build", &options);
        assert!(cmd.contains("./run.sh build"));
        assert!(cmd.contains("arg1"));
        assert!(cmd.contains("--verbose=true"));
    }

    #[test]
    fn test_build_command_flag_only() {
        let runner = ScriptRunner::new("./run.sh");
        let options = RunOptions::default().with_arg("verbose", "");

        let cmd = runner.build_command("test", &options);
        assert!(cmd.contains("--verbose"));
        assert!(!cmd.contains("="));
    }

    #[test]
    fn test_runner_name() {
        let runner = ScriptRunner::new("./build.sh");
        assert_eq!(runner.name(), "./build.sh");
    }

    #[test]
    fn test_runner_with_shell() {
        let runner = ScriptRunner::new("./run.sh").with_shell("sh");
        assert_eq!(runner.shell, "sh");
    }

    #[test]
    fn test_list_tasks_no_script() {
        let dir = TempDir::new().unwrap();
        let runner = ScriptRunner::new("./run.sh");

        let result = runner.list_tasks(dir.path());
        assert!(result.is_err());

        match result.unwrap_err() {
            TaskError::NoRunnerDetected { .. } => {}
            _ => panic!("Expected NoRunnerDetected error"),
        }
    }

    #[test]
    fn test_run_task_no_script() {
        let dir = TempDir::new().unwrap();
        let runner = ScriptRunner::new("./run.sh");

        let result = runner.run_task(dir.path(), "build", &RunOptions::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_run_task_simple() {
        let script = r#"#!/bin/bash
case "$1" in
  echo-test)
    echo "test output"
    ;;
  *)
    echo "Unknown command"
    exit 1
    ;;
esac
"#;
        let dir = create_test_dir_with_script(script);
        let runner = ScriptRunner::new("./run.sh");

        let result = runner.run_task(dir.path(), "echo-test", &RunOptions::default());

        match result {
            Ok(run_result) => {
                assert!(run_result.success);
                assert!(run_result.stdout.contains("test output"));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: bash not available");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_run_task_with_args() {
        let script = r#"#!/bin/bash
echo "Command: $1"
echo "Arg: $2"
"#;
        let dir = create_test_dir_with_script(script);
        let runner = ScriptRunner::new("./run.sh");

        let options = RunOptions::default().with_positional("hello");
        let result = runner.run_task(dir.path(), "test", &options);

        match result {
            Ok(run_result) => {
                assert!(run_result.stdout.contains("Command: test"));
                assert!(run_result.stdout.contains("Arg: hello"));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: bash not available");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_run_task_failing() {
        let script = r#"#!/bin/bash
exit 1
"#;
        let dir = create_test_dir_with_script(script);
        let runner = ScriptRunner::new("./run.sh");

        let result = runner.run_task(dir.path(), "fail", &RunOptions::default());

        match result {
            Ok(run_result) => {
                assert!(!run_result.success);
                assert_eq!(run_result.exit_code, Some(1));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: bash not available");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_is_common_word() {
        assert!(is_common_word("usage"));
        assert!(is_common_word("Options"));
        assert!(is_common_word("DESCRIPTION"));

        assert!(!is_common_word("build"));
        assert!(!is_common_word("test"));
        assert!(!is_common_word("deploy"));
    }

    #[test]
    fn test_is_internal_function() {
        assert!(is_internal_function("_helper"));
        assert!(is_internal_function("main"));
        assert!(is_internal_function("usage"));
        assert!(is_internal_function("cleanup"));

        assert!(!is_internal_function("build"));
        assert!(!is_internal_function("deploy"));
        assert!(!is_internal_function("start"));
    }

    #[test]
    fn test_complex_script_parsing() {
        let script = r#"#!/bin/bash

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Show usage information
usage() {
    cat <<EOF
Usage: run.sh <command>

Commands:
  build    Build the project
  test     Run tests
  up       Start services
  down     Stop services

Options:
  --help   Show this help
EOF
}

# Build the project
build() {
    echo "Building..."
}

# Run tests
test_cmd() {
    echo "Testing..."
}

# Start services
up() {
    docker-compose up -d
}

# Stop services
down() {
    docker-compose down
}

case "$1" in
  build)
    build
    ;;
  test)
    test_cmd
    ;;
  up)
    up
    ;;
  down)
    down
    ;;
  --help|-h|help)
    usage
    ;;
  *)
    echo "Unknown command: $1"
    usage
    exit 1
    ;;
esac
"#;
        let dir = create_test_dir_with_script(script);
        let runner = ScriptRunner::new("./run.sh");

        let tasks = runner.list_via_parse(dir.path()).unwrap();

        // Should find all commands from case statement
        let names: Vec<&str> = tasks.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"build"));
        assert!(names.contains(&"test"));
        assert!(names.contains(&"up"));
        assert!(names.contains(&"down"));
    }

    // TDD: Tests for static regex patterns (Step 2+3 of v0.1.0 cleanup)
    #[test]
    fn test_cmd_section_regex_matches() {
        assert!(CMD_SECTION_RE.is_match("Commands:"));
        assert!(CMD_SECTION_RE.is_match("commands:"));
        assert!(CMD_SECTION_RE.is_match("COMMANDS:"));
        assert!(CMD_SECTION_RE.is_match("Command:"));
        assert!(!CMD_SECTION_RE.is_match("Options:"));
    }

    #[test]
    fn test_cmd_line_regex_matches() {
        assert!(CMD_LINE_RE.is_match("  build    Build the project"));
        assert!(CMD_LINE_RE.is_match("    test   Run tests"));
        assert!(CMD_LINE_RE.captures("  deploy   Deploy to prod").is_some());

        let caps = CMD_LINE_RE
            .captures("  build    Build the project")
            .unwrap();
        assert_eq!(&caps[1], "build");
    }

    #[test]
    fn test_case_regex_matches() {
        assert!(CASE_RE.is_match("  build)"));
        assert!(CASE_RE.is_match("    test)"));
        assert!(CASE_RE.is_match(r#"  "deploy")"#));
        assert!(CASE_RE.is_match("  'start')"));
        assert!(!CASE_RE.is_match("  *)")); // Wildcard shouldn't match

        let caps = CASE_RE.captures("  build)").unwrap();
        assert_eq!(&caps[1], "build");
    }

    #[test]
    fn test_func_regex_matches() {
        assert!(FUNC_RE.is_match("build() {"));
        assert!(FUNC_RE.is_match("function test()"));
        assert!(FUNC_RE.is_match("deploy ()"));
        // Note: _private matches regex, but is_internal_function() filters it later
        assert!(FUNC_RE.is_match("_private() {"));

        let caps = FUNC_RE.captures("build() {").unwrap();
        assert_eq!(&caps[1], "build");
    }

    #[test]
    fn test_comment_regex_matches() {
        assert!(SCRIPT_COMMENT_RE.is_match("# Build the project"));
        assert!(SCRIPT_COMMENT_RE.is_match("  # Indented comment"));

        let caps = SCRIPT_COMMENT_RE.captures("# Build the project").unwrap();
        assert_eq!(&caps[1], "Build the project");
    }
}
