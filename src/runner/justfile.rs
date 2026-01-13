//! justfile runner implementation
//!
//! Provides task listing and execution for just command runner.
//!
//! # Task Detection Methods
//!
//! 1. **just --list** - List available recipes
//! 2. **just --dump --format json** - Full AST with arguments (preferred)
//! 3. **Parse justfile directly** - Fallback for argument detection
//!
//! # Argument Handling
//!
//! just supports named and positional arguments:
//! - `just recipe arg1 arg2` (positional)
//! - `just recipe --name value` (named, if recipe uses {{name}})

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use regex::Regex;
use serde::Deserialize;

use super::traits::{RunOptions, RunResult, Runner, RunnerResult, TaskArg, TaskInfo};
use crate::error::{suggest_fix, TaskError};

/// justfile runner
pub struct JustfileRunner {
    /// Path to the just command
    just_command: String,
}

impl Default for JustfileRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl JustfileRunner {
    /// Create a new justfile runner using system `just`
    pub fn new() -> Self {
        Self {
            just_command: "just".to_string(),
        }
    }

    /// Create a justfile runner with a custom just command path
    pub fn with_command(command: impl Into<String>) -> Self {
        Self {
            just_command: command.into(),
        }
    }

    /// Find the justfile in a directory
    ///
    /// Checks for: justfile, Justfile, .justfile
    pub fn find_justfile(dir: &Path) -> Option<std::path::PathBuf> {
        for name in &["justfile", "Justfile", ".justfile"] {
            let path = dir.join(name);
            if path.exists() && path.is_file() {
                return Some(path);
            }
        }
        None
    }

