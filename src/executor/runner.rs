//! Async command execution with timeout support
//!
//! Provides a unified interface for running commands with:
//! - Configurable timeouts
//! - Output capture (stdout/stderr)
//! - Output truncation for large outputs
//! - Environment variable injection
//! - Working directory control

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

use crate::error::{suggest_fix, TaskError};
use crate::runner::RunResult;

/// Maximum output size before truncation (in bytes)
const MAX_OUTPUT_SIZE: usize = 100_000; // 100KB

/// Truncation marker for large outputs
const TRUNCATION_MARKER: &str = "\n... [output truncated] ...\n";

/// Options for async command execution
#[derive(Debug, Clone)]
pub struct ExecOptions {
    /// Working directory for the command
    pub working_dir: Option<std::path::PathBuf>,
    /// Environment variables to set
    pub env: HashMap<String, String>,
    /// Timeout duration (None = no timeout)
    pub timeout: Option<Duration>,
    /// Whether to capture output (vs streaming)
    pub capture_output: bool,
    /// Maximum output size before truncation
    pub max_output_size: usize,
}

impl Default for ExecOptions {
    fn default() -> Self {
        Self {
            working_dir: None,
            env: HashMap::new(),
            timeout: None,
            capture_output: true,
            max_output_size: MAX_OUTPUT_SIZE,
        }
    }
}

impl ExecOptions {
    /// Create options with a working directory
    pub fn in_dir(dir: impl Into<std::path::PathBuf>) -> Self {
        Self {
            working_dir: Some(dir.into()),
            ..Default::default()
        }
    }

    /// Set the timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set timeout in seconds
    pub fn with_timeout_secs(self, secs: u64) -> Self {
        self.with_timeout(Duration::from_secs(secs))
    }

    /// Add an environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set maximum output size
    pub fn with_max_output(mut self, size: usize) -> Self {
        self.max_output_size = size;
        self
    }
}

/// Result of async command execution
#[derive(Debug)]
pub struct ExecResult {
    /// Whether the command succeeded (exit code 0)
    pub success: bool,
    /// Exit code if available
    pub exit_code: Option<i32>,
    /// Standard output (may be truncated)
    pub stdout: String,
    /// Whether stdout was truncated
    pub stdout_truncated: bool,
    /// Standard error
    pub stderr: String,
    /// Whether stderr was truncated
    pub stderr_truncated: bool,
    /// Duration of execution
    pub duration: Duration,
    /// Whether the command timed out
    pub timed_out: bool,
}

impl ExecResult {
    /// Convert to a RunResult
    pub fn to_run_result(self, command: impl Into<String>) -> RunResult {
        if self.success {
            RunResult::success(command, self.stdout, self.duration.as_millis() as u64)
        } else {
            RunResult::failed(
                command,
                self.exit_code,
                self.stdout,
                self.stderr,
                self.duration.as_millis() as u64,
            )
        }
    }
}

/// Execute a command asynchronously with timeout support
///
/// # Arguments
/// * `program` - The program to execute
/// * `args` - Command arguments
/// * `options` - Execution options
///
/// # Returns
/// * `Result<ExecResult, TaskError>` - Execution result or error
///
/// # Errors
/// * `TaskError::SpawnFailed` - If the command couldn't be spawned
/// * `TaskError::Timeout` - If the command timed out (when timeout is set)
pub async fn exec_command(
    program: &str,
    args: &[&str],
    options: &ExecOptions,
) -> Result<ExecResult, TaskError> {
    let start = Instant::now();
    let command_str = format!("{} {}", program, args.join(" "));

    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true); // Kill process if future is dropped

    // Set working directory
    if let Some(ref dir) = options.working_dir {
        cmd.current_dir(dir);
    }

    // Set environment variables
    for (key, value) in &options.env {
        cmd.env(key, value);
    }

    tracing::debug!("Executing async: {}", command_str);

    let child = cmd.spawn().map_err(|e| TaskError::SpawnFailed {
        command: command_str.clone(),
        error: e.to_string(),
    })?;

    // Execute with or without timeout
    let result = if let Some(timeout_duration) = options.timeout {
        match timeout(timeout_duration, wait_for_output(child, options.max_output_size)).await {
            Ok(result) => result?,
            Err(_) => {
                // Timeout occurred
                return Err(TaskError::Timeout {
                    command: command_str,
                    timeout_secs: timeout_duration.as_secs(),
                });
            }
        }
    } else {
        wait_for_output(child, options.max_output_size).await?
    };

    let duration = start.elapsed();

    Ok(ExecResult {
        success: result.exit_code == Some(0),
        exit_code: result.exit_code,
        stdout: result.stdout,
        stdout_truncated: result.stdout_truncated,
        stderr: result.stderr,
        stderr_truncated: result.stderr_truncated,
        duration,
        timed_out: false,
    })
}

