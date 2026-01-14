//! Makefile runner implementation
//!
//! Provides task listing and execution for GNU Make projects.
//!
//! # Task Detection Methods
//!
//! 1. **Parse Makefile directly** - Extract targets from the file
//! 2. **make -pRrq** - Query make's database for available targets
//!
//! # Argument Handling
//!
//! Make supports variable assignment: `make target VAR1=value1 VAR2=value2`

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use once_cell::sync::Lazy;
use regex::Regex;

use super::traits::{RunOptions, RunResult, Runner, RunnerResult, TaskArg, TaskInfo};
use crate::error::{suggest_fix, TaskError};

// Static regex patterns - compiled once at first use
/// Matches Makefile target definitions: "name:"
static TARGET_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([a-zA-Z_][a-zA-Z0-9_-]*)\s*:").unwrap());

/// Matches comment descriptions: "## description" or "# target: description"
static COMMENT_DESC_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^##\s*(.+)$|^#\s*([a-zA-Z_][a-zA-Z0-9_-]*)\s*:\s*(.+)$").unwrap());

/// Matches Make variable references: $(VAR) or ${VAR}
static MAKE_ARG_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\$[({]([A-Z_][A-Z0-9_]*)[)}]").unwrap());

/// Makefile runner for GNU Make
pub struct MakefileRunner {
    /// Path to the make command
    make_command: String,
}

impl Default for MakefileRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl MakefileRunner {
    /// Create a new Makefile runner using system `make`
    pub fn new() -> Self {
        Self {
            make_command: "make".to_string(),
        }
    }

    /// Create a Makefile runner with a custom make command path
    pub fn with_command(command: impl Into<String>) -> Self {
        Self {
            make_command: command.into(),
        }
    }

    /// Find the Makefile in a directory
    ///
    /// Checks for: Makefile, makefile, GNUmakefile
    pub fn find_makefile(dir: &Path) -> Option<std::path::PathBuf> {
        for name in &["Makefile", "makefile", "GNUmakefile"] {
            let path = dir.join(name);
            if path.exists() && path.is_file() {
                return Some(path);
            }
        }
        None
    }

    /// Parse targets directly from a Makefile
    ///
    /// Extracts targets and their descriptions from comments.
    /// Format: `# target: description` followed by `target:`
    fn parse_makefile(&self, makefile_path: &Path) -> RunnerResult<Vec<TaskInfo>> {
        let file = std::fs::File::open(makefile_path).map_err(|e| TaskError::Io(e))?;

        let reader = BufReader::new(file);
        let mut tasks = Vec::new();
        let mut seen_targets: HashSet<String> = HashSet::new();

        // Using static regexes for performance (compiled once at first use)
        let lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();

        for (i, line) in lines.iter().enumerate() {
            // Check if this line defines a target
            if let Some(caps) = TARGET_RE.captures(line) {
                let target_name = caps[1].to_string();

                // Skip variable assignments (VAR :=, VAR ?=, VAR +=, VAR =)
                // Check what follows the colon
                let after_name = &line[caps.get(0).unwrap().end().saturating_sub(1)..];
                if after_name.starts_with(":=")
                    || after_name.starts_with("::=")
                    || after_name.starts_with("?=")
                    || after_name.starts_with("+=")
                {
                    continue;
                }
                // Also skip simple assignments where VAR = (colon is part of name match)
                // This catches cases where the colon is immediately followed by = without space
                if line.contains(":=")
                    || line.contains("?=")
                    || line.contains("+=")
                    || line.contains("::=")
                {
                    continue;
                }

                // Skip if we've already seen this target
                if seen_targets.contains(&target_name) {
                    continue;
                }

                // Skip special targets
                if target_name.starts_with('.') {
                    continue;
                }

                seen_targets.insert(target_name.clone());

                // Look for description in the previous line(s)
                let description = if i > 0 {
                    self.extract_description(&lines[..i], &target_name)
                } else {
                    None
                };

                // Look for arguments in the target's recipe
                let arguments = self.extract_make_args(&lines, i);

                tasks.push(TaskInfo {
                    name: target_name,
                    description,
                    arguments,
                });
            }
        }

        // Sort targets alphabetically
        tasks.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(tasks)
    }

