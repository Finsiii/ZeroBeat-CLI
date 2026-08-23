#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("search query cannot be blank")]
    InvalidQuery,
    #[error("catalog authorization is required")]
    Unauthorized,
    #[error("catalog is unavailable: {0}")]
    Unavailable(String),
    #[error("catalog returned invalid data: {0}")]
    InvalidResponse(String),
}
