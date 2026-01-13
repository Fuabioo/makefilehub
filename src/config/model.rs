//! Configuration model for makefilehub
//!
//! Defines the structure for XDG-compliant layered configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Root configuration structure
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Config {
    /// Default settings applied to all projects
    #[serde(default)]
    pub defaults: Defaults,

    /// Project directory patterns for service lookup
    #[serde(default)]
    pub projects: ProjectsConfig,

    /// Runner-specific configuration
    #[serde(default)]
    pub runners: RunnersConfig,

    /// Service-specific overrides for rebuild_service orchestration
    #[serde(default)]
    pub services: HashMap<String, ServiceConfig>,
}

/// Default settings applied to all projects
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Defaults {
    /// Runner detection priority (first found wins)
    #[serde(default = "default_runner_priority")]
    pub runner_priority: Vec<String>,

    /// Default script to look for if no Makefile/justfile
    #[serde(default = "default_script")]
    pub default_script: String,

    /// Task name aliases (normalize across build systems)
    #[serde(default)]
    pub task_aliases: HashMap<String, Vec<String>>,

    /// Default timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_runner_priority() -> Vec<String> {
    vec!["make".to_string(), "just".to_string(), "script".to_string()]
}

fn default_script() -> String {
    "./run.sh".to_string()
}

fn default_timeout() -> u64 {
    300
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            runner_priority: default_runner_priority(),
            default_script: default_script(),
            task_aliases: HashMap::new(),
            timeout: default_timeout(),
        }
    }
}

/// Project directory patterns configuration
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ProjectsConfig {
    /// Patterns for resolving project directories
    /// {name} is replaced with the service/project name
    #[serde(default = "default_patterns")]
    pub patterns: Vec<String>,
}

fn default_patterns() -> Vec<String> {
    vec![
        "$HOME/projects/{name}".to_string(),
        "$HOME/work/{name}".to_string(),
        "./{name}".to_string(),
    ]
}

/// Runner-specific configurations
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RunnersConfig {
    #[serde(default)]
    pub make: MakeConfig,

    #[serde(default)]
    pub just: JustConfig,

    #[serde(default)]
    pub script: ScriptConfig,
}

/// Makefile runner configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MakeConfig {
    /// Command to execute make
    #[serde(default = "default_make_command")]
    pub command: String,

    /// Command to list targets
    #[serde(default = "default_make_list_cmd")]
    pub list_targets_cmd: String,
}

fn default_make_command() -> String {
    "make".to_string()
}

fn default_make_list_cmd() -> String {
    "make -pRrq : 2>/dev/null | awk -F: '/^[a-zA-Z0-9_-]+:/ {print $1}'".to_string()
}

impl Default for MakeConfig {
    fn default() -> Self {
        Self {
            command: default_make_command(),
            list_targets_cmd: default_make_list_cmd(),
        }
    }
}

/// justfile runner configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JustConfig {
    /// Command to execute just
    #[serde(default = "default_just_command")]
    pub command: String,

    /// Command to list recipes
    #[serde(default = "default_just_list_cmd")]
    pub list_targets_cmd: String,
}

fn default_just_command() -> String {
    "just".to_string()
}

fn default_just_list_cmd() -> String {
    "just --list --unsorted".to_string()
}

impl Default for JustConfig {
    fn default() -> Self {
        Self {
            command: default_just_command(),
            list_targets_cmd: default_just_list_cmd(),
        }
    }
}

/// Script runner configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScriptConfig {
    /// Scripts to look for in order
    #[serde(default = "default_scripts")]
    pub scripts: Vec<String>,

    /// How to list available commands
    #[serde(default = "default_list_mode")]
    pub list_mode: String,
}

fn default_scripts() -> Vec<String> {
    vec![
        "./run.sh".to_string(),
        "./build.sh".to_string(),
        "./task.sh".to_string(),
    ]
}

fn default_list_mode() -> String {
    "help".to_string()
}

impl Default for ScriptConfig {
    fn default() -> Self {
        Self {
            scripts: default_scripts(),
            list_mode: default_list_mode(),
        }
    }
}

/// Service-specific configuration for rebuild_service orchestration
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ServiceConfig {
    /// Project directory for this service
    pub project_dir: Option<String>,

    /// Force a specific runner
    pub runner: Option<String>,

    /// Script to use (for script runner)
    pub script: Option<String>,

    /// Services that depend on this one (will be restarted after build)
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Containers to force-recreate after build
    #[serde(default)]
    pub force_recreate: Vec<String>,

    /// Task name overrides for this service
    #[serde(default)]
    pub tasks: HashMap<String, String>,

    /// Environment variables for this service
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Timeout override in seconds
    pub timeout: Option<u64>,
}

