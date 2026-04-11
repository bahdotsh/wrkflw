pub mod debouncer;
pub mod error;
pub mod shutdown;
pub mod watcher;

// Internal modules extracted from the historical single-file
// `watcher.rs`. Keeping them private (`pub(crate)` items inside) means
// the public surface of the crate is exactly what's re-exported below —
// embedders don't accidentally depend on helpers that are meant to
// stay refactor-friendly.
pub(crate) mod event_kind;
pub(crate) mod ignore;
pub(crate) mod paths;
pub(crate) mod setup;
pub(crate) mod trigger_cache;

pub use error::WatchError;
pub use shutdown::ShutdownSignal;
pub use watcher::{WatchEvent, WatcherConfig, WorkflowWatcher, DEFAULT_MAX_CONCURRENT_EXECUTIONS};