    /// Extract description from comments above a target
    fn extract_description(&self, lines_before: &[String], target_name: &str) -> Option<String> {
        // Look at the line immediately before the target
        if let Some(prev_line) = lines_before.last() {
            if let Some(caps) = COMMENT_DESC_RE.captures(prev_line) {
                // Check for "## description" format
                if let Some(desc) = caps.get(1) {
                    return Some(desc.as_str().trim().to_string());
                }
                // Check for "# target: description" format
                if let Some(name) = caps.get(2) {
                    if name.as_str() == target_name {
                        if let Some(desc) = caps.get(3) {
                            return Some(desc.as_str().trim().to_string());
                        }
                    }
                }
            }
        }
        None
    }

    /// Extract arguments from a target's recipe (variable references)
    fn extract_make_args(&self, lines: &[String], target_line: usize) -> Vec<TaskArg> {
        let mut args: HashSet<String> = HashSet::new();

        // Look at lines following the target (recipe lines start with tab)
        for line in lines.iter().skip(target_line + 1) {
            // Recipe lines start with tab
            if !line.starts_with('\t') && !line.is_empty() {
                break;
            }

            // Find variable references (using static regex)
            for caps in MAKE_ARG_RE.captures_iter(line) {
                let var_name = caps[1].to_string();
                // Skip common built-in variables
                if !is_builtin_make_var(&var_name) {
                    args.insert(var_name);
                }
            }
        }

        // Convert to sorted vec
        let mut args_vec: Vec<TaskArg> = args
            .into_iter()
            .map(|name| TaskArg {
                name,
                required: false, // Make vars are optional by default
                default: None,
                description: None,
            })
            .collect();

        args_vec.sort_by(|a, b| a.name.cmp(&b.name));
        args_vec
    }

    /// List targets using make's database query
    ///
    /// Uses: `make -pRrq : 2>/dev/null | awk -F: '/^[a-zA-Z0-9_-]+:/ {print $1}'`
    fn list_targets_via_make(&self, dir: &Path) -> RunnerResult<Vec<TaskInfo>> {
        let output = Command::new(&self.make_command)
            .current_dir(dir)
            .args(["-pRrq", ":"])
            .stderr(Stdio::null())
            .output()
            .map_err(|e| TaskError::SpawnFailed {
                command: format!("{} -pRrq :", self.make_command),
                error: e.to_string(),
            })?;

        // Parse the output for targets
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut targets: HashSet<String> = HashSet::new();

        // Using static regex for performance (compiled once at first use)
        for line in stdout.lines() {
            // Skip lines that are not target definitions
            if line.starts_with('#') || line.starts_with('\t') || line.is_empty() {
                continue;
            }

            if let Some(caps) = TARGET_RE.captures(line) {
                let target = caps[1].to_string();
                // Skip special targets
                if !target.starts_with('.') {
                    targets.insert(target);
                }
            }
        }

        let mut tasks: Vec<TaskInfo> = targets.into_iter().map(TaskInfo::new).collect();

        tasks.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(tasks)
    }

    /// Execute a make target
    fn execute_make(
        &self,
        dir: &Path,
        task: &str,
        options: &RunOptions,
    ) -> RunnerResult<RunResult> {
        let start = Instant::now();

        let mut cmd = Command::new(&self.make_command);
        cmd.current_dir(dir);
        cmd.arg(task);

        // Add named arguments as VAR=value
        for (key, value) in &options.args {
            cmd.arg(format!("{}={}", key, value));
        }

        // Add positional arguments after --
        if !options.positional_args.is_empty() {
            cmd.arg("--");
            for arg in &options.positional_args {
                cmd.arg(arg);
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

            // Check if task exists to provide better error
            if stderr.contains("No rule to make target") {
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
                command_str.clone(),
                exit_code,
                stdout,
                stderr.clone(),
                duration_ms,
            ))
        }
    }
}

impl Runner for MakefileRunner {
    fn name(&self) -> &str {
        "make"
    }

