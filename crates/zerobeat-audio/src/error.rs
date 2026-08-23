#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("audio backend is unavailable: {0}")]
    Unavailable(String),
    #[error("audio backend failed: {0}")]
    Failed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error("operation is invalid while player is {0}")]
    InvalidState(&'static str),
}
