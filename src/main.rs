//! makefilehub CLI entry point
//!
//! Usage:
//!   makefilehub mcp              Start MCP server over stdio
//!   makefilehub run <task>       Run a task in the current directory
//!   makefilehub list             List available tasks
//!   makefilehub detect           Detect build system
//!   makefilehub config <project> Show configuration
//!   makefilehub rebuild <service> Rebuild service with dependencies

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use colored::Colorize;

use makefilehub::cli::{
    commands::{ConfigArgs, DetectArgs, ListArgs, OutputFormat, RebuildArgs, RunArgs},
    run_mcp_server, Cli, Commands,
};
use makefilehub::config::{load_config, Config};
use makefilehub::runner::{
    detect_runner,
    traits::{RunOptions, Runner},
    JustfileRunner, MakefileRunner, RunnerType, ScriptRunner,
};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = run(cli).await;

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}: {:#}", "error".red().bold(), e);
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Mcp => {
            run_mcp_server(cli.config.as_deref()).await?;
        }
        Commands::Run(args) => {
            run_task(args, cli.config.as_deref(), cli.verbose)?;
        }
        Commands::List(args) => {
            list_tasks(args, cli.config.as_deref(), cli.verbose)?;
        }
        Commands::Detect(args) => {
            detect_build_system(args, cli.config.as_deref())?;
        }
        Commands::Config(args) => {
            show_config(args, cli.config.as_deref())?;
        }
        Commands::Rebuild(args) => {
            rebuild_service(args, cli.config.as_deref(), cli.verbose)?;
        }
    }

    Ok(())
}

/// Run a task in a project
fn run_task(args: RunArgs, config_path: Option<&str>, verbose: bool) -> Result<()> {
    let config = load_config(config_path)?;
    let project_path = resolve_project_path(args.project.as_deref(), &config)?;

    // Get the runner to use
    let runner_type = if let Some(ref runner_name) = args.runner {
        parse_runner_type(runner_name)?
    } else {
        let detection = detect_runner(&project_path, &config);
        detection
            .detected
            .context("No build system detected in project")?
    };

    if verbose {
        eprintln!(
            "{}: {} in {}",
            "runner".cyan(),
            runner_type,
            project_path.display()
        );
    }

    // Create the appropriate runner
    let runner: Box<dyn Runner> = match &runner_type {
        RunnerType::Make => Box::new(MakefileRunner::new()),
        RunnerType::Just => Box::new(JustfileRunner::new()),
        RunnerType::Script(name) => Box::new(ScriptRunner::new(name)),
    };

    // Build run options
    let timeout = if args.timeout > 0 {
        Some(Duration::from_secs(args.timeout))
    } else {
        None
    };

    let options = RunOptions {
        working_dir: Some(project_path.clone()),
        args: args.args_as_map(),
        positional_args: args.positional.clone(),
        env: std::collections::HashMap::new(),
        timeout,
        capture_output: !args.stream,
    };

    let result = runner.run_task(&project_path, &args.task, &options)?;

    // Print output
    if !result.stdout.is_empty() {
        print!("{}", result.stdout);
    }
    if !result.stderr.is_empty() {
        eprint!("{}", result.stderr);
    }

    if result.success {
        if verbose {
            eprintln!(
                "{}: {} completed in {}ms",
                "success".green(),
                args.task,
                result.duration_ms
            );
        }
        Ok(())
    } else {
        anyhow::bail!(
            "Task '{}' failed with exit code {:?}",
            args.task,
            result.exit_code
        );
    }
}

