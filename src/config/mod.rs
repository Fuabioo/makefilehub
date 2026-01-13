//! Configuration module for makefilehub
//!
//! Provides XDG-compliant layered configuration loading with
//! environment variable and shell command interpolation.

pub mod interpolate;
pub mod loader;
pub mod model;

pub use interpolate::interpolate_config;
pub use loader::{config_paths, find_config_files, load_config};
pub use model::*;
