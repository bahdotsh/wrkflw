# WRKFLW

WRKFLW is a command-line tool for validating and executing GitHub Actions workflows locally, without requiring a full GitHub environment. It helps developers test their workflows directly on their own machines before pushing changes to GitHub.

## Features

- **Validate Workflow Files**: Check for syntax errors and common mistakes in GitHub Actions workflow files
- **Execute Workflows Locally**: Run workflows directly on your machine using Docker or emulation
- **Dependency Resolution**: Automatically determines the correct order to run jobs based on their dependencies
- **Docker Integration**: Execute workflow steps in Docker containers for better isolation
- **Emulation Mode**: Run workflows without Docker by emulating the container environment
- **GitHub Context**: Provides GitHub-like environment variables to workflows
- **Action Support**: Supports GitHub Actions including `actions/checkout` and many common actions

## Installation

The recommended way to install `wrkflw` is using Rust's package manager, Cargo. Here are several methods:

### Using Cargo Install (Recommended)
```bash
cargo install wrkflw
```

### From Source

Clone the repository and build using Cargo:

```bash
git clone https://github.com/yourusername/wrkflw.git
cd wrkflw
cargo build --release
```

The compiled binary will be available at `target/release/wrkflw`.

## Usage

### Validating Workflow Files

```bash
# Validate all workflow files in the default location (.github/workflows)
wrkflw validate

# Validate a specific workflow file
wrkflw validate path/to/workflow.yml

# Validate workflows in a specific directory
wrkflw validate path/to/workflows
```

### Running Workflows Locally

```bash
# Run a workflow with Docker (default)
wrkflw run .github/workflows/ci.yml

# Run a workflow in emulation mode (without Docker)
wrkflw run --emulate .github/workflows/ci.yml

# Run with verbose output
wrkflw run --verbose .github/workflows/ci.yml
```

## Examples

### Validating a Workflow

```bash
$ wrkflw validate .github/workflows/rust.yml

Evaluating workflows in: .github/workflows/rust.yml
============================================================
✓ Valid: rust.yml
------------------------------------------------------------

Summary
============================================================
✓ 1 valid workflow file(s)

All workflows are valid! 🎉
```

### Running a Workflow

```bash
$ wrkflw run .github/workflows/rust.yml

Executing workflow: .github/workflows/rust.yml
============================================================
Runtime: Docker
------------------------------------------------------------

✓ Job succeeded: build

------------------------------------------------------------
  ✓ Checkout code
  ✓ Set up Rust
  ✓ Build
  ✓ Run tests

Summary
============================================================
✓ 1 job(s) succeeded

Workflow completed successfully! 🎉
```

## Requirements

- Rust 1.55 or later
- Docker (optional, for container-based execution)

## How It Works

WRKFLW parses your GitHub Actions workflow files and executes each job and step in the correct order. When using Docker mode, it creates containers that closely match GitHub's runner environments. In emulation mode, it runs commands directly on your system while still maintaining the workflow structure.

## Limitations

- Not all GitHub Actions features are fully supported
- Complex matrix builds may not work exactly as they do on GitHub
- Some actions that require specific GitHub environment features may not work correctly

## License

This project is licensed under the MIT License - see the LICENSE file for details.
