# Test Organization for wrkflw

Following Rust best practices, we have reorganized the tests in this project to improve maintainability and clarity.

## Test Structure

Tests are now organized as follows:

### 1. Unit Tests

Unit tests remain in the source files using the `#[cfg(test)]` attribute. These tests are designed to test individual functions and small units of code in isolation.

Example:
```rust
// In src/matrix.rs
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_function() {
        // Test code here
    }
}
```

### 2. Integration Tests

Integration tests have been moved to the `tests/` directory. These tests import and test the public API of the crate, ensuring that different components work together correctly.

- `tests/matrix_test.rs` - Tests for matrix expansion functionality
- `tests/reusable_workflow_test.rs` - Tests for reusable workflow validation

### 3. End-to-End Tests

End-to-end tests are also located in the `tests/` directory. These tests simulate real-world usage scenarios and often involve external dependencies like Docker.

- `tests/cleanup_test.rs` - Tests for cleanup functionality with Docker containers, networks, etc.

## Running Tests

You can run all tests using:
```bash
cargo test
```

To run only unit tests:
```bash
cargo test --lib
```

To run only integration tests:
```bash
cargo test --test matrix_test --test reusable_workflow_test
```

To run only end-to-end tests:
```bash
cargo test --test cleanup_test
```

To run a specific test:
```bash
cargo test test_name
```

## CI Configuration

Our CI workflow has been updated to run all types of tests separately, allowing for better isolation and clearer failure reporting:

```yaml
- name: Run unit tests
  run: cargo test --lib --verbose

- name: Run integration tests
  run: cargo test --test matrix_test --test reusable_workflow_test --verbose

- name: Run e2e tests (if Docker available)
  run: cargo test --test cleanup_test --verbose -- --skip docker --skip processes
```

## Writing New Tests

When adding new tests:

1. For unit tests, add them to the relevant source file using `#[cfg(test)]`
2. For integration tests, add them to the `tests/` directory with a descriptive name like `feature_name_test.rs`
3. For end-to-end tests, also add them to the `tests/` directory with a descriptive name

Follow the existing patterns to ensure consistency. 