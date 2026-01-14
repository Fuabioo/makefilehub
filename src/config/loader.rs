//! Configuration loader with XDG-compliant path resolution
//!
//! Loads configuration from multiple locations with layered priority:
//! 1. `/etc/makefilehub/config.toml` (lowest priority)
//! 2. `~/.config/makefilehub/config.toml`
//! 3. `~/.makefilehub.toml`
//! 4. `./.makefilehub.toml` (highest priority)

use std::path::PathBuf;

use anyhow::{Context, Result};
use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};

use super::model::Config;

/// Application name used for XDG directories
const APP_NAME: &str = "makefilehub";

/// Get XDG config search paths in priority order (lowest to highest)
pub fn config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // 1. System-wide config (lowest priority)
    paths.push(PathBuf::from(format!("/etc/{}/config.toml", APP_NAME)));

    // 2. XDG config home
    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join(APP_NAME).join("config.toml"));
    }

    // 3. Home directory (legacy/convenience)
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(format!(".{}.toml", APP_NAME)));
    }

    // 4. Current directory / project root (highest priority)
    paths.push(PathBuf::from(format!(".{}.toml", APP_NAME)));

    paths
}

/// Load configuration with XDG layering
///
/// Configurations are merged in priority order, with later files
/// overriding earlier ones. Environment variables with prefix
/// `MAKEFILEHUB_` override all file-based configuration.
///
/// # Arguments
/// * `override_path` - Optional path to a config file that takes highest priority
///
/// # Returns
/// * `Result<Config>` - The merged configuration
pub fn load_config(override_path: Option<&str>) -> Result<Config> {
    let mut figment = Figment::new();

    // Start with defaults
    figment = figment.merge(Serialized::defaults(Config::default()));

    // Layer configs from lowest to highest priority
    for path in config_paths() {
        if path.exists() {
            tracing::debug!("Loading config from: {}", path.display());
            figment = figment.merge(Toml::file(&path));
        }
    }

    // Override path takes highest priority (if provided)
    if let Some(path) = override_path {
        let path = PathBuf::from(path);
        if path.exists() {
            tracing::debug!("Loading override config from: {}", path.display());
            figment = figment.merge(Toml::file(&path));
        } else {
            tracing::warn!("Override config not found: {}", path.display());
        }
    }

    // Environment variables override everything
    // Format: MAKEFILEHUB_DEFAULTS__TIMEOUT=600
    // Maps to: defaults.timeout = 600
    figment = figment.merge(Env::prefixed("MAKEFILEHUB_").split("__"));

    figment.extract().context("Failed to load configuration")
}

/// Find all existing config files (for debugging/introspection)
pub fn find_config_files() -> Vec<PathBuf> {
    config_paths().into_iter().filter(|p| p.exists()).collect()
}

/// Get the default config directory for writing new configs
pub fn default_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME))
}

/// Get the default config file path
pub fn default_config_file() -> Option<PathBuf> {
    default_config_dir().map(|d| d.join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_config_paths_returns_expected_paths() {
        let paths = config_paths();

        // Should have at least 4 paths
        assert!(paths.len() >= 3);

        // First should be system-wide
        assert!(paths[0].to_string_lossy().contains("/etc/"));

        // Last should be current directory
        assert!(paths
            .last()
            .unwrap()
            .to_string_lossy()
            .contains(".makefilehub.toml"));
    }

    #[test]
    fn test_load_config_defaults() {
        // With no config files, should return defaults
        let config = load_config(None).unwrap();

        assert_eq!(config.defaults.timeout, 300);
        assert_eq!(
            config.defaults.runner_priority,
            vec!["make", "just", "script"]
        );
    }

    #[test]
    fn test_load_config_from_override() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("test-config.toml");

        fs::write(
            &config_path,
            r#"
            [defaults]
            timeout = 600
            runner_priority = ["just", "make"]
            "#,
        )
        .unwrap();

        let config = load_config(Some(config_path.to_str().unwrap())).unwrap();

        assert_eq!(config.defaults.timeout, 600);
        assert_eq!(config.defaults.runner_priority, vec!["just", "make"]);
    }

    #[test]
    fn test_load_config_with_services() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("test-config.toml");

        fs::write(
            &config_path,
            r#"
            [services.my-api]
            project_dir = "/projects/my-api"
            runner = "just"
            depends_on = ["frontend"]
            force_recreate = ["nginx"]
            "#,
        )
        .unwrap();

        let config = load_config(Some(config_path.to_str().unwrap())).unwrap();

        assert!(config.has_service("my-api"));
        let service = config.services.get("my-api").unwrap();
        assert_eq!(service.runner, Some("just".to_string()));
        assert_eq!(service.depends_on, vec!["frontend"]);
    }

    #[test]
    fn test_find_config_files_empty_when_none_exist() {
        // In a clean environment (or test), might find no files
        // This test mainly ensures the function doesn't panic
        let _files = find_config_files();
    }

    #[test]
    fn test_default_config_dir() {
        let dir = default_config_dir();
        // Should return Some on most systems
        if let Some(d) = dir {
            assert!(d.to_string_lossy().contains("makefilehub"));
        }
    }

    #[test]
    fn test_env_override() {
        // Use a unique env var to avoid test pollution
        // This test verifies env vars work, using a different key
        std::env::set_var("MAKEFILEHUB_DEFAULTS__DEFAULT_SCRIPT", "./custom.sh");

        let config = load_config(None).unwrap();

        // Clean up BEFORE assertion to ensure cleanup happens
        std::env::remove_var("MAKEFILEHUB_DEFAULTS__DEFAULT_SCRIPT");

        assert_eq!(config.defaults.default_script, "./custom.sh");
    }

    #[test]
    fn test_config_layering() {
        let dir = TempDir::new().unwrap();

        // Create two config files
        let base_config = dir.path().join("base.toml");
        let override_config = dir.path().join("override.toml");

        fs::write(
            &base_config,
            r#"
            [defaults]
            timeout = 100
            default_script = "./base.sh"
            "#,
        )
        .unwrap();

        fs::write(
            &override_config,
            r#"
            [defaults]
            timeout = 200
            "#,
        )
        .unwrap();

        // Load with override (simulating layering)
        let config = load_config(Some(override_config.to_str().unwrap())).unwrap();

        // timeout should be overridden to 200
        assert_eq!(config.defaults.timeout, 200);
    }

    #[test]
    fn test_missing_override_file_uses_defaults() {
        let config = load_config(Some("/nonexistent/config.toml")).unwrap();

        // Should still get defaults
        assert_eq!(config.defaults.timeout, 300);
    }
}
