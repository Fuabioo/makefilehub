//! Error types for makefilehub
//!
//! Provides structured error types with suggestions for common issues.

use serde::Serialize;
use thiserror::Error;

/// Main error type for task operations
#[derive(Error, Debug)]
pub enum TaskError {
    /// Project directory not found
    #[error("Project not found: {path}")]
    ProjectNotFound {
        path: String,
        suggestion: Option<String>,
    },

    /// No build system detected in the project
    #[error("No build system detected in {path}")]
    NoRunnerDetected {
        path: String,
        available: Vec<String>,
    },

    /// Requested task/target not found
    #[error("Task '{task}' not found")]
    TaskNotFound {
        task: String,
        available: Vec<String>,
        suggestion: Option<String>,
    },

    /// Command execution failed
    #[error("Command failed: {command}")]
    CommandFailed {
        command: String,
        exit_code: Option<i32>,
        stderr: String,
        suggestion: Option<String>,
    },

    /// Failed to spawn the command
    #[error("Failed to spawn command: {command}")]
    SpawnFailed { command: String, error: String },

    /// Command timed out
    #[error("Command timed out after {timeout_secs}s: {command}")]
    Timeout { command: String, timeout_secs: u64 },

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Service not found in configuration
    #[error("Service not found: {0}")]
    ServiceNotFound(String),

