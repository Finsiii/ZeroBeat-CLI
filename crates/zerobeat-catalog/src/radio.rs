use zerobeat_core::Track;

const MAX_RADIO_RESULTS: usize = 50;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RadioRequest {
    pub seed_track_id: String,
    pub continuation: Option<String>,
    pub limit: usize,
}

impl RadioRequest {
    pub fn from_seed(seed_track_id: impl Into<String>, limit: usize) -> Self {
        Self {
            seed_track_id: seed_track_id.into(),
            continuation: None,
            limit: limit.clamp(1, MAX_RADIO_RESULTS),
        }
    }

    pub fn from_continuation(
        seed_track_id: impl Into<String>,
        continuation: impl Into<String>,
        limit: usize,
    ) -> Self {
        Self {
            seed_track_id: seed_track_id.into(),
            continuation: Some(continuation.into()),
            limit: limit.clamp(1, MAX_RADIO_RESULTS),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RadioPage {
    pub tracks: Vec<Track>,
    pub continuation: Option<String>,
}