/// Internal result from waiting for process output
struct WaitResult {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

/// Wait for a child process and capture its output
async fn wait_for_output(
    mut child: tokio::process::Child,
    max_output_size: usize,
) -> Result<WaitResult, TaskError> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Read stdout and stderr concurrently
    let stdout_handle = tokio::spawn(async move {
        if let Some(stdout) = stdout {
            read_and_truncate(stdout, max_output_size).await
        } else {
            (String::new(), false)
        }
    });

    let stderr_handle = tokio::spawn(async move {
        if let Some(stderr) = stderr {
            read_and_truncate(stderr, max_output_size).await
        } else {
            (String::new(), false)
        }
    });

    // Wait for process to complete
    let status = child
        .wait()
        .await
        .map_err(|e| TaskError::Io(e))?;

    // Get output results
    let (stdout, stdout_truncated) = stdout_handle
        .await
        .map_err(|e| TaskError::Io(std::io::Error::other(format!("stdout task failed: {}", e))))?;

    let (stderr, stderr_truncated) = stderr_handle
        .await
        .map_err(|e| TaskError::Io(std::io::Error::other(format!("stderr task failed: {}", e))))?;

    Ok(WaitResult {
        exit_code: status.code(),
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
}

/// Read from an async reader and truncate if too large
///
/// Optimized for memory efficiency:
/// - Pre-allocates output buffer to avoid repeated reallocations
/// - Reuses line buffer across iterations instead of allocating new String each time
async fn read_and_truncate<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    max_size: usize,
) -> (String, bool) {
    let mut buf_reader = BufReader::new(reader);
    // Pre-allocate output buffer (cap at 64KB to avoid over-allocation for small max_size)
    let mut output = String::with_capacity(max_size.min(64 * 1024));
    // Reuse line buffer across iterations (typical line is ~80 chars, allow some margin)
    let mut line = String::with_capacity(4096);
    let mut truncated = false;

    loop {
        line.clear(); // Reuse buffer instead of allocating new String
        match buf_reader.read_line(&mut line).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                if output.len() + line.len() > max_size {
                    // Truncate
                    let remaining = max_size.saturating_sub(output.len());
                    if remaining > 0 {
                        output.push_str(&line[..remaining.min(line.len())]);
                    }
                    output.push_str(TRUNCATION_MARKER);
                    truncated = true;
                    break;
                }
                output.push_str(&line);
            }
            Err(e) => {
                tracing::warn!("Error reading output: {}", e);
                break;
            }
        }
    }

    (output, truncated)
}

/// Execute a command synchronously (convenience wrapper for sync contexts)
///
/// This is a blocking wrapper around `exec_command` for use in non-async code.
pub fn exec_command_sync(
    program: &str,
    args: &[&str],
    options: &ExecOptions,
) -> Result<ExecResult, TaskError> {
    // Create a new runtime for the sync call
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| TaskError::Io(std::io::Error::other(format!("Failed to create runtime: {}", e))))?;

    rt.block_on(exec_command(program, args, options))
}

/// Execute a shell command with proper quoting
///
/// # Arguments
/// * `shell` - Shell to use (e.g., "bash", "sh")
/// * `command` - Command string to execute
/// * `options` - Execution options
pub async fn exec_shell_command(
    shell: &str,
    command: &str,
    options: &ExecOptions,
) -> Result<ExecResult, TaskError> {
    exec_command(shell, &["-c", command], options).await
}

/// High-level task executor that integrates with runners
pub struct TaskExecutor {
    /// Default timeout for commands
    default_timeout: Option<Duration>,
    /// Default working directory
    working_dir: Option<std::path::PathBuf>,
    /// Default environment variables
    env: HashMap<String, String>,
}

impl Default for TaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskExecutor {
    /// Create a new task executor
    pub fn new() -> Self {
        Self {
            default_timeout: None,
            working_dir: None,
            env: HashMap::new(),
        }
    }