/// Fully resolved service configuration (after applying defaults)
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedService {
    pub name: String,
    pub project_dir: String,
    pub runner: Option<String>,
    pub script: Option<String>,
    pub depends_on: Vec<String>,
    pub force_recreate: Vec<String>,
    pub tasks: HashMap<String, String>,
    pub env: HashMap<String, String>,
    pub timeout: u64,
}

impl Config {
    /// Get resolved configuration for a service
    pub fn get_service(&self, name: &str) -> ResolvedService {
        let service = self.services.get(name);

        let project_dir = service
            .and_then(|s| s.project_dir.clone())
            .unwrap_or_else(|| self.resolve_project_dir(name));

        ResolvedService {
            name: name.to_string(),
            project_dir,
            runner: service.and_then(|s| s.runner.clone()),
            script: service.and_then(|s| s.script.clone()),
            depends_on: service.map(|s| s.depends_on.clone()).unwrap_or_default(),
            force_recreate: service.map(|s| s.force_recreate.clone()).unwrap_or_default(),
            tasks: service.map(|s| s.tasks.clone()).unwrap_or_default(),
            env: service.map(|s| s.env.clone()).unwrap_or_default(),
            timeout: service
                .and_then(|s| s.timeout)
                .unwrap_or(self.defaults.timeout),
        }
    }

    /// Resolve project directory using patterns
    fn resolve_project_dir(&self, name: &str) -> String {
        // Try each pattern and return the first one that exists
        for pattern in &self.projects.patterns {
            let path = pattern.replace("{name}", name);
            // Expand $HOME
            let expanded = if path.starts_with("$HOME") {
                if let Some(home) = dirs::home_dir() {
                    path.replace("$HOME", home.to_string_lossy().as_ref())
                } else {
                    path
                }
            } else {
                path
            };

            if Path::new(&expanded).exists() {
                return expanded;
            }
        }

        // Fallback to first pattern (unexpanded)
        self.projects
            .patterns
            .first()
            .map(|p| p.replace("{name}", name))
            .unwrap_or_else(|| format!("./{}", name))
    }

    /// List all configured service names
    pub fn list_services(&self) -> Vec<String> {
        self.services.keys().cloned().collect()
    }

    /// Check if a service is configured
    pub fn has_service(&self, name: &str) -> bool {
        self.services.contains_key(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();

        assert_eq!(config.defaults.runner_priority, vec!["make", "just", "script"]);
        assert_eq!(config.defaults.default_script, "./run.sh");
        assert_eq!(config.defaults.timeout, 300);
    }

    #[test]
    fn test_default_runners_config() {
        let config = Config::default();

        assert_eq!(config.runners.make.command, "make");
        assert_eq!(config.runners.just.command, "just");
        assert_eq!(config.runners.script.scripts, vec!["./run.sh", "./build.sh", "./task.sh"]);
    }

    #[test]
    fn test_deserialize_minimal_config() {
        let toml = r#"
            [defaults]
            timeout = 600
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.defaults.timeout, 600);
        // Defaults should still apply
        assert_eq!(config.defaults.runner_priority, vec!["make", "just", "script"]);
    }

    #[test]
    fn test_deserialize_full_config() {
        let toml = r#"
            [defaults]
            runner_priority = ["just", "make", "script"]
            default_script = "./build.sh"
            timeout = 120

            [projects]
            patterns = ["$HOME/myprojects/{name}", "./{name}"]

            [runners.make]
            command = "/usr/bin/make"

            [runners.just]
            command = "/usr/local/bin/just"

            [runners.script]
            scripts = ["./run.sh", "./scripts/build.sh"]
            list_mode = "hardcoded"

            [services.my-api]
            project_dir = "$HOME/projects/my-api"
            runner = "just"
            depends_on = ["my-frontend"]
            force_recreate = ["nginx"]
            timeout = 60
        "#;

        let config: Config = toml::from_str(toml).unwrap();

        assert_eq!(config.defaults.runner_priority, vec!["just", "make", "script"]);
        assert_eq!(config.defaults.default_script, "./build.sh");
        assert_eq!(config.defaults.timeout, 120);
        assert_eq!(config.projects.patterns.len(), 2);
        assert_eq!(config.runners.make.command, "/usr/bin/make");
        assert_eq!(config.runners.script.list_mode, "hardcoded");

        let service = config.services.get("my-api").unwrap();
        assert_eq!(service.runner, Some("just".to_string()));
        assert_eq!(service.depends_on, vec!["my-frontend"]);
        assert_eq!(service.timeout, Some(60));
    }

