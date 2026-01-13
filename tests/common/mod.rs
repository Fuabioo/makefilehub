//! Common test utilities for makefilehub tests

use std::path::PathBuf;
use tempfile::TempDir;

/// Creates a temporary directory with a Makefile
pub fn create_makefile_project(content: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let makefile_path = dir.path().join("Makefile");
    std::fs::write(&makefile_path, content).expect("Failed to write Makefile");
    let path = dir.path().to_path_buf();
    (dir, path)
}

/// Creates a temporary directory with a justfile
pub fn create_justfile_project(content: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let justfile_path = dir.path().join("justfile");
    std::fs::write(&justfile_path, content).expect("Failed to write justfile");
    let path = dir.path().to_path_buf();
    (dir, path)
}

/// Creates a temporary directory with a custom script
pub fn create_script_project(script_name: &str, content: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let script_path = dir.path().join(script_name);
    std::fs::write(&script_path, content).expect("Failed to write script");

    // Make script executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script_path)
            .expect("Failed to get metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).expect("Failed to set permissions");
    }

    let path = dir.path().to_path_buf();
    (dir, path)
}

/// Creates a temporary directory with no build files
pub fn create_empty_project() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let path = dir.path().to_path_buf();
    (dir, path)
}

/// Sample Makefile content for testing
pub const SAMPLE_MAKEFILE: &str = r#"
# Build the project
build:
	@echo "Building..."

# Run tests
test:
	@echo "Testing..."

# Clean build artifacts
clean:
	@echo "Cleaning..."

# Target with arguments
deploy: ARG ?= production
deploy:
	@echo "Deploying to $(ARG)..."
"#;

/// Sample justfile content for testing
pub const SAMPLE_JUSTFILE: &str = r#"
# Build the project
build target="release":
    @echo "Building {{target}}..."

# Run tests
test pattern="":
    @echo "Testing {{pattern}}..."

# Clean build artifacts
clean:
    @echo "Cleaning..."

# Default recipe
default: build
"#;

/// Sample run.sh script for testing
pub const SAMPLE_SCRIPT: &str = r#"#!/bin/bash
set -e

script_usage() {
    echo "Usage: $0 <command>"
    echo ""
    echo "Commands:"
    echo "  build    Build the project"
    echo "  test     Run tests"
    echo "  up       Start services"
    echo "  down     Stop services"
}

case "$1" in
    build)
        echo "Building..."
        ;;
    test)
        echo "Testing..."
        ;;
    up)
        echo "Starting services..."
        ;;
    down)
        echo "Stopping services..."
        ;;
    --help|-h)
        script_usage
        ;;
    *)
        echo "Unknown command: $1"
        script_usage
        exit 1
        ;;
esac
"#;