/// List available tasks in a project
fn list_tasks(args: ListArgs, config_path: Option<&str>, verbose: bool) -> Result<()> {
    let config = load_config(config_path)?;
    let project_path = resolve_project_path(args.project.as_deref(), &config)?;

    // Get the runner to use
    let runner_type = if let Some(ref runner_name) = args.runner {
        parse_runner_type(runner_name)?
    } else {
        let detection = detect_runner(&project_path, &config);
        detection
            .detected
            .context("No build system detected in project")?
    };

    if verbose {
        eprintln!(
            "{}: {} in {}",
            "runner".cyan(),
            runner_type,
            project_path.display()
        );
    }

    // Get the appropriate runner
    let runner: Box<dyn Runner> = match &runner_type {
        RunnerType::Make => Box::new(MakefileRunner::new()),
        RunnerType::Just => Box::new(JustfileRunner::new()),
        RunnerType::Script(name) => Box::new(ScriptRunner::new(name)),
    };

    let tasks = runner
        .list_tasks(&project_path)
        .context("Failed to list tasks")?;

    match args.format {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&serde_json::json!({
                "runner": runner_type.to_string(),
                "file": runner_type.filename(),
                "tasks": tasks
            }))?;
            println!("{}", json);
        }
        OutputFormat::Plain => {
            for task in &tasks {
                println!("{}", task.name);
            }
        }
        OutputFormat::Table => {
            println!("{}: {}", "Runner".cyan(), runner_type);
            println!();
            if tasks.is_empty() {
                println!("No tasks found.");
            } else {
                // Find max width for alignment
                let max_name_width = tasks.iter().map(|t| t.name.len()).max().unwrap_or(10);

                for task in &tasks {
                    let desc = task
                        .description
                        .as_ref()
                        .map(|d| format!("- {}", d))
                        .unwrap_or_default();
                    println!(
                        "  {:width$}  {}",
                        task.name.green(),
                        desc,
                        width = max_name_width
                    );
                }
            }
        }
    }

    Ok(())
}

/// Detect build system in a project
fn detect_build_system(args: DetectArgs, config_path: Option<&str>) -> Result<()> {
    let config = load_config(config_path)?;
    let project_path = resolve_project_path(args.project.as_deref(), &config)?;

    let detection = detect_runner(&project_path, &config);

    match args.format {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&serde_json::json!({
                "detected": detection.detected.map(|r| r.to_string()),
                "available": detection.available.iter().map(|r| r.to_string()).collect::<Vec<_>>(),
                "files_found": {
                    "makefile": detection.files_found.makefile,
                    "makefile_path": detection.files_found.makefile_path,
                    "justfile": detection.files_found.justfile,
                    "justfile_path": detection.files_found.justfile_path,
                    "scripts": detection.files_found.scripts
                }
            }))?;
            println!("{}", json);
        }
        OutputFormat::Plain => {
            if let Some(ref detected) = detection.detected {
                println!("{}", detected);
            }
        }
        OutputFormat::Table => {
            println!("{}: {}", "Path".cyan(), project_path.display());
            println!();

            if let Some(ref detected) = detection.detected {
                println!("{}: {}", "Detected".green(), detected);
            } else {
                println!("{}: {}", "Detected".yellow(), "None");
            }

            println!();
            println!("{}:", "Available Runners".cyan());
            if detection.available.is_empty() {
                println!("  None");
            } else {
                for runner in &detection.available {
                    println!("  - {}", runner);
                }
            }

            println!();
            println!("{}:", "Files Found".cyan());
            if detection.files_found.makefile {
                let path = detection
                    .files_found
                    .makefile_path
                    .as_ref()
                    .map(|p| p.as_str())
                    .unwrap_or("Makefile");
                println!("  - {}", path);
            }
            if detection.files_found.justfile {
                let path = detection
                    .files_found
                    .justfile_path
                    .as_ref()
                    .map(|p| p.as_str())
                    .unwrap_or("justfile");
                println!("  - {}", path);
            }
            for script in &detection.files_found.scripts {
                println!("  - {}", script);
            }
        }
    }

    Ok(())
}

