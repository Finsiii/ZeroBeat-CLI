use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct Lyrics {
    pub synced: bool,
    pub lines: Vec<LyricsLine>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct LyricsLine {
    pub start_ms: Option<u64>,
    pub words: String,
}
