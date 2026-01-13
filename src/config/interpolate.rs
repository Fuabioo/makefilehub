//! Configuration value interpolation
//!
//! Supports environment variable and shell command interpolation in config values:
//! - `$VAR` or `${VAR}` - Environment variable substitution
//! - `$(command)` - Shell command execution
//!
//! # Security Note
//!
//! Shell command execution runs with the current user's permissions.
//! Config files should have restricted permissions (600) to prevent
//! unauthorized command execution.

use regex::Regex;
use std::process::Command;

/// Interpolate a string with environment variables and shell commands
///
/// # Interpolation Syntax
///
/// - `$VAR` - Simple environment variable
/// - `${VAR}` - Environment variable with explicit boundaries
/// - `$(command)` - Shell command execution
///
/// # Examples
///
/// ```
/// use makefilehub::config::interpolate::interpolate_string;
///
/// std::env::set_var("MY_VAR", "hello");
/// let result = interpolate_string("Value: $MY_VAR");
/// assert_eq!(result, "Value: hello");
/// std::env::remove_var("MY_VAR");
/// ```
pub fn interpolate_string(s: &str) -> String {
    let mut result = s.to_string();

    // First, handle shell commands: $(...)
    // Do this first so we don't accidentally interpret command output as variables
    result = interpolate_commands(&result);

    // Then, handle environment variables: ${VAR} or $VAR
    result = interpolate_env_vars(&result);

    result
}

/// Interpolate shell commands: $(command)
fn interpolate_commands(s: &str) -> String {
    let cmd_re = Regex::new(r"\$\(([^)]+)\)").expect("Invalid regex");

    cmd_re
        .replace_all(s, |caps: &regex::Captures| {
            let cmd = &caps[1];
            match execute_shell_command(cmd) {
                Ok(output) => output,
                Err(e) => {
                    tracing::warn!("Failed to execute config command '{}': {}", cmd, e);
                    // Return original on error so it's visible
                    format!("$({})_ERROR", cmd)
                }
            }
        })
        .to_string()
}

/// Interpolate environment variables: $VAR or ${VAR}
fn interpolate_env_vars(s: &str) -> String {
    // Match ${VAR} first (explicit boundaries)
    let bracketed_re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").expect("Invalid regex");
    let result = bracketed_re
        .replace_all(s, |caps: &regex::Captures| {
            let var = &caps[1];
            std::env::var(var).unwrap_or_else(|_| {
                tracing::debug!("Environment variable '{}' not set", var);
                String::new()
            })
        })
        .to_string();

    // Then match $VAR (simple form)
    // Match variable names that don't start with a digit
    // The regex crate doesn't support lookahead, so we use a simple approach:
    // Match $VARNAME where VARNAME starts with letter or underscore
    let simple_re = Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)").expect("Invalid regex");
    simple_re
        .replace_all(&result, |caps: &regex::Captures| {
            let var = &caps[1];
            std::env::var(var).unwrap_or_else(|_| {
                tracing::debug!("Environment variable '{}' not set", var);
                String::new()
            })
        })
        .to_string()
}

/// Execute a shell command and return its stdout
fn execute_shell_command(cmd: &str) -> Result<String, std::io::Error> {
    let output = Command::new("sh").arg("-c").arg(cmd).output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(std::io::Error::other(format!("Command failed: {}", stderr)))
    }
}

