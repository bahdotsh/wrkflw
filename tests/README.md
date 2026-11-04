# Testing

This directory holds test fixtures, test workflows and scripts for wrkflw.

## Test Organization

- **Unit tests**: alongside source files in `src/` using `#[cfg(test)]` modules
- **Integration tests**: in each crate's own `tests/` directory, mainly `crates/wrkflw/tests/`
  (acceleration, agent CLI, container fidelity, `continue-on-error`, dynamic matrices,
  GitLab pipelines, `needs` failure propagation, reusable workflow inputs and outputs,
  startup, and `--job` targeting)

The `.rs` files in this top-level directory (`matrix_test.rs`, `reusable_workflow_test.rs`,
`reusable_workflow_execution_test.rs`, `cleanup_test.rs`) are not compiled. The root
`Cargo.toml` is a virtual workspace, so no package owns them and `cargo test` never runs them.
Keep them as reference only, and add new tests under a crate's `tests/` directory.

## Directory Structure

- **`fixtures/`** — test data (e.g., `gitlab-ci/` configs)
- **`workflows/`** — GitHub Actions workflow YAML files for testing
- **`safe_workflow.yml`, `security_demo.yml`, `security_comparison.yml`** — workflows for comparing emulation with secure emulation
- **`scripts/`** — test automation scripts (`test-podman-basic.sh`, `test-preserve-containers.sh`)

## Running Tests

```bash
# All tests
cargo test

# Unit tests only
cargo test --lib

# Integration tests of the CLI crate only
cargo test -p wrkflw --tests

# One integration test file
cargo test -p wrkflw --test reusable_inputs_outputs_test

# A specific test
cargo test test_name
```