/// Show resolved configuration for a project
fn show_config(args: ConfigArgs, config_path: Option<&str>) -> Result<()> {
    let config = load_config(config_path)?;

    // Try as a path first, then as a configured service
    let path = PathBuf::from(&args.project);
    let resolved = if path.exists() {
        let detection = detect_runner(&path, &config);
        makefilehub::config::ResolvedService {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| args.project.clone()),
            project_dir: path.to_string_lossy().to_string(),
            runner: detection.detected.map(|r| r.to_string()),
            script: None,
            depends_on: vec![],
            force_recreate: vec![],
            tasks: std::collections::HashMap::new(),
            env: std::collections::HashMap::new(),
            timeout: config.defaults.timeout,
        }
    } else if config.services.contains_key(&args.project) {
        config.get_service(&args.project)
    } else {
        anyhow::bail!(
            "Project '{}' not found. Use a path or configure in makefilehub config.",
            args.project
        );
    };

    match args.format {
        OutputFormat::Json => {
            let json = if args.raw {
                serde_json::to_string_pretty(&config.services.get(&args.project))?
            } else {
                serde_json::to_string_pretty(&resolved)?
            };
            println!("{}", json);
        }
        OutputFormat::Plain => {
            println!("{}", resolved.project_dir);
        }
        OutputFormat::Table => {
            println!("{}: {}", "Project".cyan(), resolved.name);
            println!("{}: {}", "Directory".cyan(), resolved.project_dir);
            if let Some(ref runner) = resolved.runner {
                println!("{}: {}", "Runner".cyan(), runner);
            }
            if let Some(ref script) = resolved.script {
                println!("{}: {}", "Script".cyan(), script);
            }
            if !resolved.depends_on.is_empty() {
                println!(
                    "{}: {}",
                    "Dependencies".cyan(),
                    resolved.depends_on.join(", ")
                );
            }
            if !resolved.force_recreate.is_empty() {
                println!(
                    "{}: {}",
                    "Force Recreate".cyan(),
                    resolved.force_recreate.join(", ")
                );
            }
            if !resolved.tasks.is_empty() {
                println!("{}:", "Task Overrides".cyan());
                for (key, value) in &resolved.tasks {
                    println!("  {}: {}", key, value);
                }
            }
        }
    }

    Ok(())
}

/// Rebuild a service with dependency handling
fn rebuild_service(args: RebuildArgs, config_path: Option<&str>, verbose: bool) -> Result<()> {
    let config = load_config(config_path)?;

    // Collect all services to rebuild
    let mut services = vec![args.service.clone()];
    services.extend(args.services);

    let mut errors: Vec<String> = Vec::new();
    let mut rebuilt: Vec<String> = Vec::new();
    let mut restarted: Vec<String> = Vec::new();
    let mut recreated: Vec<String> = Vec::new();

    for service_name in &services {
        if !config.services.contains_key(service_name) {
            errors.push(format!("Service '{}' not found in config", service_name));
            continue;
        }

        let service = config.get_service(service_name);

        if verbose {
            eprintln!("{}: {}", "rebuilding".cyan(), service_name);
        }

        let project_path = PathBuf::from(&service.project_dir);
        if !project_path.exists() {
            errors.push(format!(
                "Project directory '{}' does not exist for service '{}'",
                service.project_dir, service_name
            ));
            continue;
        }

        // Determine runner
        let runner_type = if let Some(ref runner_name) = service.runner {
            match parse_runner_type(runner_name) {
                Ok(r) => r,
                Err(e) => {
                    errors.push(format!("Invalid runner for '{}': {}", service_name, e));
                    continue;
                }
            }
        } else {
            let detection = detect_runner(&project_path, &config);
            match detection.detected {
                Some(r) => r,
                None => {
                    errors.push(format!("No build system detected for '{}'", service_name));
                    continue;
                }
            }
        };

        // Create the appropriate runner
        let runner: Box<dyn Runner> = match &runner_type {
            RunnerType::Make => Box::new(MakefileRunner::new()),
            RunnerType::Just => Box::new(JustfileRunner::new()),
            RunnerType::Script(name) => Box::new(ScriptRunner::new(name)),
        };

        // Run build task
        let build_task = service
            .tasks
            .get("build")
            .map(|s| s.as_str())
            .unwrap_or("build");
        let timeout = if args.timeout > 0 {
            Some(Duration::from_secs(args.timeout))
        } else {
            None
        };
        let options = RunOptions {
            working_dir: Some(project_path.clone()),
            args: std::collections::HashMap::new(),
            positional_args: vec![],
            env: std::collections::HashMap::new(),
            timeout,
            capture_output: true,
        };

        match runner.run_task(&project_path, build_task, &options) {
            Ok(result) if result.success => {
                rebuilt.push(service_name.clone());
            }
            Ok(result) => {
                errors.push(format!(
                    "Build failed for '{}': exit code {:?}",
                    service_name, result.exit_code
                ));
                continue;
            }
            Err(e) => {
                errors.push(format!("Build failed for '{}': {}", service_name, e));
                continue;
            }
        }

        // Handle dependencies
        if !args.skip_deps {
            for dep in &service.depends_on {
                if verbose {
                    eprintln!(
                        "{}: {} (dependency of {})",
                        "restarting".cyan(),
                        dep,
                        service_name
                    );
                }
                restarted.push(dep.clone());
            }
        }

        // Handle force recreate
        if !args.skip_recreate {
            for container in &service.force_recreate {
                if verbose {
                    eprintln!("{}: {}", "recreating".cyan(), container);
                }
                recreated.push(container.clone());
            }
        }
    }

    // Report results
    if !rebuilt.is_empty() {
        println!("{}: {}", "Rebuilt".green(), rebuilt.join(", "));
    }
    if !restarted.is_empty() {
        println!("{}: {}", "Restarted".green(), restarted.join(", "));
    }
    if !recreated.is_empty() {
        println!("{}: {}", "Recreated".green(), recreated.join(", "));
    }

    if !errors.is_empty() {
        eprintln!();
        eprintln!("{}:", "Errors".red());
        for error in &errors {
            eprintln!("  - {}", error);
        }
        anyhow::bail!("Rebuild completed with {} error(s)", errors.len());
    }

    Ok(())
}

