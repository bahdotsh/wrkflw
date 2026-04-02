# Testing

This directory contains integration and end-to-end tests for wrkflw.

## Test Organization

- **Unit tests**: alongside source files in `src/` using `#[cfg(test)]` modules
- **Integration tests**: in this `tests/` directory
  - `matrix_test.rs` — matrix expansion
  - `reusable_workflow_test.rs` — reusable workflow validation
- **End-to-end tests**: also in this directory
  - `cleanup_test.rs` — cleanup with Docker resources

## Directory Structure

- **`fixtures/`** — test data (e.g., `gitlab-ci/` configs)
- **`workflows/`** — GitHub Actions workflow YAML files for testing
- **`scripts/`** — test automation scripts (`test-podman-basic.sh`, `test-preserve-containers.sh`)

## Running Tests

```bash
# All tests
cargo test

# Unit tests only
cargo test --lib

# Integration tests only
cargo test --test matrix_test --test reusable_workflow_test

# End-to-end tests only
cargo test --test cleanup_test

# A specific test
cargo test test_name
```
