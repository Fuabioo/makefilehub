# makefilehub development justfile
#
# Usage: just <recipe>
# Run `just --list` to see available recipes

set shell := ["bash", "-c"]

# Default recipe - show help
default:
    @just --list

# Build the project
build:
    cargo build

# Build release version
release:
    cargo build --release

# Run all tests
test:
    cargo test

# Run tests with output
test-verbose:
    cargo test -- --nocapture

# Run tests matching a pattern
test-match pattern:
    cargo test {{pattern}}

# Run clippy linter
lint:
    cargo clippy -- -D warnings

# Format code
fmt:
    cargo fmt

# Check formatting
fmt-check:
    cargo fmt -- --check

# Run all checks (test + lint + fmt-check)
check: test lint fmt-check
    @echo "All checks passed!"

# Clean build artifacts
clean:
    cargo clean

# Run the CLI
run *args:
    cargo run -- {{args}}

# Run MCP server
mcp:
    cargo run -- mcp

# Run with verbose output
run-verbose *args:
    cargo run -- -v {{args}}

# Detect runner in current directory
detect:
    cargo run -- detect

# List tasks in current directory
list:
    cargo run -- list

# Build and install locally
install:
    cargo install --path .

# Uninstall
uninstall:
    cargo uninstall makefilehub

# Generate documentation
doc:
    cargo doc --no-deps --open

# Watch for changes and run tests
watch:
    cargo watch -x test

# Watch for changes and run clippy
watch-lint:
    cargo watch -x clippy

# Run benchmarks (if any)
bench:
    cargo bench

# Show dependency tree
deps:
    cargo tree

# Check for outdated dependencies
outdated:
    cargo outdated

# Update dependencies
update:
    cargo update

# Create a new release (requires version bump first)
tag version:
    git tag -a v{{version}} -m "Release v{{version}}"
    git push origin v{{version}}

# Run security audit
audit:
    cargo audit

# Show coverage report (requires cargo-tarpaulin)
coverage:
    cargo tarpaulin --out Html

# Profile the binary (requires cargo-flamegraph)
profile *args:
    cargo flamegraph -- {{args}}

# Check binary size
size:
    @cargo build --release 2>/dev/null
    @ls -lh target/release/makefilehub | awk '{print "Binary size:", $5}'

# Run the example config
example-config:
    @echo "Example config:"
    @cat config.example.toml 2>/dev/null || echo "No config.example.toml found"

# Development setup
setup:
    @echo "Installing development tools..."
    rustup component add clippy rustfmt
    cargo install cargo-watch cargo-audit cargo-outdated
    @echo "Setup complete!"
