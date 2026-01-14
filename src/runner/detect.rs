//! Build system auto-detection
//!
//! Detects which build system a project uses by checking for:
//! - Makefile or makefile (make)
//! - justfile or Justfile (just)
//! - Custom scripts like run.sh, build.sh (configurable)

use std::path::Path;

use serde::Serialize;

use crate::config::Config;

/// Type of build system runner
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum RunnerType {
    /// GNU Make with Makefile
    Make,
    /// just command runner with justfile
    Just,
    /// Custom script (e.g., run.sh, build.sh)
    Script(String),
}

impl RunnerType {
    /// Get the display name for this runner type
    pub fn name(&self) -> &str {
        match self {
            RunnerType::Make => "make",
            RunnerType::Just => "just",
            RunnerType::Script(s) => s,
        }
    }

    /// Get the typical filename for this runner type
    pub fn filename(&self) -> &str {
        match self {
            RunnerType::Make => "Makefile",
            RunnerType::Just => "justfile",
            RunnerType::Script(s) => s,
        }
    }
}

impl std::fmt::Display for RunnerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunnerType::Make => write!(f, "make"),
            RunnerType::Just => write!(f, "just"),
            RunnerType::Script(s) => write!(f, "script:{}", s),
        }
    }
}

/// Files found during detection
#[derive(Debug, Clone, Default, Serialize)]
pub struct FilesFound {
    /// Whether a Makefile was found
    pub makefile: bool,
    /// Path to makefile if found (could be "Makefile" or "makefile")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub makefile_path: Option<String>,
    /// Whether a justfile was found
    pub justfile: bool,
    /// Path to justfile if found (could be "justfile" or "Justfile")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justfile_path: Option<String>,
    /// Scripts found
    pub scripts: Vec<String>,
}

/// Result of build system detection
#[derive(Debug, Clone, Serialize)]
#[derive(Default)]
pub struct DetectionResult {
    /// The detected runner (first match by priority)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected: Option<RunnerType>,
    /// All available runners found
    pub available: Vec<RunnerType>,
    /// Details about files found
    pub files_found: FilesFound,
}


/// Detect which build system a project uses
///
/// Checks for build files in the given directory according to the
/// priority order configured in `config.defaults.runner_priority`.
///
/// # Arguments
/// * `dir` - Directory to check
/// * `config` - Configuration with runner priority and script list
///
/// # Returns
/// * `DetectionResult` with detected runner and all available options
pub fn detect_runner(dir: &Path, config: &Config) -> DetectionResult {
    let mut result = DetectionResult::default();

    // Check for each runner type according to priority
    for runner in &config.defaults.runner_priority {
        match runner.as_str() {
            "make" => {
                check_makefile(dir, &mut result);
            }
            "just" => {
                check_justfile(dir, &mut result);
            }
            "script" => {
                check_scripts(dir, config, &mut result);
            }
            _ => {
                tracing::warn!("Unknown runner type in priority list: {}", runner);
            }
        }
    }

    result
}

/// Check for Makefile in the directory
fn check_makefile(dir: &Path, result: &mut DetectionResult) {
    // Check both "Makefile" and "makefile"
    for name in &["Makefile", "makefile", "GNUmakefile"] {
        let path = dir.join(name);
        if path.exists() && path.is_file() {
            result.files_found.makefile = true;
            result.files_found.makefile_path = Some(name.to_string());
            result.available.push(RunnerType::Make);

            if result.detected.is_none() {
                result.detected = Some(RunnerType::Make);
            }
            break;
        }
    }
}

/// Check for justfile in the directory
fn check_justfile(dir: &Path, result: &mut DetectionResult) {
    // Check both "justfile" and "Justfile"
    for name in &["justfile", "Justfile", ".justfile"] {
        let path = dir.join(name);
        if path.exists() && path.is_file() {
            result.files_found.justfile = true;
            result.files_found.justfile_path = Some(name.to_string());
            result.available.push(RunnerType::Just);

            if result.detected.is_none() {
                result.detected = Some(RunnerType::Just);
            }
            break;
        }
    }
}

/// Check for custom scripts in the directory
fn check_scripts(dir: &Path, config: &Config, result: &mut DetectionResult) {
    for script_name in &config.runners.script.scripts {
        // Handle both relative (./run.sh) and plain (run.sh) names
        let script_name_clean = script_name.strip_prefix("./").unwrap_or(script_name);
        let path = dir.join(script_name_clean);

        if path.exists() && path.is_file() {
            // Check if executable on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = path.metadata() {
                    let permissions = metadata.permissions();
                    if permissions.mode() & 0o111 == 0 {
                        // Not executable, skip
                        tracing::debug!(
                            "Script {} exists but is not executable",
                            script_name_clean
                        );
                        continue;
                    }
                }
            }

            let script_path = format!("./{}", script_name_clean);
            result.files_found.scripts.push(script_path.clone());
            result
                .available
                .push(RunnerType::Script(script_path.clone()));

            if result.detected.is_none() {
                result.detected = Some(RunnerType::Script(script_path));
            }
        }
    }
}

