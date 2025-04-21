# WRKFLW

[![Crates.io](https://img.shields.io/crates/v/wrkflw)](https://crates.io/crates/wrkflw)
[![Rust Version](https://img.shields.io/badge/rust-1.67%2B-orange)](https://www.rust-lang.org/)
[![License](https://img.shields.io/crates/l/wrkflw)](LICENSE)
[![Build Status](https://img.shields.io/github/actions/workflow/status/bahdotsh/wrkflw/build.yml?branch=main)](https://github.com/bahdotsh/wrkflw/actions/workflows/build.yml)
[![Downloads](https://img.shields.io/crates/d/wrkflw)](https://crates.io/crates/wrkflw)

WRKFLW is a powerful command-line tool for validating and executing GitHub Actions workflows locally, without requiring a full GitHub environment. It helps developers test their workflows directly on their machines before pushing changes to GitHub.

![WRKFLW Demo](demo.gif)

## Features

- **TUI Interface**: A full-featured terminal user interface for managing and monitoring workflow executions
- **Validate Workflow Files**: Check for syntax errors and common mistakes in GitHub Actions workflow files
- **Execute Workflows Locally**: Run workflows directly on your machine using Docker containers
- **Emulation Mode**: Optional execution without Docker by emulating the container environment locally
- **Job Dependency Resolution**: Automatically determines the correct execution order based on job dependencies
- **Docker Integration**: Execute workflow steps in isolated Docker containers with proper environment setup
- **GitHub Context**: Provides GitHub-like environment variables and workflow commands
- **Multiple Runtime Modes**: Choose between Docker containers or local emulation for maximum flexibility
- **Action Support**: Supports various GitHub Actions types:
  - Docker container actions
  - JavaScript actions
  - Composite actions
  - Local actions
- **Special Action Handling**: Native handling for commonly used actions like `actions/checkout`
- **Output Capturing**: View logs, step outputs, and execution details
- **Parallel Job Execution**: Runs independent jobs in parallel for faster workflow execution
- **Trigger Workflows Remotely**: Manually trigger workflow runs on GitHub

## Installation

The recommended way to install `wrkflw` is using Rust's package manager, Cargo:

### Using Cargo Install (Recommended)
```bash
cargo install wrkflw
```

### From Source

Clone the repository and build using Cargo:

```bash
git clone https://github.com/bahdotsh/wrkflw.git
cd wrkflw
cargo build --release
```

The compiled binary will be available at `target/release/wrkflw`.

## Usage

The simplest way to use WRKFLW is to navigate to your project's root directory and run:

```bash
wrkflw
```

This will automatically detect and load all workflows from `.github/workflows` directory into the TUI interface.

WRKFLW also provides three main command modes:

### Validating Workflow Files

```bash
# Validate all workflow files in the default location (.github/workflows)
wrkflw validate

# Validate a specific workflow file
wrkflw validate path/to/workflow.yml

# Validate workflows in a specific directory
wrkflw validate path/to/workflows

# Validate with verbose output
wrkflw validate --verbose path/to/workflow.yml
```

### Running Workflows in CLI Mode

```bash
# Run a workflow with Docker (default)
wrkflw run .github/workflows/ci.yml

# Run a workflow in emulation mode (without Docker)
wrkflw run --emulate .github/workflows/ci.yml

# Run with verbose output
wrkflw run --verbose .github/workflows/ci.yml
```

### Using the TUI Interface

```bash
# Open TUI with workflows from the default directory
wrkflw tui

# Open TUI with a specific directory of workflows
wrkflw tui path/to/workflows

# Open TUI with a specific workflow pre-selected
wrkflw tui path/to/workflow.yml

# Open TUI in emulation mode
wrkflw tui --emulate
```

### Triggering Workflows Remotely

```bash
# Trigger a workflow remotely on GitHub
wrkflw trigger workflow-name --branch main --input key1=value1 --input key2=value2
```

## TUI Controls

The terminal user interface provides an interactive way to manage workflows:

- **Tab / 1-4**: Switch between tabs (Workflows, Execution, Logs, Help)
- **Up/Down or j/k**: Navigate lists
- **Space**: Toggle workflow selection
- **Enter**: Run selected workflow / View job details
- **r**: Run all selected workflows
- **a**: Select all workflows
- **n**: Deselect all workflows
- **e**: Toggle between Docker and Emulation mode
- **v**: Toggle between Execution and Validation mode
- **Esc**: Back / Exit detailed view
- **q**: Quit application

## Examples

### Validating a Workflow

```bash
$ wrkflw validate .github/workflows/rust.yml

Validating workflows in: .github/workflows/rust.yml
============================================================
✅ Valid: rust.yml
------------------------------------------------------------

Summary
============================================================
✅ 1 valid workflow file(s)

All workflows are valid! 🎉
```

### Running a Workflow

```bash
$ wrkflw run .github/workflows/rust.yml

Executing workflow: .github/workflows/rust.yml
============================================================
Runtime: Docker
------------------------------------------------------------

✅ Job succeeded: build

------------------------------------------------------------
  ✅ Checkout code
  ✅ Set up Rust
  ✅ Build
  ✅ Run tests

✅ Workflow completed successfully!
```

### Quick TUI Startup

```bash
# Navigate to project root and run wrkflw
$ cd my-project
$ wrkflw

# This will automatically load .github/workflows files into the TUI
```

## Requirements

- Rust 1.67 or later
- Docker (optional, for container-based execution)
  - When not using Docker, the emulation mode can run workflows using your local system tools

## How It Works

WRKFLW parses your GitHub Actions workflow files and executes each job and step in the correct order. For Docker mode, it creates containers that closely match GitHub's runner environments. The workflow execution process:

1. **Parsing**: Reads and validates the workflow YAML structure
2. **Dependency Resolution**: Creates an execution plan based on job dependencies
3. **Environment Setup**: Prepares GitHub-like environment variables and context
4. **Execution**: Runs each job and step either in Docker containers or through local emulation
5. **Monitoring**: Tracks progress and captures outputs in the TUI or command line

## Advanced Features

### GitHub Environment Files Support

WRKFLW supports GitHub's environment files and special commands:

- `GITHUB_OUTPUT`: For storing step outputs (`echo "result=value" >> $GITHUB_OUTPUT`)
- `GITHUB_ENV`: For setting environment variables (`echo "VAR=value" >> $GITHUB_ENV`)
- `GITHUB_PATH`: For modifying the PATH (`echo "/path/to/dir" >> $GITHUB_PATH`)
- `GITHUB_STEP_SUMMARY`: For creating step summaries (`echo "# Summary" >> $GITHUB_STEP_SUMMARY`)

### Composite Actions

WRKFLW supports composite actions, which are actions made up of multiple steps. This includes:

- Local composite actions referenced with `./path/to/action`
- Remote composite actions from GitHub repositories
- Nested composite actions (composite actions that use other actions)

### Container Cleanup

WRKFLW automatically cleans up any Docker containers created during workflow execution, even if the process is interrupted with Ctrl+C.

## Limitations

- Some GitHub-specific functionality might not work exactly as it does on GitHub
- Complex matrix builds with very large matrices may have performance limitations
- Actions that require specific GitHub environment features may need customization
- Network-isolated actions might need internet connectivity configured differently

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Remote Workflow Triggering

WRKFLW allows you to manually trigger workflow runs on GitHub through both the command-line interface (CLI) and the terminal user interface (TUI).

### Requirements:

1. You need a GitHub token with workflow permissions. Set it in the `GITHUB_TOKEN` environment variable:
   ```bash
   export GITHUB_TOKEN=ghp_your_token_here
   ```

2. The workflow must have the `workflow_dispatch` trigger defined in your workflow YAML:
   ```yaml
   on:
     workflow_dispatch:
       inputs:
         name:
           description: 'Person to greet'
           default: 'World'
           required: true
         debug:
           description: 'Enable debug mode'
           required: false
           type: boolean
           default: false
   ```

### Triggering from CLI:

```bash
# Trigger a workflow using the default branch
wrkflw trigger workflow-name

# Trigger a workflow on a specific branch
wrkflw trigger workflow-name --branch feature-branch

# Trigger with input parameters
wrkflw trigger workflow-name --branch main --input name=Alice --input debug=true
```

After triggering, WRKFLW will provide feedback including the URL to view the triggered workflow on GitHub.

### Triggering from TUI:

1. Launch the TUI interface:
   ```bash
   wrkflw tui
   ```

2. Navigate to the "Workflows" tab (use `Tab` key or press `1`).

3. Use the arrow keys (`↑`/`↓`) or `j`/`k` to select the desired workflow.

4. Press `t` to trigger the selected workflow.

5. If the workflow is successfully triggered, you'll see a notification in the UI.

6. You can monitor the triggered workflow's execution on GitHub using the provided URL.

### Verifying Triggered Workflows:

To verify that your workflow was triggered:

1. Visit your GitHub repository in a web browser.
2. Navigate to the "Actions" tab.
3. Look for your workflow in the list of workflow runs.
4. Click on it to view the details of the run.
