# Wrkflw Crates

This directory contains the Rust crates that make up the wrkflw workspace.

## Crate Structure

| Crate | Purpose |
|-------|---------|
| **wrkflw** | CLI binary and library entry point |
| **executor** | Workflow execution engine (Docker, Podman, emulation) |
| **parser** | Workflow file parsing and JSON Schema validation |
| **evaluator** | Structural evaluation of workflow files |
| **validators** | Validation rules for jobs, steps, triggers, matrix |
| **runtime** | Container management and emulation runtime |
| **ui** | Terminal user interface (ratatui-based) |
| **models** | Shared data structures (`ValidationResult`, GitLab models) |
| **matrix** | Matrix expansion (`include`, `exclude`, `fail-fast`) |
| **secrets** | Secrets management with multiple providers and encryption |
| **github** | GitHub API integration (list/trigger workflows) |
| **gitlab** | GitLab API integration (trigger pipelines) |
| **logging** | Thread-safe in-memory logging for TUI/CLI |
| **utils** | Workflow file detection and fd redirection helpers |

## Building

```bash
# Build everything
cargo build

# Build a specific crate
cargo build -p wrkflw-executor
```

## Testing

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p wrkflw-executor
```

Each crate has its own `Cargo.toml` with dependencies managed through workspace inheritance in the root `Cargo.toml`.
