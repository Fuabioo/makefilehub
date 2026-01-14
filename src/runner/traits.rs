//! Common traits and types for build system runners
//!
//! Defines the interface that all runners (make, just, script) must implement.

use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use crate::error::TaskError;

/// Result type for runner operations
pub type RunnerResult<T> = Result<T, TaskError>;

/// Information about a task/target argument
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TaskArg {
    /// Argument name
    pub name: String,
    /// Whether the argument is required
    pub required: bool,
    /// Default value if any
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Description of the argument
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Information about a task/target
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TaskInfo {
    /// Task name
    pub name: String,
    /// Description if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Arguments for this task
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<TaskArg>,
}

impl TaskInfo {
    /// Create a new task with just a name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            arguments: vec![],
        }
    }

    /// Add a description to the task
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Add an argument to the task
    pub fn with_arg(mut self, arg: TaskArg) -> Self {
        self.arguments.push(arg);
        self
    }
}

/// Options for running a task
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Working directory (defaults to current directory)
    pub working_dir: Option<std::path::PathBuf>,
    /// Named arguments (key=value pairs)
    pub args: HashMap<String, String>,
    /// Positional arguments
    pub positional_args: Vec<String>,
    /// Environment variables to set
    pub env: HashMap<String, String>,
    /// Timeout for the command
    pub timeout: Option<Duration>,
    /// Capture output instead of streaming
    pub capture_output: bool,
}

impl RunOptions {
    /// Create new run options with a working directory
    pub fn in_dir(dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            working_dir: Some(dir.into()),
            ..Default::default()
        }
    }

    /// Add a named argument
    pub fn with_arg(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.args.insert(key.into(), value.into());
        self
    }

    /// Add a positional argument
    pub fn with_positional(mut self, arg: impl Into<String>) -> Self {
        self.positional_args.push(arg.into());
        self
    }

    /// Set timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }
}

/// Result of running a task
#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    /// Whether the task succeeded (exit code 0)
    pub success: bool,
    /// Exit code if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Standard output (may be truncated)
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Command that was executed
    pub command: String,
    /// Duration in milliseconds
    pub duration_ms: u64,
}

impl RunResult {
    /// Create a successful result
    pub fn success(
        command: impl Into<String>,
        stdout: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self {
            success: true,
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
            command: command.into(),
            duration_ms,
        }
    }

    /// Create a failed result
    pub fn failed(
        command: impl Into<String>,
        exit_code: Option<i32>,
        stdout: impl Into<String>,
        stderr: impl Into<String>,
        duration_ms: u64,
    ) -> Self {
        Self {
            success: false,
            exit_code,
            stdout: stdout.into(),
            stderr: stderr.into(),
            command: command.into(),
            duration_ms,
        }
    }
}

/// Trait for build system runners
///
/// Each runner (make, just, script) implements this trait to provide
/// a unified interface for listing and running tasks.
pub trait Runner: Send + Sync {
    /// Get the name of this runner (e.g., "make", "just")
    fn name(&self) -> &str;

    /// List available tasks in the given directory
    ///
    /// # Arguments
    /// * `dir` - Directory containing the build file
    ///
    /// # Returns
    /// * `RunnerResult<Vec<TaskInfo>>` - List of available tasks
    ///
    /// # Errors
    /// * `TaskError::Io` - If reading the build file fails
    /// * `TaskError::CommandFailed` - If listing command fails
    fn list_tasks(&self, dir: &Path) -> RunnerResult<Vec<TaskInfo>>;

    /// Run a task
    ///
    /// # Arguments
    /// * `dir` - Directory containing the build file
    /// * `task` - Name of the task to run
    /// * `options` - Run options (args, env, timeout)
    ///
    /// # Returns
    /// * `RunnerResult<RunResult>` - Result of running the task
    ///
    /// # Errors
    /// * `TaskError::TaskNotFound` - If the task doesn't exist
    /// * `TaskError::CommandFailed` - If the command fails
    /// * `TaskError::Timeout` - If the command times out
    fn run_task(&self, dir: &Path, task: &str, options: &RunOptions) -> RunnerResult<RunResult>;

