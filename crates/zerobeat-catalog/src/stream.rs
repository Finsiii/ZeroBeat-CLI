#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioQuality {
    DataSaver,
    Balanced,
    High,
    #[default]
    Automatic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedStream {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub expires_at_epoch_seconds: Option<u64>,
}

impl ResolvedStream {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
            expires_at_epoch_seconds: None,
        }
    }
}
