# Wrkflw Crates

This directory contains the Rust crates that make up the Wrkflw project. The project has been restructured to use a workspace-based approach with individual crates for better modularity and maintainability.

## Crate Structure

- **wrkflw**: Main binary crate and entry point for the application
- **models**: Data models and structures used throughout the application
- **evaluator**: Workflow evaluation functionality
- **executor**: Workflow execution engine
- **github**: GitHub API integration
- **gitlab**: GitLab API integration
- **logging**: Logging functionality
- **matrix**: Matrix-based parallelization support
- **parser**: Workflow parsing functionality
- **runtime**: Runtime execution environment
- **ui**: User interface components
- **utils**: Utility functions
- **validators**: Validation functionality

## Dependencies

Each crate has its own `Cargo.toml` file that defines its dependencies. The root `Cargo.toml` file defines the workspace and shared dependencies.

## Build Instructions

To build the entire project:

```bash
cargo build
```

To build a specific crate:

```bash
cargo build -p <crate-name>
```

## Testing

To run tests for the entire project:

```bash
cargo test
```

To run tests for a specific crate:

```bash
cargo test -p <crate-name>
```

## Rust Best Practices

When contributing to wrkflw, please follow these Rust best practices:

### Code Organization

- Place modules in their respective crates to maintain separation of concerns
- Use `pub` selectively to expose only the necessary APIs
- Follow the Rust module system conventions (use `mod` and `pub mod` appropriately)

### Errors and Error Handling

- Prefer using the `thiserror` crate for defining custom error types
- Use the `?` operator for error propagation instead of match statements when appropriate
- Implement custom error types that provide context for the error
- Avoid using `.unwrap()` and `.expect()` in production code

### Performance

- Profile code before optimizing using tools like `cargo flamegraph`
- Use `Arc` and `Mutex` judiciously for shared mutable state
- Leverage Rust's zero-cost abstractions (iterators, closures)
- Consider adding benchmark tests using the `criterion` crate for performance-critical code

### Security

- Validate all input, especially from external sources
- Avoid using `unsafe` code unless absolutely necessary
- Handle secrets securely using environment variables
- Check for integer overflows with `checked_` operations

### Testing

- Write unit tests for all public functions
- Use integration tests to verify crate-to-crate interactions
- Consider property-based testing for complex logic
- Structure tests with clear preparation, execution, and verification phases

### Tooling

- Run `cargo clippy` before committing changes to catch common mistakes
- Use `cargo fmt` to maintain consistent code formatting
- Enable compiler warnings with `#![warn(clippy::all)]`

For more detailed guidance, refer to the project's best practices documentation. 