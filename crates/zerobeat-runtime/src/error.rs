use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("runtime directory I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("runtime path is not a private directory: {0}")]
    UnsafeDirectory(PathBuf),
    #[error("cannot locate the user data directory")]
    MissingHomeDirectory,
}
