use std::path::PathBuf;

use url::Url;

use crate::ApiError;

#[derive(Clone)]
pub struct ApiConfig {
    pub(crate) base_url: Url,
    pub(crate) canonical_prefix: String,
    pub(crate) identity_path: PathBuf,
    pub(crate) app_version: String,
}

impl ApiConfig {
    pub fn new(
        base_url: impl AsRef<str>,
        canonical_prefix: impl Into<String>,
        identity_path: impl Into<PathBuf>,
        app_version: impl Into<String>,
    ) -> Result<Self, ApiError> {
        let base_url = Url::parse(base_url.as_ref())?;
        if base_url.host_str().is_none() {
            return Err(ApiError::InvalidBaseUrl);
        }
        Ok(Self {
            base_url,
            canonical_prefix: canonical_prefix.into().trim_end_matches('/').to_owned(),
            identity_path: identity_path.into(),
            app_version: app_version.into(),
        })
    }
}
