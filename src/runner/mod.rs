//! Runner module for build system detection and execution
//!
//! Provides auto-detection and execution for:
//! - Makefile (make)
//! - justfile (just)
//! - Custom scripts (run.sh, build.sh, etc.)

pub mod detect;
pub mod justfile;
pub mod makefile;
pub mod script;
pub mod traits;

pub use detect::*;
pub use justfile::JustfileRunner;
pub use makefile::MakefileRunner;
pub use script::ScriptRunner;
pub use traits::*;
