# makefilehub

A general-purpose MCP (Model Context Protocol) server for running tasks across different build systems.

**makefilehub** provides a unified interface for Claude Code to interact with:
- **Makefile** - GNU Make
- **justfile** - just command runner
- **Custom scripts** - run.sh, build.sh, etc.

## Features

- **Auto-detection** of build systems by configurable priority
- **XDG-compliant** layered configuration
- **Environment variable interpolation** (`$VAR`, `${VAR}`)
- **Shell command interpolation** (`$(command)`)
- **Service dependency management** for complex rebuild orchestration
- **MCP tools** for seamless Claude Code integration

## Installation

### From Source

```bash
cargo install --path .
```

### Using Cargo

```bash
cargo install makefilehub
```

## MCP Integration

Add makefilehub to Claude Code:

```bash
claude mcp add --transport stdio --scope user makefilehub makefilehub mcp
```

## CLI Usage

```bash
# Start MCP server (for Claude Code)
makefilehub mcp

# Run a task in the current directory
makefilehub run build
makefilehub run test

# Run a task in a specific project
makefilehub run build -p /path/to/project
makefilehub run build -p my-service  # uses configured service

# Force a specific runner
makefilehub run build -r just
makefilehub run build -r make
makefilehub run build -r ./run.sh

# Pass arguments
makefilehub run build -a TARGET=release -a DEBUG=0
makefilehub run test -- --verbose --filter pattern

# List available tasks
makefilehub list
makefilehub list -f json

# Detect build system
makefilehub detect
makefilehub detect -p /path/to/project

# Show project configuration
makefilehub config my-service

# Rebuild service with dependencies
makefilehub rebuild web-api
makefilehub rebuild web-api -s frontend --skip-deps
```

## MCP Tools

### run_task

Run a task/target in a project. Auto-detects build system.

```json
{
  "task": "build",
  "project": "/path/to/project",
  "runner": "make",
  "args": {"TARGET": "release"},
  "positional_args": ["--verbose"]
}
```

### list_tasks

List available tasks/targets in a project.

```json
{
  "project": "/path/to/project",
  "runner": "just"
}
```

### detect_runner

Detect which build system a project uses.

```json
{
  "project": "/path/to/project"
}
```

### get_project_config

Get resolved configuration for a project/service.

```json
{
  "project": "my-service"
}
```

### rebuild_service

Build service with dependency handling.

```json
{
  "service": "web-api",
  "services": ["frontend"],
  "skip_deps": false,
  "skip_recreate": false
}
```

## Configuration

Configuration files are loaded in order (lowest to highest priority):

1. `/etc/makefilehub/config.toml`
2. `~/.config/makefilehub/config.toml`
3. `~/.makefilehub.toml`
4. `./.makefilehub.toml` (project root)

### Example Configuration

```toml
[defaults]
runner_priority = ["make", "just", "script"]
default_script = "./run.sh"
timeout = 300

[defaults.task_aliases]
build = ["build", "compile"]
test = ["test", "check"]

[runners.make]
command = "make"

[runners.just]
command = "just"

[runners.script]
scripts = ["./run.sh", "./build.sh"]

[services.web-api]
project_dir = "$HOME/projects/web-api"
runner = "script"
depends_on = ["frontend"]
force_recreate = ["nginx"]

[services.web-api.tasks]
build = "build"
up = "up"
```

### Interpolation

- `$VAR` or `${VAR}` - Environment variables
- `$(command)` - Shell command execution

Example:
```toml
[services.dynamic]
project_dir = "${PROJECTS_DIR}/my-project"

[services.current]
project_dir = "$(pwd)/subproject"
```

## Build System Priority

When no explicit runner is configured, makefilehub auto-detects in this order:

1. **Makefile** - `Makefile`, `makefile`, `GNUmakefile`
2. **justfile** - `justfile`, `Justfile`, `.justfile`
3. **Script** - `./run.sh`, `./build.sh`, etc. (configurable)

## Development

```bash
# Install development tools
just setup

# Run tests
just test

# Run linter
just lint

# Run all checks
just check

# Build release
just release
```

## License

MIT