    /// Set default timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = Some(timeout);
        self
    }

    /// Set default working directory
    pub fn with_working_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Add a default environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Build execution options with defaults
    fn build_options(&self, overrides: &ExecOptions) -> ExecOptions {
        let mut options = ExecOptions {
            working_dir: self.working_dir.clone(),
            env: self.env.clone(),
            timeout: self.default_timeout,
            ..Default::default()
        };

        // Apply overrides
        if overrides.working_dir.is_some() {
            options.working_dir = overrides.working_dir.clone();
        }
        if overrides.timeout.is_some() {
            options.timeout = overrides.timeout;
        }
        for (k, v) in &overrides.env {
            options.env.insert(k.clone(), v.clone());
        }
        if overrides.max_output_size != MAX_OUTPUT_SIZE {
            options.max_output_size = overrides.max_output_size;
        }

        options
    }

    /// Execute a command
    pub async fn execute(
        &self,
        program: &str,
        args: &[&str],
        options: &ExecOptions,
    ) -> Result<ExecResult, TaskError> {
        let merged_options = self.build_options(options);
        exec_command(program, args, &merged_options).await
    }

    /// Execute using a runner's task
    pub async fn run_task<R: crate::runner::Runner + ?Sized>(
        &self,
        runner: &R,
        dir: &Path,
        task: &str,
        options: &crate::runner::RunOptions,
    ) -> Result<RunResult, TaskError> {
        // For now, delegate to the runner's synchronous implementation
        // In the future, we can make runners fully async
        runner.run_task(dir, task, options)
    }
}

