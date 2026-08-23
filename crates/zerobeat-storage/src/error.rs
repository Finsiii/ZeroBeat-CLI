#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("invalid download state: {0}")]
    InvalidDownloadState(String),
    #[error("duration is outside SQLite integer range")]
    DurationOutOfRange,
}
