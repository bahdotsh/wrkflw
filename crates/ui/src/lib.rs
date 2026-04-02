// Modular UI crate for wrkflw
//
// This crate is organized into several modules:
// - app: Contains the main App state and TUI entry point
// - models: Contains the data structures for the UI
// - components: Contains reusable UI elements
// - handlers: Contains workflow handling logic
// - utils: Contains utility functions
// - views: Contains UI rendering code

// Always-available modules (CLI validation/execution)
pub mod cli_style;
pub mod handlers;

// TUI-specific modules (require ratatui/crossterm)
#[cfg(feature = "tui")]
pub mod app;
#[cfg(feature = "tui")]
pub mod components;
#[cfg(feature = "tui")]
pub mod log_processor;
#[cfg(feature = "tui")]
pub mod models;
#[cfg(feature = "tui")]
pub mod theme;
#[cfg(feature = "tui")]
pub mod utils;
#[cfg(feature = "tui")]
pub mod views;

// Re-export main entry points
#[cfg(feature = "tui")]
pub use app::run_wrkflw_tui;
pub use handlers::workflow::execute_workflow_cli;
pub use handlers::workflow::validate_workflow;