    /// Build the command line for a task (for display/logging)
    ///
    /// # Arguments
    /// * `task` - Task name
    /// * `options` - Run options
    ///
    /// # Returns
    /// * Full command string that would be executed
    fn build_command(&self, task: &str, options: &RunOptions) -> String;

    /// Check if a task exists
    ///
    /// # Arguments
    /// * `dir` - Directory containing the build file
    /// * `task` - Name of the task to check
    ///
    /// # Returns
    /// * `RunnerResult<bool>` - Whether the task exists
    fn task_exists(&self, dir: &Path, task: &str) -> RunnerResult<bool> {
        let tasks = self.list_tasks(dir)?;
        Ok(tasks.iter().any(|t| t.name == task))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_info_builder() {
        let task = TaskInfo::new("build")
            .with_description("Build the project")
            .with_arg(TaskArg {
                name: "target".to_string(),
                required: false,
                default: Some("release".to_string()),
                description: Some("Build target".to_string()),
            });

        assert_eq!(task.name, "build");
        assert_eq!(task.description, Some("Build the project".to_string()));
        assert_eq!(task.arguments.len(), 1);
        assert_eq!(task.arguments[0].name, "target");
    }

    #[test]
    fn test_task_arg_required() {
        let arg = TaskArg {
            name: "config".to_string(),
            required: true,
            default: None,
            description: Some("Config file path".to_string()),
        };

        assert!(arg.required);
        assert!(arg.default.is_none());
    }

    #[test]
    fn test_run_options_builder() {
        let options = RunOptions::in_dir("/projects/myapp")
            .with_arg("TARGET", "debug")
            .with_arg("VERBOSE", "1")
            .with_positional("extra")
            .with_env("RUST_LOG", "debug")
            .with_timeout(Duration::from_secs(60));

        assert_eq!(
            options.working_dir,
            Some(std::path::PathBuf::from("/projects/myapp"))
        );
        assert_eq!(options.args.get("TARGET"), Some(&"debug".to_string()));
        assert_eq!(options.args.get("VERBOSE"), Some(&"1".to_string()));
        assert_eq!(options.positional_args, vec!["extra"]);
        assert_eq!(options.env.get("RUST_LOG"), Some(&"debug".to_string()));
        assert_eq!(options.timeout, Some(Duration::from_secs(60)));
    }

    #[test]
    fn test_run_result_success() {
        let result = RunResult::success("make build", "Build successful", 1234);

        assert!(result.success);
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout, "Build successful");
        assert!(result.stderr.is_empty());
        assert_eq!(result.command, "make build");
        assert_eq!(result.duration_ms, 1234);
    }

    #[test]
    fn test_run_result_failed() {
        let result = RunResult::failed(
            "make test",
            Some(1),
            "Running tests...",
            "Test failed: assertion error",
            5678,
        );

        assert!(!result.success);
        assert_eq!(result.exit_code, Some(1));
        assert_eq!(result.stdout, "Running tests...");
        assert_eq!(result.stderr, "Test failed: assertion error");
    }

    #[test]
    fn test_task_info_serialization() {
        let task = TaskInfo::new("test").with_description("Run tests");

        let json = serde_json::to_string(&task).unwrap();
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"description\":\"Run tests\""));
        // arguments should be skipped since it's empty
        assert!(!json.contains("\"arguments\""));
    }

    #[test]
    fn test_run_result_serialization() {
        let result = RunResult::success("make build", "ok", 100);

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"exit_code\":0"));
        assert!(json.contains("\"command\":\"make build\""));
    }

    #[test]
    fn test_run_options_default() {
        let options = RunOptions::default();

        assert!(options.working_dir.is_none());
        assert!(options.args.is_empty());
        assert!(options.positional_args.is_empty());
        assert!(options.env.is_empty());
        assert!(options.timeout.is_none());
        assert!(!options.capture_output);
    }
}
