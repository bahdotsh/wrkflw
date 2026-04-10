use thiserror::Error;

#[derive(Debug, Error)]
pub enum TriggerFilterError {
    #[error("Failed to parse trigger config: {0}")]
    ParseError(String),

    #[error("Git error: {0}")]
    GitError(String),
}