    fn list_tasks(&self, dir: &Path) -> RunnerResult<Vec<TaskInfo>> {
        // Prefer parsing Makefile directly for better descriptions
        if let Some(makefile_path) = Self::find_makefile(dir) {
            match self.parse_makefile(&makefile_path) {
                Ok(tasks) if !tasks.is_empty() => return Ok(tasks),
                Ok(_) => {
                    tracing::debug!("No targets found in Makefile, trying make -pRrq");
                }
                Err(e) => {
                    tracing::warn!("Failed to parse Makefile directly: {}", e);
                }
            }

            // Fallback to make's database
            self.list_targets_via_make(dir)
        } else {
            Err(TaskError::NoRunnerDetected {
                path: dir.display().to_string(),
                available: vec![],
            })
        }
    }

    fn run_task(&self, dir: &Path, task: &str, options: &RunOptions) -> RunnerResult<RunResult> {
        // Verify Makefile exists
        if Self::find_makefile(dir).is_none() {
            return Err(TaskError::NoRunnerDetected {
                path: dir.display().to_string(),
                available: vec![],
            });
        }

        self.execute_make(dir, task, options)
    }

    fn build_command(&self, task: &str, options: &RunOptions) -> String {
        let mut parts = vec![self.make_command.clone(), task.to_string()];

        // Add named arguments
        for (key, value) in &options.args {
            parts.push(format!("{}={}", key, value));
        }

        // Add positional arguments
        if !options.positional_args.is_empty() {
            parts.push("--".to_string());
            for arg in &options.positional_args {
                parts.push(arg.clone());
            }
        }

        parts.join(" ")
    }
}