    #[test]
    fn test_get_service_with_config() {
        let toml = r#"
            [defaults]
            timeout = 300

            [services.web-api]
            project_dir = "/projects/web-api"
            runner = "script"
            depends_on = ["frontend"]
            force_recreate = ["nginx"]
            timeout = 120
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        let resolved = config.get_service("web-api");

        assert_eq!(resolved.name, "web-api");
        assert_eq!(resolved.project_dir, "/projects/web-api");
        assert_eq!(resolved.runner, Some("script".to_string()));
        assert_eq!(resolved.depends_on, vec!["frontend"]);
        assert_eq!(resolved.force_recreate, vec!["nginx"]);
        assert_eq!(resolved.timeout, 120);
    }

    #[test]
    fn test_get_service_without_config() {
        let config = Config::default();
        let resolved = config.get_service("unknown-service");

        assert_eq!(resolved.name, "unknown-service");
        assert!(resolved.runner.is_none());
        assert!(resolved.depends_on.is_empty());
        assert_eq!(resolved.timeout, 300); // Default timeout
    }

    #[test]
    fn test_list_services() {
        let toml = r#"
            [services.api]
            project_dir = "/api"

            [services.web]
            project_dir = "/web"
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        let services = config.list_services();

        assert_eq!(services.len(), 2);
        assert!(services.contains(&"api".to_string()));
        assert!(services.contains(&"web".to_string()));
    }

    #[test]
    fn test_has_service() {
        let toml = r#"
            [services.my-service]
            project_dir = "/my-service"
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.has_service("my-service"));
        assert!(!config.has_service("other-service"));
    }

    #[test]
    fn test_service_config_defaults() {
        let service = ServiceConfig::default();

        assert!(service.project_dir.is_none());
        assert!(service.runner.is_none());
        assert!(service.depends_on.is_empty());
        assert!(service.force_recreate.is_empty());
        assert!(service.tasks.is_empty());
        assert!(service.env.is_empty());
        assert!(service.timeout.is_none());
    }

    #[test]
    fn test_task_aliases() {
        let toml = r#"
            [defaults.task_aliases]
            build = ["build", "compile", "make"]
            up = ["up", "start", "run"]
        "#;

        let config: Config = toml::from_str(toml).unwrap();

        assert_eq!(
            config.defaults.task_aliases.get("build"),
            Some(&vec!["build".to_string(), "compile".to_string(), "make".to_string()])
        );
        assert_eq!(
            config.defaults.task_aliases.get("up"),
            Some(&vec!["up".to_string(), "start".to_string(), "run".to_string()])
        );
    }

    #[test]
    fn test_service_env_vars() {
        let toml = r#"
            [services.my-service]
            project_dir = "/service"

            [services.my-service.env]
            API_KEY = "secret"
            DEBUG = "true"
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        let service = config.services.get("my-service").unwrap();

        assert_eq!(service.env.get("API_KEY"), Some(&"secret".to_string()));
        assert_eq!(service.env.get("DEBUG"), Some(&"true".to_string()));
    }

    #[test]
    fn test_service_task_overrides() {
        let toml = r#"
            [services.my-service]
            project_dir = "/service"

            [services.my-service.tasks]
            build = "compile"
            up = "start"
        "#;

        let config: Config = toml::from_str(toml).unwrap();
        let service = config.services.get("my-service").unwrap();

        assert_eq!(service.tasks.get("build"), Some(&"compile".to_string()));
        assert_eq!(service.tasks.get("up"), Some(&"start".to_string()));
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();

        // Should be able to deserialize what we serialized
        let _: Config = toml::from_str(&toml_str).unwrap();
    }

    #[test]
    fn test_resolved_service_serialization() {
        let resolved = ResolvedService {
            name: "test".to_string(),
            project_dir: "/test".to_string(),
            runner: Some("make".to_string()),
            script: None,
            depends_on: vec!["dep".to_string()],
            force_recreate: vec!["container".to_string()],
            tasks: HashMap::new(),
            env: HashMap::new(),
            timeout: 300,
        };

        let json = serde_json::to_string(&resolved).unwrap();
        assert!(json.contains("\"name\":\"test\""));
        assert!(json.contains("\"runner\":\"make\""));
    }
}