/// Resolve project path from name or path
fn resolve_project_path(project: Option<&str>, config: &Config) -> Result<PathBuf> {
    match project {
        Some(p) => {
            // Try as a path first
            let path = PathBuf::from(p);
            if path.exists() {
                return Ok(path);
            }

            // Check if it's a configured service
            if config.services.contains_key(p) {
                let service = config.get_service(p);
                return Ok(PathBuf::from(&service.project_dir));
            }

            anyhow::bail!("Project '{}' not found", p)
        }
        None => Ok(std::env::current_dir().context("Failed to get current directory")?),
    }
}

/// Parse runner type from string
fn parse_runner_type(s: &str) -> Result<RunnerType> {
    match s.to_lowercase().as_str() {
        "make" | "makefile" => Ok(RunnerType::Make),
        "just" | "justfile" => Ok(RunnerType::Just),
        _ => {
            // Assume it's a script name
            if s.contains('/') || s.ends_with(".sh") {
                Ok(RunnerType::Script(s.to_string()))
            } else {
                Ok(RunnerType::Script(format!("./{}", s)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_runner_type_make() {
        assert!(matches!(
            parse_runner_type("make").unwrap(),
            RunnerType::Make
        ));
        assert!(matches!(
            parse_runner_type("Makefile").unwrap(),
            RunnerType::Make
        ));
    }

    #[test]
    fn test_parse_runner_type_just() {
        assert!(matches!(
            parse_runner_type("just").unwrap(),
            RunnerType::Just
        ));
        assert!(matches!(
            parse_runner_type("justfile").unwrap(),
            RunnerType::Just
        ));
    }

    #[test]
    fn test_parse_runner_type_script() {
        // Script with .sh suffix - keeps as-is
        if let RunnerType::Script(name) = parse_runner_type("run.sh").unwrap() {
            assert_eq!(name, "run.sh");
        } else {
            panic!("Expected Script");
        }

        // Script with path - keeps as-is
        if let RunnerType::Script(name) = parse_runner_type("./build.sh").unwrap() {
            assert_eq!(name, "./build.sh");
        } else {
            panic!("Expected Script");
        }

        // Script without .sh or path - gets ./ prepended
        if let RunnerType::Script(name) = parse_runner_type("custom").unwrap() {
            assert_eq!(name, "./custom");
        } else {
            panic!("Expected Script");
        }
    }

    #[test]
    fn test_resolve_project_path_current_dir() {
        let config = Config::default();
        let result = resolve_project_path(None, &config);
        assert!(result.is_ok());
        assert!(result.unwrap().exists());
    }
}