/// Check if a specific runner type is available in a directory
pub fn is_runner_available(dir: &Path, runner: &RunnerType) -> bool {
    match runner {
        RunnerType::Make => {
            dir.join("Makefile").exists()
                || dir.join("makefile").exists()
                || dir.join("GNUmakefile").exists()
        }
        RunnerType::Just => {
            dir.join("justfile").exists()
                || dir.join("Justfile").exists()
                || dir.join(".justfile").exists()
        }
        RunnerType::Script(name) => {
            let name_clean = name.strip_prefix("./").unwrap_or(name);
            let path = dir.join(name_clean);
            if path.exists() && path.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(metadata) = path.metadata() {
                        return metadata.permissions().mode() & 0o111 != 0;
                    }
                }
                #[cfg(not(unix))]
                {
                    return true;
                }
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn default_config() -> Config {
        Config::default()
    }

    #[test]
    fn test_detect_makefile() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Makefile"), "build:\n\t@echo building").unwrap();

        let result = detect_runner(dir.path(), &default_config());

        assert!(result.detected.is_some());
        assert_eq!(result.detected.unwrap(), RunnerType::Make);
        assert!(result.files_found.makefile);
        assert_eq!(
            result.files_found.makefile_path,
            Some("Makefile".to_string())
        );
    }

    #[test]
    fn test_detect_lowercase_makefile() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("makefile"), "build:\n\t@echo building").unwrap();

        let result = detect_runner(dir.path(), &default_config());

        assert_eq!(result.detected, Some(RunnerType::Make));
        assert_eq!(
            result.files_found.makefile_path,
            Some("makefile".to_string())
        );
    }

    #[test]
    fn test_detect_justfile() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("justfile"), "build:\n    @echo building").unwrap();

        let result = detect_runner(dir.path(), &default_config());

        assert!(result.detected.is_some());
        assert_eq!(result.detected.unwrap(), RunnerType::Just);
        assert!(result.files_found.justfile);
    }

    #[test]
    fn test_detect_uppercase_justfile() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Justfile"), "build:\n    @echo building").unwrap();

        let result = detect_runner(dir.path(), &default_config());

        assert_eq!(result.detected, Some(RunnerType::Just));
    }

    #[test]
    fn test_detect_script() {
        let dir = TempDir::new().unwrap();
        let script_path = dir.path().join("run.sh");
        fs::write(&script_path, "#!/bin/bash\necho hello").unwrap();

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).unwrap();
        }

        let result = detect_runner(dir.path(), &default_config());

        assert!(result.detected.is_some());
        if let Some(RunnerType::Script(name)) = result.detected {
            assert!(name.contains("run.sh"));
        } else {
            panic!("Expected Script runner");
        }
    }

    #[test]
    fn test_detect_priority_make_first() {
        let dir = TempDir::new().unwrap();

        // Create both Makefile and justfile
        fs::write(dir.path().join("Makefile"), "build:").unwrap();
        fs::write(dir.path().join("justfile"), "build:").unwrap();

        let result = detect_runner(dir.path(), &default_config());

        // make is first in default priority
        assert_eq!(result.detected, Some(RunnerType::Make));
        assert_eq!(result.available.len(), 2);
    }

    #[test]
    fn test_detect_priority_just_first() {
        let dir = TempDir::new().unwrap();

        // Create both Makefile and justfile
        fs::write(dir.path().join("Makefile"), "build:").unwrap();
        fs::write(dir.path().join("justfile"), "build:").unwrap();

        let mut config = default_config();
        config.defaults.runner_priority = vec!["just".to_string(), "make".to_string()];

        let result = detect_runner(dir.path(), &config);

        // just is first in custom priority
        assert_eq!(result.detected, Some(RunnerType::Just));
    }

    #[test]
    fn test_detect_empty_directory() {
        let dir = TempDir::new().unwrap();

        let result = detect_runner(dir.path(), &default_config());

        assert!(result.detected.is_none());
        assert!(result.available.is_empty());
        assert!(!result.files_found.makefile);
        assert!(!result.files_found.justfile);
        assert!(result.files_found.scripts.is_empty());
    }

    #[test]
    fn test_detect_non_executable_script() {
        let dir = TempDir::new().unwrap();
        let script_path = dir.path().join("run.sh");
        fs::write(&script_path, "#!/bin/bash\necho hello").unwrap();

        // Don't make it executable

        let result = detect_runner(dir.path(), &default_config());

        // On Unix, non-executable scripts should not be detected
        #[cfg(unix)]
        {
            assert!(
                result.detected.is_none()
                    || !matches!(result.detected, Some(RunnerType::Script(_)))
            );
        }
    }

    #[test]
    fn test_detect_all_available() {
        let dir = TempDir::new().unwrap();

        // Create all types
        fs::write(dir.path().join("Makefile"), "build:").unwrap();
        fs::write(dir.path().join("justfile"), "build:").unwrap();
        let script_path = dir.path().join("run.sh");
        fs::write(&script_path, "#!/bin/bash").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).unwrap();
        }

        let result = detect_runner(dir.path(), &default_config());

        // Should have all three available
        assert_eq!(result.available.len(), 3);
        assert!(result.available.contains(&RunnerType::Make));
        assert!(result.available.contains(&RunnerType::Just));
    }

    #[test]
    fn test_runner_type_display() {
        assert_eq!(RunnerType::Make.to_string(), "make");
        assert_eq!(RunnerType::Just.to_string(), "just");
        assert_eq!(
            RunnerType::Script("./run.sh".to_string()).to_string(),
            "script:./run.sh"
        );
    }

    #[test]
    fn test_is_runner_available() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Makefile"), "build:").unwrap();

        assert!(is_runner_available(dir.path(), &RunnerType::Make));
        assert!(!is_runner_available(dir.path(), &RunnerType::Just));
    }

    #[test]
    fn test_detection_result_serialization() {
        let result = DetectionResult {
            detected: Some(RunnerType::Make),
            available: vec![RunnerType::Make, RunnerType::Just],
            files_found: FilesFound {
                makefile: true,
                makefile_path: Some("Makefile".to_string()),
                justfile: true,
                justfile_path: Some("justfile".to_string()),
                scripts: vec![],
            },
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"type\":\"Make\""));
        assert!(json.contains("\"makefile\":true"));
    }
}
