use zerobeat_core::Track;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamSource {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

impl StreamSource {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            headers: Vec::new(),
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueItem {
    pub track: Track,
    pub source: StreamSource,
}

impl QueueItem {
    pub fn new(track: Track, source: StreamSource) -> Self {
        Self { track, source }
    }
}
