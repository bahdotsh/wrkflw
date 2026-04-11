use thiserror::Error;

#[derive(Debug, Error)]
pub enum WatchError {
    #[error("Watcher error: {0}")]
    Notify(#[from] notify::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Execution error: {0}")]
    Execution(String),
}