/// Check if a variable name is a built-in Make variable
fn is_builtin_make_var(name: &str) -> bool {
    matches!(
        name,
        "MAKE"
            | "MAKEFLAGS"
            | "MAKEFILES"
            | "MAKELEVEL"
            | "MAKECMDGOALS"
            | "CURDIR"
            | "SHELL"
            | "PATH"
            | "HOME"
            | "USER"
            | "CC"
            | "CXX"
            | "CFLAGS"
            | "CXXFLAGS"
            | "LDFLAGS"
            | "AR"
            | "RM"
            | "ARFLAGS"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_dir_with_makefile(content: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Makefile"), content).unwrap();
        dir
    }

    #[test]
    fn test_find_makefile_uppercase() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Makefile"), "build:").unwrap();

        let found = MakefileRunner::find_makefile(dir.path());
        assert!(found.is_some());
        assert!(found.unwrap().ends_with("Makefile"));
    }

    #[test]
    fn test_find_makefile_lowercase() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("makefile"), "build:").unwrap();

        let found = MakefileRunner::find_makefile(dir.path());
        assert!(found.is_some());
    }

    #[test]
    fn test_find_makefile_gnu() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("GNUmakefile"), "build:").unwrap();

        let found = MakefileRunner::find_makefile(dir.path());
        assert!(found.is_some());
        assert!(found.unwrap().ends_with("GNUmakefile"));
    }

    #[test]
    fn test_find_makefile_priority() {
        let dir = TempDir::new().unwrap();
        // Create both - Makefile should win
        fs::write(dir.path().join("Makefile"), "build:").unwrap();
        fs::write(dir.path().join("makefile"), "other:").unwrap();

        let found = MakefileRunner::find_makefile(dir.path());
        assert!(found.unwrap().ends_with("Makefile"));
    }

    #[test]
    fn test_find_makefile_none() {
        let dir = TempDir::new().unwrap();

        let found = MakefileRunner::find_makefile(dir.path());
        assert!(found.is_none());
    }

    #[test]
    fn test_parse_simple_targets() {
        let makefile = r#"
build:
	@echo building

test:
	@echo testing

clean:
	rm -rf dist/
"#;
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        let tasks = runner.list_tasks(dir.path()).unwrap();

        assert!(tasks.iter().any(|t| t.name == "build"));
        assert!(tasks.iter().any(|t| t.name == "test"));
        assert!(tasks.iter().any(|t| t.name == "clean"));
    }

    #[test]
    fn test_parse_targets_with_descriptions() {
        let makefile = r#"
## Build the project
build:
	@echo building

# test: Run all tests
test:
	@echo testing
"#;
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        let tasks = runner.list_tasks(dir.path()).unwrap();

        let build_task = tasks.iter().find(|t| t.name == "build").unwrap();
        assert_eq!(
            build_task.description,
            Some("Build the project".to_string())
        );

        let test_task = tasks.iter().find(|t| t.name == "test").unwrap();
        assert_eq!(test_task.description, Some("Run all tests".to_string()));
    }

    #[test]
    fn test_parse_targets_with_dependencies() {
        let makefile = r#"
all: build test

build:
	@echo building

test: build
	@echo testing
"#;
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        let tasks = runner.list_tasks(dir.path()).unwrap();

        assert!(tasks.iter().any(|t| t.name == "all"));
        assert!(tasks.iter().any(|t| t.name == "build"));
        assert!(tasks.iter().any(|t| t.name == "test"));
    }

    #[test]
    fn test_parse_skips_phony() {
        let makefile = r#"
.PHONY: build test

build:
	@echo building

.DEFAULT_GOAL := build
"#;
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        let tasks = runner.list_tasks(dir.path()).unwrap();

        // Should not include .PHONY or .DEFAULT_GOAL
        assert!(tasks.iter().all(|t| !t.name.starts_with('.')));
        // Should include build
        assert!(tasks.iter().any(|t| t.name == "build"));
    }

    #[test]
    fn test_parse_targets_with_variables() {
        let makefile = r#"
## Build with target
build:
	@echo "Building for $(TARGET)"
	@echo "Config: $(CONFIG_FILE)"
"#;
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        let tasks = runner.list_tasks(dir.path()).unwrap();
        let build_task = tasks.iter().find(|t| t.name == "build").unwrap();

        // Should detect TARGET and CONFIG_FILE as arguments
        assert!(build_task.arguments.iter().any(|a| a.name == "TARGET"));
        assert!(build_task.arguments.iter().any(|a| a.name == "CONFIG_FILE"));
    }

    #[test]
    fn test_parse_skips_builtin_vars() {
        let makefile = r#"
build:
	$(MAKE) -C subdir
	$(CC) -o output $(CFLAGS) src.c
"#;
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        let tasks = runner.list_tasks(dir.path()).unwrap();
        let build_task = tasks.iter().find(|t| t.name == "build").unwrap();

        // Should NOT include MAKE, CC, CFLAGS (built-ins)
        assert!(build_task.arguments.iter().all(|a| a.name != "MAKE"));
        assert!(build_task.arguments.iter().all(|a| a.name != "CC"));
        assert!(build_task.arguments.iter().all(|a| a.name != "CFLAGS"));
    }

    #[test]
    fn test_build_command_simple() {
        let runner = MakefileRunner::new();
        let options = RunOptions::default();

        let cmd = runner.build_command("build", &options);
        assert_eq!(cmd, "make build");
    }

    #[test]
    fn test_build_command_with_args() {
        let runner = MakefileRunner::new();
        let mut options = RunOptions::default();
        options
            .args
            .insert("TARGET".to_string(), "release".to_string());
        options.args.insert("VERBOSE".to_string(), "1".to_string());

        let cmd = runner.build_command("build", &options);
        assert!(cmd.contains("make build"));
        assert!(cmd.contains("TARGET=release"));
        assert!(cmd.contains("VERBOSE=1"));
    }

    #[test]
    fn test_build_command_with_positional() {
        let runner = MakefileRunner::new();
        let options = RunOptions::default()
            .with_positional("arg1")
            .with_positional("arg2");

        let cmd = runner.build_command("test", &options);
        assert!(cmd.contains("make test -- arg1 arg2"));
    }

    #[test]
    fn test_build_command_with_custom_make() {
        let runner = MakefileRunner::with_command("gmake");
        let options = RunOptions::default();

        let cmd = runner.build_command("build", &options);
        assert!(cmd.starts_with("gmake"));
    }

    #[test]
    fn test_runner_name() {
        let runner = MakefileRunner::new();
        assert_eq!(runner.name(), "make");
    }

    #[test]
    fn test_list_tasks_no_makefile() {
        let dir = TempDir::new().unwrap();
        let runner = MakefileRunner::new();

        let result = runner.list_tasks(dir.path());
        assert!(result.is_err());

        match result.unwrap_err() {
            TaskError::NoRunnerDetected { path, .. } => {
                assert!(path.contains(dir.path().to_str().unwrap()));
            }
            _ => panic!("Expected NoRunnerDetected error"),
        }
    }

    #[test]
    fn test_run_task_no_makefile() {
        let dir = TempDir::new().unwrap();
        let runner = MakefileRunner::new();

        let result = runner.run_task(dir.path(), "build", &RunOptions::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_run_task_simple() {
        let makefile = r#"
.PHONY: echo-test
echo-test:
	@echo "test output"
"#;
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        let result = runner.run_task(dir.path(), "echo-test", &RunOptions::default());

        // This should work if make is installed
        match result {
            Ok(run_result) => {
                assert!(run_result.success);
                assert!(run_result.stdout.contains("test output"));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                // make not installed, skip test
                eprintln!("Skipping test: make not installed");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_run_task_with_variables() {
        let makefile = r#"
.PHONY: show-var
show-var:
	@echo "Value: $(MY_VAR)"
"#;
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        let options = RunOptions::default().with_arg("MY_VAR", "hello");

        let result = runner.run_task(dir.path(), "show-var", &options);

        match result {
            Ok(run_result) => {
                assert!(run_result.success);
                assert!(run_result.stdout.contains("Value: hello"));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: make not installed");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_run_task_failing() {
        let makefile = r#"
.PHONY: fail
fail:
	@exit 1
"#;
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        let result = runner.run_task(dir.path(), "fail", &RunOptions::default());

        match result {
            Ok(run_result) => {
                assert!(!run_result.success);
                assert_eq!(run_result.exit_code, Some(2)); // make returns 2 on recipe failure
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: make not installed");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_run_task_nonexistent() {
        let makefile = "build:\n\t@echo building\n";
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        let result = runner.run_task(dir.path(), "nonexistent", &RunOptions::default());

        match result {
            Err(TaskError::TaskNotFound { task, .. }) => {
                assert_eq!(task, "nonexistent");
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: make not installed");
            }
            Ok(_) => panic!("Expected TaskNotFound error"),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_task_exists() {
        let makefile = "build:\n\t@echo building\n\ntest:\n\t@echo testing\n";
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        assert!(runner.task_exists(dir.path(), "build").unwrap());
        assert!(runner.task_exists(dir.path(), "test").unwrap());
        assert!(!runner.task_exists(dir.path(), "nonexistent").unwrap());
    }

    #[test]
    fn test_is_builtin_make_var() {
        assert!(is_builtin_make_var("MAKE"));
        assert!(is_builtin_make_var("CC"));
        assert!(is_builtin_make_var("CFLAGS"));
        assert!(is_builtin_make_var("PATH"));

        assert!(!is_builtin_make_var("MY_VAR"));
        assert!(!is_builtin_make_var("TARGET"));
        assert!(!is_builtin_make_var("CONFIG"));
    }

    #[test]
    fn test_parse_complex_makefile() {
        let makefile = r#"
# Makefile for my project

SHELL := /bin/bash
.DEFAULT_GOAL := all

.PHONY: all build test clean install

## Build and test everything
all: build test

## Build the project
build:
	@echo "Building $(TARGET)"
	cargo build --release

## Run all tests
test: build
	cargo test

## Clean build artifacts
clean:
	rm -rf target/

## Install the binary
install: build
	cargo install --path .
"#;
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        let tasks = runner.list_tasks(dir.path()).unwrap();

        // Should find all targets
        let names: Vec<&str> = tasks.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"all"));
        assert!(names.contains(&"build"));
        assert!(names.contains(&"test"));
        assert!(names.contains(&"clean"));
        assert!(names.contains(&"install"));

        // Should have descriptions
        let build = tasks.iter().find(|t| t.name == "build").unwrap();
        assert_eq!(build.description, Some("Build the project".to_string()));

        // build should have TARGET as argument
        assert!(build.arguments.iter().any(|a| a.name == "TARGET"));
    }

    #[test]
    fn test_no_duplicate_targets() {
        let makefile = r#"
build:
	@echo first

build:
	@echo second
"#;
        let dir = create_test_dir_with_makefile(makefile);
        let runner = MakefileRunner::new();

        let tasks = runner.list_tasks(dir.path()).unwrap();

        // Should only have one "build" target
        let build_count = tasks.iter().filter(|t| t.name == "build").count();
        assert_eq!(build_count, 1);
    }
}