/// Interpolate all string values in a Config
///
/// This applies interpolation to string fields that commonly contain
/// paths or dynamic values (e.g., project_dir, env values).
pub fn interpolate_config(config: &mut super::model::Config) {
    // Interpolate project patterns
    for pattern in &mut config.projects.patterns {
        *pattern = interpolate_string(pattern);
    }

    // Interpolate service configs
    for service in config.services.values_mut() {
        if let Some(ref mut dir) = service.project_dir {
            *dir = interpolate_string(dir);
        }
        if let Some(ref mut script) = service.script {
            *script = interpolate_string(script);
        }
        for value in service.env.values_mut() {
            *value = interpolate_string(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolate_simple_env_var() {
        std::env::set_var("TEST_SIMPLE_VAR", "hello");

        let result = interpolate_string("Value: $TEST_SIMPLE_VAR");
        assert_eq!(result, "Value: hello");

        std::env::remove_var("TEST_SIMPLE_VAR");
    }

    #[test]
    fn test_interpolate_bracketed_env_var() {
        std::env::set_var("TEST_BRACKET_VAR", "world");

        let result = interpolate_string("Value: ${TEST_BRACKET_VAR}!");
        assert_eq!(result, "Value: world!");

        std::env::remove_var("TEST_BRACKET_VAR");
    }

    #[test]
    fn test_interpolate_home_var() {
        // HOME should be set on most systems
        if std::env::var("HOME").is_ok() {
            let result = interpolate_string("$HOME/projects");
            assert!(!result.starts_with("$HOME"));
            assert!(result.contains("/projects"));
        }
    }

    #[test]
    fn test_interpolate_missing_var() {
        let result = interpolate_string("Value: $NONEXISTENT_VAR_12345");
        assert_eq!(result, "Value: ");
    }

    #[test]
    fn test_interpolate_shell_command() {
        let result = interpolate_string("Value: $(echo hello)");
        assert_eq!(result, "Value: hello");
    }

    #[test]
    fn test_interpolate_shell_command_with_args() {
        let result = interpolate_string("$(echo -n 'test')");
        assert_eq!(result, "test");
    }

    #[test]
    fn test_interpolate_complex_shell_command() {
        let result = interpolate_string("Date: $(date +%Y)");
        // Should be a 4-digit year
        let year_part = result.strip_prefix("Date: ").unwrap();
        assert!(
            year_part.len() == 4,
            "Expected 4-digit year, got: {}",
            year_part
        );
        assert!(
            year_part.chars().all(|c| c.is_ascii_digit()),
            "Expected all digits, got: {}",
            year_part
        );
    }

    #[test]
    fn test_interpolate_failed_command() {
        let result = interpolate_string("Value: $(nonexistent_command_12345)");
        assert!(result.contains("_ERROR"));
    }

    #[test]
    fn test_interpolate_multiple_vars() {
        std::env::set_var("TEST_VAR_A", "foo");
        std::env::set_var("TEST_VAR_B", "bar");

        let result = interpolate_string("$TEST_VAR_A and $TEST_VAR_B");
        assert_eq!(result, "foo and bar");

        std::env::remove_var("TEST_VAR_A");
        std::env::remove_var("TEST_VAR_B");
    }

    #[test]
    fn test_interpolate_mixed_vars_and_commands() {
        std::env::set_var("TEST_MIXED_VAR", "world");

        let result = interpolate_string("Hello $(echo $TEST_MIXED_VAR)!");

        // The shell command executes first, then we get the result
        // Note: the inner $TEST_MIXED_VAR is interpreted by the shell, not our code
        assert_eq!(result, "Hello world!");

        std::env::remove_var("TEST_MIXED_VAR");
    }

    #[test]
    fn test_interpolate_no_vars() {
        let result = interpolate_string("No variables here");
        assert_eq!(result, "No variables here");
    }

    #[test]
    fn test_interpolate_adjacent_vars() {
        std::env::set_var("TEST_ADJ_A", "foo");
        std::env::set_var("TEST_ADJ_B", "bar");

        let result = interpolate_string("${TEST_ADJ_A}${TEST_ADJ_B}");
        assert_eq!(result, "foobar");

        std::env::remove_var("TEST_ADJ_A");
        std::env::remove_var("TEST_ADJ_B");
    }

    #[test]
    fn test_interpolate_var_in_path() {
        std::env::set_var("TEST_PROJECT", "myapp");

        let result = interpolate_string("/home/user/projects/$TEST_PROJECT/src");
        assert_eq!(result, "/home/user/projects/myapp/src");

        std::env::remove_var("TEST_PROJECT");
    }

    #[test]
    fn test_interpolate_preserves_non_var_dollar() {
        // $$ should not be interpreted as a variable
        // (In shell, $$ is the PID, but we don't support that)
        let result = interpolate_string("Price: $100");
        // $1 is not a valid var name (starts with digit), so it stays
        // Actually $100 starts with 1, which is a digit, so the regex won't match
        assert_eq!(result, "Price: $100");
    }

    #[test]
    fn test_interpolate_config() {
        let mut config = super::super::model::Config::default();
        config.projects.patterns = vec!["$HOME/projects/{name}".to_string()];

        interpolate_config(&mut config);

        if std::env::var("HOME").is_ok() {
            assert!(!config.projects.patterns[0].starts_with("$HOME"));
        }
    }

    #[test]
    fn test_interpolate_service_config() {
        use std::collections::HashMap;

        let mut config = super::super::model::Config::default();
        config.services.insert(
            "test".to_string(),
            super::super::model::ServiceConfig {
                project_dir: Some("$HOME/test".to_string()),
                env: {
                    let mut m = HashMap::new();
                    m.insert("TOKEN".to_string(), "$(echo secret)".to_string());
                    m
                },
                ..Default::default()
            },
        );

        interpolate_config(&mut config);

        let service = config.services.get("test").unwrap();

        // Project dir should be interpolated
        if std::env::var("HOME").is_ok() {
            assert!(!service.project_dir.as_ref().unwrap().starts_with("$HOME"));
        }

        // Env var with command should be interpolated
        assert_eq!(service.env.get("TOKEN"), Some(&"secret".to_string()));
    }
}
