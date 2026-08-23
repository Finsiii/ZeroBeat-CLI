use zerobeat_core::Track;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamSource {
    pub url: String,
}

impl StreamSource {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
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