    /// Security violation - path outside allowed directories
    #[error("Security violation: {message}")]
    SecurityViolation { message: String, path: String },

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Serializable error info for MCP responses
#[derive(Debug, Serialize, Clone)]
pub struct ErrorInfo {
    pub message: String,
    pub error_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub available: Vec<String>,
}

impl From<&TaskError> for ErrorInfo {
    fn from(err: &TaskError) -> Self {
        match err {
            TaskError::ProjectNotFound { path, suggestion } => ErrorInfo {
                message: format!("Project not found: {}", path),
                error_type: "project_not_found".to_string(),
                suggestion: suggestion.clone(),
                exit_code: None,
                stderr: None,
                available: vec![],
            },
            TaskError::NoRunnerDetected { path, available } => ErrorInfo {
                message: format!("No build system detected in {}", path),
                error_type: "no_runner_detected".to_string(),
                suggestion: Some("Add a Makefile, justfile, or run.sh to the project".to_string()),
                exit_code: None,
                stderr: None,
                available: available.clone(),
            },
            TaskError::TaskNotFound {
                task,
                available,
                suggestion,
            } => ErrorInfo {
                message: format!("Task '{}' not found", task),
                error_type: "task_not_found".to_string(),
                suggestion: suggestion.clone(),
                exit_code: None,
                stderr: None,
                available: available.clone(),
            },
            TaskError::CommandFailed {
                command,
                exit_code,
                stderr,
                suggestion,
            } => ErrorInfo {
                message: format!("Command failed: {}", command),
                error_type: "command_failed".to_string(),
                suggestion: suggestion.clone(),
                exit_code: *exit_code,
                stderr: Some(stderr.clone()),
                available: vec![],
            },
            TaskError::SpawnFailed { command, error } => ErrorInfo {
                message: format!("Failed to spawn command: {}", command),
                error_type: "spawn_failed".to_string(),
                suggestion: Some(format!("Check if the command exists: {}", error)),
                exit_code: None,
                stderr: None,
                available: vec![],
            },
            TaskError::Timeout {
                command,
                timeout_secs,
            } => ErrorInfo {
                message: format!("Command timed out after {}s: {}", timeout_secs, command),
                error_type: "timeout".to_string(),
                suggestion: Some("Try increasing the timeout or checking if the command hangs".to_string()),
                exit_code: None,
                stderr: None,
                available: vec![],
            },
            TaskError::Config(msg) => ErrorInfo {
                message: format!("Configuration error: {}", msg),
                error_type: "config_error".to_string(),
                suggestion: Some("Check your makefilehub configuration file".to_string()),
                exit_code: None,
                stderr: None,
                available: vec![],
            },
            TaskError::ServiceNotFound(name) => ErrorInfo {
                message: format!("Service not found: {}", name),
                error_type: "service_not_found".to_string(),
                suggestion: Some("Check [services] section in your configuration".to_string()),
                exit_code: None,
                stderr: None,
                available: vec![],
            },
            TaskError::SecurityViolation { message, path } => ErrorInfo {
                message: format!("Security violation: {}", message),
                error_type: "security_violation".to_string(),
                suggestion: Some(format!(
                    "Path '{}' is not in allowed directories. Configure [security].allowed_paths in your config.",
                    path
                )),
                exit_code: None,
                stderr: None,
                available: vec![],
            },
            TaskError::Io(e) => ErrorInfo {
                message: format!("IO error: {}", e),
                error_type: "io_error".to_string(),
                suggestion: None,
                exit_code: None,
                stderr: None,
                available: vec![],
            },
        }
    }
}

/// Suggest fixes for common error patterns
pub fn suggest_fix(command: &str, stderr: &str) -> Option<String> {
    // Docker-related errors
    if stderr.contains("docker") || stderr.contains("Docker") {
        if stderr.contains("not running") || stderr.contains("Cannot connect") {
            return Some(
                "Docker daemon is not running. Start Docker Desktop or the Docker service."
                    .to_string(),
            );
        }
        if stderr.contains("No such container") {
            return Some(
                "Container not found. Try running 'up' first to start the services.".to_string(),
            );
        }
        if stderr.contains("port is already allocated") {
            return Some(
                "Port conflict. Stop the conflicting service or use a different port.".to_string(),
            );
        }
    }

    // Permission errors
    if stderr.contains("Permission denied") {
        return Some(
            "Permission denied. Check file permissions or run with appropriate access.".to_string(),
        );
    }

    // Command not found
    if stderr.contains("command not found") || stderr.contains("not found") {
        if command.contains("make") {
            return Some("'make' command not found. Install build-essential or make.".to_string());
        }
        if command.contains("just") {
            return Some("'just' command not found. Install just: cargo install just".to_string());
        }
        return Some("Required command not found. Check PATH and dependencies.".to_string());
    }

    // File not found
    if stderr.contains("No such file") {
        if command.contains("run.sh") {
            return Some("run.sh not found. Verify the working directory is correct.".to_string());
        }
        return Some("File not found. Check the project path and file existence.".to_string());
    }

    // Make-specific errors
    if stderr.contains("No rule to make target") {
        return Some(
            "Target not found in Makefile. Run 'list_tasks' to see available targets.".to_string(),
        );
    }

    // Just-specific errors
    if stderr.contains("Justfile does not contain recipe") {
        return Some(
            "Recipe not found in justfile. Run 'list_tasks' to see available recipes.".to_string(),
        );
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_not_found_error() {
        let err = TaskError::ProjectNotFound {
            path: "/nonexistent".to_string(),
            suggestion: Some("Check the path".to_string()),
        };
        assert_eq!(err.to_string(), "Project not found: /nonexistent");

        let info = ErrorInfo::from(&err);
        assert_eq!(info.error_type, "project_not_found");
        assert_eq!(info.suggestion, Some("Check the path".to_string()));
    }

    #[test]
    fn test_no_runner_detected_error() {
        let err = TaskError::NoRunnerDetected {
            path: "/some/project".to_string(),
            available: vec!["make".to_string(), "just".to_string()],
        };
        assert!(err.to_string().contains("No build system detected"));

        let info = ErrorInfo::from(&err);
        assert_eq!(info.error_type, "no_runner_detected");
        assert_eq!(info.available, vec!["make", "just"]);
    }

    #[test]
    fn test_task_not_found_error() {
        let err = TaskError::TaskNotFound {
            task: "deploy".to_string(),
            available: vec!["build".to_string(), "test".to_string()],
            suggestion: Some("Did you mean 'build'?".to_string()),
        };
        assert_eq!(err.to_string(), "Task 'deploy' not found");

        let info = ErrorInfo::from(&err);
        assert_eq!(info.error_type, "task_not_found");
        assert!(info.available.contains(&"build".to_string()));
    }

    #[test]
    fn test_command_failed_error() {
        let err = TaskError::CommandFailed {
            command: "make build".to_string(),
            exit_code: Some(1),
            stderr: "error: compilation failed".to_string(),
            suggestion: None,
        };
        assert_eq!(err.to_string(), "Command failed: make build");

        let info = ErrorInfo::from(&err);
        assert_eq!(info.exit_code, Some(1));
        assert_eq!(info.stderr, Some("error: compilation failed".to_string()));
    }

    #[test]
    fn test_timeout_error() {
        let err = TaskError::Timeout {
            command: "make test".to_string(),
            timeout_secs: 300,
        };
        assert!(err.to_string().contains("timed out"));
        assert!(err.to_string().contains("300s"));
    }

    #[test]
    fn test_suggest_fix_docker_not_running() {
        let suggestion = suggest_fix("docker-compose up", "Cannot connect to Docker daemon");
        assert!(suggestion.is_some());
        assert!(suggestion.unwrap().contains("Docker daemon"));
    }

    #[test]
    fn test_suggest_fix_permission_denied() {
        let suggestion = suggest_fix("./run.sh build", "Permission denied");
        assert!(suggestion.is_some());
        assert!(suggestion.unwrap().contains("Permission"));
    }

    #[test]
    fn test_suggest_fix_command_not_found() {
        let suggestion = suggest_fix("make build", "make: command not found");
        assert!(suggestion.is_some());
        assert!(suggestion.unwrap().contains("make"));
    }

    #[test]
    fn test_suggest_fix_no_such_file() {
        let suggestion = suggest_fix("./run.sh build", "No such file or directory");
        assert!(suggestion.is_some());
        assert!(suggestion.unwrap().contains("run.sh"));
    }

    #[test]
    fn test_suggest_fix_make_target() {
        let suggestion = suggest_fix("make deploy", "No rule to make target 'deploy'");
        assert!(suggestion.is_some());
        assert!(suggestion.unwrap().contains("Makefile"));
    }

    #[test]
    fn test_suggest_fix_no_match() {
        let suggestion = suggest_fix("some command", "some random error");
        assert!(suggestion.is_none());
    }

    #[test]
    fn test_error_info_serialization() {
        let info = ErrorInfo {
            message: "Test error".to_string(),
            error_type: "test".to_string(),
            suggestion: Some("Fix it".to_string()),
            exit_code: Some(1),
            stderr: Some("error output".to_string()),
            available: vec!["option1".to_string()],
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("Test error"));
        assert!(json.contains("exit_code"));
    }

    #[test]
    fn test_error_info_skips_empty_fields() {
        let info = ErrorInfo {
            message: "Test".to_string(),
            error_type: "test".to_string(),
            suggestion: None,
            exit_code: None,
            stderr: None,
            available: vec![],
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("suggestion"));
        assert!(!json.contains("exit_code"));
        assert!(!json.contains("stderr"));
        assert!(!json.contains("available"));
    }
}
