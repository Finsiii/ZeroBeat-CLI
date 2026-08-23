#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("invalid API base URL")]
    InvalidBaseUrl,
    #[error("API URL failed: {0}")]
    Url(#[from] url::ParseError),
    #[error("API transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("device security failed: {0}")]
    Security(#[from] zerobeat_security::SecurityError),
    #[error("API rejected the request with status {status}: {message}")]
    Rejected { status: u16, message: String },
    #[error("API response was incomplete: {0}")]
    InvalidResponse(&'static str),
    #[error("API returned invalid JSON: {0}")]
    InvalidJson(String),
    #[error("system clock is before Unix epoch")]
    InvalidClock,
}
