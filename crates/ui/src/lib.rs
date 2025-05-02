// Modular UI crate for wrkflw
//
// This crate is organized into several modules:
// - app: Contains the main App state and TUI entry point
// - models: Contains the data structures for the UI
// - components: Contains reusable UI elements
// - handlers: Contains workflow handling logic
// - utils: Contains utility functions
// - views: Contains UI rendering code

// Re-export public modules
pub mod app;
pub mod components;
pub mod handlers;
pub mod models;
pub mod utils;
pub mod views;

// Re-export main entry points
pub use app::run_wrkflw_tui;
pub use handlers::workflow::execute_workflow_cli;
pub use handlers::workflow::validate_workflow;