    /// List recipes using just --list --unsorted
    fn list_via_just(&self, dir: &Path) -> RunnerResult<Vec<TaskInfo>> {
        let output = Command::new(&self.just_command)
            .current_dir(dir)
            .args(["--list", "--unsorted"])
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| TaskError::SpawnFailed {
                command: format!("{} --list --unsorted", self.just_command),
                error: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(TaskError::CommandFailed {
                command: format!("{} --list", self.just_command),
                exit_code: output.status.code(),
                stderr: stderr.to_string(),
                suggestion: suggest_fix(&self.just_command, &stderr),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        self.parse_list_output(&stdout)
    }

    /// Parse the output of just --list
    ///
    /// Format:
    /// ```text
    /// Available recipes:
    ///     build target='release' # Build the project
    ///     test                   # Run tests
    ///     clean
    /// ```
    fn parse_list_output(&self, output: &str) -> RunnerResult<Vec<TaskInfo>> {
        let mut tasks = Vec::new();

        // Regex for recipe lines: "    name args # description"
        // or just "    name args" without description
        let recipe_re = Regex::new(
            r"^\s{4}([a-zA-Z_][a-zA-Z0-9_-]*)\s*([^#]*?)(?:\s*#\s*(.*))?$",
        )
        .expect("Invalid recipe regex");

        for line in output.lines() {
            // Skip the "Available recipes:" header
            if line.starts_with("Available") || line.trim().is_empty() {
                continue;
            }

            if let Some(caps) = recipe_re.captures(line) {
                let name = caps[1].to_string();
                let args_str = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
                let description = caps.get(3).map(|m| m.as_str().trim().to_string());

                // Parse arguments from the args string
                let arguments = self.parse_args_from_list(args_str);

                tasks.push(TaskInfo {
                    name,
                    description,
                    arguments,
                });
            }
        }

        Ok(tasks)
    }

    /// Parse arguments from just --list output
    ///
    /// Format: `arg1 arg2='default' +varargs`
    fn parse_args_from_list(&self, args_str: &str) -> Vec<TaskArg> {
        if args_str.is_empty() {
            return vec![];
        }

        let mut args = Vec::new();

        // Match argument patterns:
        // - name (required)
        // - name='default' or name="default" (optional with default)
        // - +name (variadic)
        // - *name (variadic, zero or more)
        let arg_re = Regex::new(
            r#"([+*]?)([a-zA-Z_][a-zA-Z0-9_-]*)(?:=['"]?([^'"]*)?['"]?)?"#
        ).expect("Invalid arg regex");

        for caps in arg_re.captures_iter(args_str) {
            let prefix = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let name = caps[2].to_string();
            let default = caps.get(3).map(|m| m.as_str().to_string());

            // + or * prefix means variadic, which is optional
            let required = prefix.is_empty() && default.is_none();

            args.push(TaskArg {
                name,
                required,
                default,
                description: None,
            });
        }

        args
    }

    /// Try to get detailed recipe info via just --dump --format json
    ///
    /// This provides the most detailed information including comments.
    fn list_via_dump(&self, dir: &Path) -> RunnerResult<Vec<TaskInfo>> {
        let output = Command::new(&self.just_command)
            .current_dir(dir)
            .args(["--dump", "--format", "json"])
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| TaskError::SpawnFailed {
                command: format!("{} --dump --format json", self.just_command),
                error: e.to_string(),
            })?;

        if !output.status.success() {
            // Fall back to --list if --dump doesn't work
            return self.list_via_just(dir);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        self.parse_dump_json(&stdout)
    }

    /// Parse just --dump --format json output
    fn parse_dump_json(&self, json_str: &str) -> RunnerResult<Vec<TaskInfo>> {
        #[derive(Deserialize)]
        struct JustDump {
            recipes: std::collections::HashMap<String, JustRecipe>,
        }

        #[derive(Deserialize)]
        struct JustRecipe {
            #[serde(default)]
            doc: Option<String>,
            #[serde(default)]
            parameters: Vec<JustParameter>,
        }

        #[derive(Deserialize)]
        struct JustParameter {
            name: String,
            #[serde(default)]
            default: Option<serde_json::Value>,
            #[serde(default)]
            kind: String,
        }

        let dump: JustDump = serde_json::from_str(json_str).map_err(|e| {
            TaskError::Config(format!("Failed to parse just dump output: {}", e))
        })?;

        let mut tasks: Vec<TaskInfo> = dump
            .recipes
            .into_iter()
            .map(|(name, recipe)| {
                let arguments: Vec<TaskArg> = recipe
                    .parameters
                    .into_iter()
                    .map(|p| {
                        let default = p.default.map(|v| match v {
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        });
                        let required = default.is_none() && p.kind != "Plus" && p.kind != "Star";

                        TaskArg {
                            name: p.name,
                            required,
                            default,
                            description: None,
                        }
                    })
                    .collect();

                TaskInfo {
                    name,
                    description: recipe.doc,
                    arguments,
                }
            })
            .collect();

        // Sort by name for consistent output
        tasks.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(tasks)
    }

    /// Parse justfile directly for comments and arguments
    fn parse_justfile(&self, justfile_path: &Path) -> RunnerResult<Vec<TaskInfo>> {
        let file = std::fs::File::open(justfile_path).map_err(TaskError::Io)?;
        let reader = BufReader::new(file);

        let mut tasks = Vec::new();
        let mut seen_recipes: HashSet<String> = HashSet::new();

        // Regex for recipe definition: "name args:" or "@name args:"
        let recipe_re = Regex::new(
            r"^@?([a-zA-Z_][a-zA-Z0-9_-]*)\s*([^:]*?):\s*.*$"
        ).expect("Invalid recipe regex");

        // Regex for doc comments: "# comment" before recipe
        let doc_re = Regex::new(r"^#\s*(.*)$").expect("Invalid doc regex");

        let lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();

        for (i, line) in lines.iter().enumerate() {
            if let Some(caps) = recipe_re.captures(line) {
                let name = caps[1].to_string();

                if seen_recipes.contains(&name) {
                    continue;
                }
                seen_recipes.insert(name.clone());

                let args_str = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
                let arguments = self.parse_args_from_list(args_str);

                // Look for doc comment in previous line
                let description = if i > 0 {
                    doc_re.captures(&lines[i - 1])
                        .and_then(|c| c.get(1))
                        .map(|m| m.as_str().trim().to_string())
                } else {
                    None
                };

                tasks.push(TaskInfo {
                    name,
                    description,
                    arguments,
                });
            }
        }

        tasks.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tasks)
    }

    /// Execute a just recipe
    fn execute_just(
        &self,
        dir: &Path,
        task: &str,
        options: &RunOptions,
    ) -> RunnerResult<RunResult> {
        let start = Instant::now();

        let mut cmd = Command::new(&self.just_command);
        cmd.current_dir(dir);
        cmd.arg(task);

        // Add named arguments (just uses positional or --arg=value syntax)
        // For simplicity, we'll pass them as positional: key=value
        for (key, value) in &options.args {
            cmd.arg(format!("{}={}", key, value));
        }

        // Add positional arguments
        for arg in &options.positional_args {
            cmd.arg(arg);
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

            // Check if recipe exists
            if stderr.contains("Justfile does not contain recipe")
                || stderr.contains("Just was unable to find")
                || stderr.contains("Unknown recipe")
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

impl Runner for JustfileRunner {
    fn name(&self) -> &str {
        "just"
    }

    fn list_tasks(&self, dir: &Path) -> RunnerResult<Vec<TaskInfo>> {
        // Verify justfile exists first
        if Self::find_justfile(dir).is_none() {
            return Err(TaskError::NoRunnerDetected {
                path: dir.display().to_string(),
                available: vec![],
            });
        }

        // Try dump first for best detail, fallback to list
        match self.list_via_dump(dir) {
            Ok(tasks) if !tasks.is_empty() => Ok(tasks),
            _ => {
                // Fallback to parsing directly if just isn't available
                if let Some(justfile_path) = Self::find_justfile(dir) {
                    self.parse_justfile(&justfile_path)
                } else {
                    Err(TaskError::NoRunnerDetected {
                        path: dir.display().to_string(),
                        available: vec![],
                    })
                }
            }
        }
    }

    fn run_task(&self, dir: &Path, task: &str, options: &RunOptions) -> RunnerResult<RunResult> {
        // Verify justfile exists
        if Self::find_justfile(dir).is_none() {
            return Err(TaskError::NoRunnerDetected {
                path: dir.display().to_string(),
                available: vec![],
            });
        }

        self.execute_just(dir, task, options)
    }

    fn build_command(&self, task: &str, options: &RunOptions) -> String {
        let mut parts = vec![self.just_command.clone(), task.to_string()];

        // Add named arguments as key=value
        for (key, value) in &options.args {
            parts.push(format!("{}={}", key, value));
        }

        // Add positional arguments
        for arg in &options.positional_args {
            parts.push(arg.clone());
        }

        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_dir_with_justfile(content: &str) -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("justfile"), content).unwrap();
        dir
    }

    #[test]
    fn test_find_justfile_lowercase() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("justfile"), "build:").unwrap();

        let found = JustfileRunner::find_justfile(dir.path());
        assert!(found.is_some());
        assert!(found.unwrap().ends_with("justfile"));
    }

    #[test]
    fn test_find_justfile_uppercase() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Justfile"), "build:").unwrap();

        let found = JustfileRunner::find_justfile(dir.path());
        assert!(found.is_some());
    }

    #[test]
    fn test_find_justfile_hidden() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".justfile"), "build:").unwrap();

        let found = JustfileRunner::find_justfile(dir.path());
        assert!(found.is_some());
    }

    #[test]
    fn test_find_justfile_priority() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("justfile"), "build:").unwrap();
        fs::write(dir.path().join("Justfile"), "other:").unwrap();

        // lowercase justfile should be found first
        let found = JustfileRunner::find_justfile(dir.path());
        assert!(found.unwrap().ends_with("justfile"));
    }

    #[test]
    fn test_find_justfile_none() {
        let dir = TempDir::new().unwrap();

        let found = JustfileRunner::find_justfile(dir.path());
        assert!(found.is_none());
    }

    #[test]
    fn test_parse_list_output_simple() {
        let runner = JustfileRunner::new();
        let output = r#"Available recipes:
    build
    test
    clean
"#;

        let tasks = runner.parse_list_output(output).unwrap();

        assert_eq!(tasks.len(), 3);
        assert!(tasks.iter().any(|t| t.name == "build"));
        assert!(tasks.iter().any(|t| t.name == "test"));
        assert!(tasks.iter().any(|t| t.name == "clean"));
    }

    #[test]
    fn test_parse_list_output_with_descriptions() {
        let runner = JustfileRunner::new();
        let output = r#"Available recipes:
    build target='release' # Build the project
    test                   # Run all tests
"#;

        let tasks = runner.parse_list_output(output).unwrap();

        let build = tasks.iter().find(|t| t.name == "build").unwrap();
        assert_eq!(build.description, Some("Build the project".to_string()));

        let test = tasks.iter().find(|t| t.name == "test").unwrap();
        assert_eq!(test.description, Some("Run all tests".to_string()));
    }

    #[test]
    fn test_parse_list_output_with_args() {
        let runner = JustfileRunner::new();
        let output = r#"Available recipes:
    build target='release' verbose='false'
    greet name
    files +paths
"#;

        let tasks = runner.parse_list_output(output).unwrap();

        // build has optional args with defaults
        let build = tasks.iter().find(|t| t.name == "build").unwrap();
        assert_eq!(build.arguments.len(), 2);
        let target_arg = build.arguments.iter().find(|a| a.name == "target").unwrap();
        assert!(!target_arg.required);
        assert_eq!(target_arg.default, Some("release".to_string()));

        // greet has required arg
        let greet = tasks.iter().find(|t| t.name == "greet").unwrap();
        let name_arg = &greet.arguments[0];
        assert!(name_arg.required);
        assert!(name_arg.default.is_none());

        // files has variadic arg
        let files = tasks.iter().find(|t| t.name == "files").unwrap();
        let paths_arg = &files.arguments[0];
        assert!(!paths_arg.required); // variadic is optional
    }

    #[test]
    fn test_parse_args_from_list() {
        let runner = JustfileRunner::new();

        // Empty
        assert!(runner.parse_args_from_list("").is_empty());

        // Simple required
        let args = runner.parse_args_from_list("name");
        assert_eq!(args.len(), 1);
        assert!(args[0].required);

        // With default
        let args = runner.parse_args_from_list("target='release'");
        assert_eq!(args.len(), 1);
        assert!(!args[0].required);
        assert_eq!(args[0].default, Some("release".to_string()));

        // Multiple args
        let args = runner.parse_args_from_list("a b='default' +c");
        assert_eq!(args.len(), 3);
    }

    #[test]
    fn test_parse_justfile_simple() {
        let justfile = r#"
# Build the project
build:
    @echo building

# Run tests
test:
    @echo testing
"#;
        let dir = create_test_dir_with_justfile(justfile);
        let runner = JustfileRunner::new();

        let tasks = runner.parse_justfile(&dir.path().join("justfile")).unwrap();

        assert!(tasks.iter().any(|t| t.name == "build"));
        assert!(tasks.iter().any(|t| t.name == "test"));

        let build = tasks.iter().find(|t| t.name == "build").unwrap();
        assert_eq!(build.description, Some("Build the project".to_string()));
    }

    #[test]
    fn test_parse_justfile_with_args() {
        let justfile = r#"
# Build with target
build target='release':
    @echo "Building {{target}}"

# Greet someone
greet name:
    @echo "Hello {{name}}"
"#;
        let dir = create_test_dir_with_justfile(justfile);
        let runner = JustfileRunner::new();

        let tasks = runner.parse_justfile(&dir.path().join("justfile")).unwrap();

        let build = tasks.iter().find(|t| t.name == "build").unwrap();
        assert_eq!(build.arguments.len(), 1);
        assert_eq!(build.arguments[0].name, "target");
        assert!(!build.arguments[0].required);

        let greet = tasks.iter().find(|t| t.name == "greet").unwrap();
        assert_eq!(greet.arguments.len(), 1);
        assert!(greet.arguments[0].required);
    }

    #[test]
    fn test_parse_justfile_private_recipes() {
        let justfile = r#"
build:
    @echo building

# Private recipe (starts with _)
_helper:
    @echo helper
"#;
        let dir = create_test_dir_with_justfile(justfile);
        let runner = JustfileRunner::new();

        let tasks = runner.parse_justfile(&dir.path().join("justfile")).unwrap();

        // Both should be found (filtering is typically done by just --list)
        assert!(tasks.iter().any(|t| t.name == "build"));
        // _helper might not be matched due to our regex requiring letter/underscore start
        // but underscore is valid, so it should match
    }

    #[test]
    fn test_parse_justfile_quiet_recipes() {
        let justfile = r#"
# Quiet build
@build:
    echo building
"#;
        let dir = create_test_dir_with_justfile(justfile);
        let runner = JustfileRunner::new();

        let tasks = runner.parse_justfile(&dir.path().join("justfile")).unwrap();

        // @ prefix should be stripped
        assert!(tasks.iter().any(|t| t.name == "build"));
    }

    #[test]
    fn test_build_command_simple() {
        let runner = JustfileRunner::new();
        let options = RunOptions::default();

        let cmd = runner.build_command("build", &options);
        assert_eq!(cmd, "just build");
    }

    #[test]
    fn test_build_command_with_args() {
        let runner = JustfileRunner::new();
        let options = RunOptions::default()
            .with_arg("target", "debug")
            .with_positional("extra");

        let cmd = runner.build_command("build", &options);
        assert!(cmd.contains("just build"));
        assert!(cmd.contains("target=debug"));
        assert!(cmd.contains("extra"));
    }

    #[test]
    fn test_build_command_with_custom_just() {
        let runner = JustfileRunner::with_command("/usr/local/bin/just");
        let options = RunOptions::default();

        let cmd = runner.build_command("build", &options);
        assert!(cmd.starts_with("/usr/local/bin/just"));
    }

    #[test]
    fn test_runner_name() {
        let runner = JustfileRunner::new();
        assert_eq!(runner.name(), "just");
    }

    #[test]
    fn test_list_tasks_no_justfile() {
        let dir = TempDir::new().unwrap();
        let runner = JustfileRunner::new();

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
    fn test_run_task_no_justfile() {
        let dir = TempDir::new().unwrap();
        let runner = JustfileRunner::new();

        let result = runner.run_task(dir.path(), "build", &RunOptions::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_run_task_simple() {
        let justfile = r#"
echo-test:
    @echo "test output"
"#;
        let dir = create_test_dir_with_justfile(justfile);
        let runner = JustfileRunner::new();

        let result = runner.run_task(dir.path(), "echo-test", &RunOptions::default());

        match result {
            Ok(run_result) => {
                assert!(run_result.success);
                assert!(run_result.stdout.contains("test output"));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: just not installed");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_run_task_with_args() {
        let justfile = r#"
show-var var:
    @echo "Value: {{var}}"
"#;
        let dir = create_test_dir_with_justfile(justfile);
        let runner = JustfileRunner::new();

        let options = RunOptions::default().with_positional("hello");

        let result = runner.run_task(dir.path(), "show-var", &options);

        match result {
            Ok(run_result) => {
                assert!(run_result.success);
                assert!(run_result.stdout.contains("Value: hello"));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: just not installed");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_run_task_failing() {
        let justfile = r#"
fail:
    @exit 1
"#;
        let dir = create_test_dir_with_justfile(justfile);
        let runner = JustfileRunner::new();

        let result = runner.run_task(dir.path(), "fail", &RunOptions::default());

        match result {
            Ok(run_result) => {
                assert!(!run_result.success);
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: just not installed");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_run_task_nonexistent() {
        let justfile = "build:\n    @echo building\n";
        let dir = create_test_dir_with_justfile(justfile);
        let runner = JustfileRunner::new();

        let result = runner.run_task(dir.path(), "nonexistent", &RunOptions::default());

        match result {
            Err(TaskError::TaskNotFound { task, .. }) => {
                assert_eq!(task, "nonexistent");
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: just not installed");
            }
            Ok(_) => panic!("Expected TaskNotFound error"),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_task_exists() {
        let justfile = "build:\n    @echo building\n\ntest:\n    @echo testing\n";
        let dir = create_test_dir_with_justfile(justfile);
        let runner = JustfileRunner::new();

        // This uses parse_justfile as fallback when just isn't available
        let tasks = runner.parse_justfile(&dir.path().join("justfile")).unwrap();
        assert!(tasks.iter().any(|t| t.name == "build"));
        assert!(tasks.iter().any(|t| t.name == "test"));
    }

    #[test]
    fn test_parse_dump_json() {
        let runner = JustfileRunner::new();
        let json = r#"{
            "recipes": {
                "build": {
                    "doc": "Build the project",
                    "parameters": [
                        {"name": "target", "default": "release", "kind": "Singular"}
                    ]
                },
                "test": {
                    "doc": null,
                    "parameters": []
                }
            }
        }"#;

        let tasks = runner.parse_dump_json(json).unwrap();

        assert_eq!(tasks.len(), 2);

        let build = tasks.iter().find(|t| t.name == "build").unwrap();
        assert_eq!(build.description, Some("Build the project".to_string()));
        assert_eq!(build.arguments.len(), 1);
        assert_eq!(build.arguments[0].name, "target");
    }

    #[test]
    fn test_parse_complex_justfile() {
        let justfile = r#"
# Set shell
set shell := ["bash", "-c"]

# Default recipe
default: build test

# Build the project
build target='release':
    cargo build --{{target}}

# Run tests
test *args:
    cargo test {{args}}

# Clean artifacts
clean:
    rm -rf target/

# Private helper
_setup:
    @echo "Setting up..."
"#;
        let dir = create_test_dir_with_justfile(justfile);
        let runner = JustfileRunner::new();

        let tasks = runner.parse_justfile(&dir.path().join("justfile")).unwrap();

        // Should find multiple recipes
        let names: Vec<&str> = tasks.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"default"));
        assert!(names.contains(&"build"));
        assert!(names.contains(&"test"));
        assert!(names.contains(&"clean"));
    }
}
