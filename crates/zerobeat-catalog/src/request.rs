use crate::CatalogError;

const MAX_SEARCH_RESULTS: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRequest {
    pub query: String,
    pub limit: usize,
}

impl SearchRequest {
    pub fn new(query: impl Into<String>, limit: usize) -> Result<Self, CatalogError> {
        let query = query.into().trim().to_owned();
        if query.is_empty() {
            return Err(CatalogError::InvalidQuery);
        }
        Ok(Self {
            query,
            limit: limit.clamp(1, MAX_SEARCH_RESULTS),
        })
    }
}