/// Helper to create an error with suggestion
pub fn command_error(
    command: &str,
    exit_code: Option<i32>,
    stderr: &str,
) -> TaskError {
    TaskError::CommandFailed {
        command: command.to_string(),
        exit_code,
        stderr: stderr.to_string(),
        suggestion: suggest_fix(command, stderr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exec_options_default() {
        let options = ExecOptions::default();

        assert!(options.working_dir.is_none());
        assert!(options.env.is_empty());
        assert!(options.timeout.is_none());
        assert!(options.capture_output);
        assert_eq!(options.max_output_size, MAX_OUTPUT_SIZE);
    }

    #[test]
    fn test_exec_options_builder() {
        let options = ExecOptions::in_dir("/tmp")
            .with_timeout_secs(60)
            .with_env("KEY", "value")
            .with_max_output(1000);

        assert_eq!(
            options.working_dir,
            Some(std::path::PathBuf::from("/tmp"))
        );
        assert_eq!(options.timeout, Some(Duration::from_secs(60)));
        assert_eq!(options.env.get("KEY"), Some(&"value".to_string()));
        assert_eq!(options.max_output_size, 1000);
    }

    #[tokio::test]
    async fn test_exec_command_success() {
        let result = exec_command("echo", &["hello world"], &ExecOptions::default()).await;

        match result {
            Ok(res) => {
                assert!(res.success);
                assert_eq!(res.exit_code, Some(0));
                assert!(res.stdout.contains("hello world"));
                assert!(!res.timed_out);
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: echo not available");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_exec_command_failure() {
        let result = exec_command("false", &[], &ExecOptions::default()).await;

        match result {
            Ok(res) => {
                assert!(!res.success);
                assert_ne!(res.exit_code, Some(0));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: false not available");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_exec_command_with_env() {
        let options = ExecOptions::default().with_env("MY_VAR", "test_value");

        let result = exec_command("sh", &["-c", "echo $MY_VAR"], &options).await;

        match result {
            Ok(res) => {
                assert!(res.success);
                assert!(res.stdout.contains("test_value"));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: sh not available");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_exec_command_timeout() {
        let options = ExecOptions::default()
            .with_timeout(Duration::from_millis(100));

        let result = exec_command("sleep", &["10"], &options).await;

        match result {
            Err(TaskError::Timeout { timeout_secs, .. }) => {
                // Timeout should be 0 since we used milliseconds
                assert!(timeout_secs <= 1);
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: sleep not available");
            }
            Ok(_) => panic!("Expected timeout error"),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_exec_command_output_truncation() {
        // Generate output larger than max
        let options = ExecOptions::default().with_max_output(100);

        let result = exec_command(
            "sh",
            &["-c", "for i in $(seq 1 100); do echo 'line of output $i'; done"],
            &options,
        )
        .await;

        match result {
            Ok(res) => {
                assert!(res.stdout_truncated);
                assert!(res.stdout.contains("[output truncated]"));
                // Output should be roughly at max size
                assert!(res.stdout.len() <= 200); // Some margin for truncation marker
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: sh not available");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_exec_command_working_dir() {
        let options = ExecOptions::in_dir("/tmp");

        let result = exec_command("pwd", &[], &options).await;

        match result {
            Ok(res) => {
                assert!(res.success);
                assert!(res.stdout.trim() == "/tmp" || res.stdout.contains("/tmp"));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: pwd not available");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_exec_command_spawn_failed() {
        let result = exec_command(
            "nonexistent_command_12345",
            &[],
            &ExecOptions::default(),
        )
        .await;

        match result {
            Err(TaskError::SpawnFailed { command, .. }) => {
                assert!(command.contains("nonexistent_command_12345"));
            }
            _ => panic!("Expected SpawnFailed error"),
        }
    }

    #[tokio::test]
    async fn test_exec_shell_command() {
        let result = exec_shell_command(
            "sh",
            "echo 'hello' && echo 'world'",
            &ExecOptions::default(),
        )
        .await;

        match result {
            Ok(res) => {
                assert!(res.success);
                assert!(res.stdout.contains("hello"));
                assert!(res.stdout.contains("world"));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: sh not available");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_exec_command_sync() {
        let result = exec_command_sync("echo", &["sync test"], &ExecOptions::default());

        match result {
            Ok(res) => {
                assert!(res.success);
                assert!(res.stdout.contains("sync test"));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: echo not available");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_task_executor_defaults() {
        let executor = TaskExecutor::new()
            .with_timeout(Duration::from_secs(30))
            .with_working_dir("/tmp")
            .with_env("DEFAULT_VAR", "default_value");

        assert_eq!(executor.default_timeout, Some(Duration::from_secs(30)));
        assert_eq!(executor.working_dir, Some(std::path::PathBuf::from("/tmp")));
        assert_eq!(executor.env.get("DEFAULT_VAR"), Some(&"default_value".to_string()));
    }

    #[test]
    fn test_task_executor_build_options() {
        let executor = TaskExecutor::new()
            .with_timeout(Duration::from_secs(30))
            .with_env("DEFAULT", "1");

        let overrides = ExecOptions::default()
            .with_timeout(Duration::from_secs(60))
            .with_env("OVERRIDE", "2");

        let merged = executor.build_options(&overrides);

        // Override should win for timeout
        assert_eq!(merged.timeout, Some(Duration::from_secs(60)));
        // Both env vars should be present
        assert_eq!(merged.env.get("DEFAULT"), Some(&"1".to_string()));
        assert_eq!(merged.env.get("OVERRIDE"), Some(&"2".to_string()));
    }

    #[test]
    fn test_exec_result_to_run_result_success() {
        let exec_result = ExecResult {
            success: true,
            exit_code: Some(0),
            stdout: "output".to_string(),
            stdout_truncated: false,
            stderr: String::new(),
            stderr_truncated: false,
            duration: Duration::from_millis(100),
            timed_out: false,
        };

        let run_result = exec_result.to_run_result("test command");

        assert!(run_result.success);
        assert_eq!(run_result.exit_code, Some(0));
        assert_eq!(run_result.stdout, "output");
        assert_eq!(run_result.duration_ms, 100);
    }

    #[test]
    fn test_exec_result_to_run_result_failure() {
        let exec_result = ExecResult {
            success: false,
            exit_code: Some(1),
            stdout: "out".to_string(),
            stdout_truncated: false,
            stderr: "error".to_string(),
            stderr_truncated: false,
            duration: Duration::from_millis(50),
            timed_out: false,
        };

        let run_result = exec_result.to_run_result("failing command");

        assert!(!run_result.success);
        assert_eq!(run_result.exit_code, Some(1));
        assert_eq!(run_result.stderr, "error");
    }

    #[test]
    fn test_command_error() {
        let err = command_error("make build", Some(2), "No rule to make target");

        match err {
            TaskError::CommandFailed { command, exit_code, stderr, suggestion } => {
                assert_eq!(command, "make build");
                assert_eq!(exit_code, Some(2));
                assert!(stderr.contains("No rule"));
                // Should have a suggestion for make errors
                assert!(suggestion.is_some());
            }
            _ => panic!("Expected CommandFailed error"),
        }
    }

    // TDD: Tests for memory allocation optimization (Step 4 of v0.1.0 cleanup)
    #[tokio::test]
    async fn test_read_and_truncate_large_output() {
        // Test that large output is properly truncated
        let options = ExecOptions::default().with_max_output(500);

        // Generate 1000 lines of output - should be truncated
        let result = exec_command(
            "sh",
            &["-c", "for i in $(seq 1 1000); do echo 'line $i'; done"],
            &options,
        )
        .await;

        match result {
            Ok(res) => {
                assert!(res.stdout_truncated, "Output should be truncated");
                assert!(res.stdout.contains("[output truncated]"));
                // Should be close to max_size
                assert!(
                    res.stdout.len() <= 600,
                    "Output too large: {} bytes",
                    res.stdout.len()
                );
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: sh not available");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[tokio::test]
    async fn test_read_and_truncate_small_output() {
        // Small output should not be truncated
        let options = ExecOptions::default().with_max_output(10000);

        let result = exec_command("echo", &["hello world"], &options).await;

        match result {
            Ok(res) => {
                assert!(!res.stdout_truncated, "Small output should not be truncated");
                assert!(!res.stdout.contains("[output truncated]"));
                assert!(res.stdout.contains("hello world"));
            }
            Err(TaskError::SpawnFailed { .. }) => {
                eprintln!("Skipping test: echo not available");
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }
}
