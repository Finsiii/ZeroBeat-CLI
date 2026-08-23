use serde::{Deserialize, Serialize};
use zerobeat_core::{NavigationState, Route, SessionMode, Track};

pub const PROTOCOL_VERSION: u16 = 3;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppSnapshot {
    pub session: SessionMode,
    pub navigation: NavigationState,
    pub search: SearchSnapshot,
    pub playback: PlaybackSnapshot,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SearchSnapshot {
    pub status: SearchStatus,
    pub results: Vec<Track>,
    pub selected_index: usize,
    pub request_id: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum SearchStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlaybackSnapshot {
    pub status: PlaybackStatus,
    pub current: Option<Track>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub buffered_ms: u64,
    pub volume_percent: u8,
    pub error: Option<String>,
    pub request_id: u64,
}

impl Default for PlaybackSnapshot {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Idle,
            current: None,
            position_ms: 0,
            duration_ms: 0,
            buffered_ms: 0,
            volume_percent: 100,
            error: None,
            request_id: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum PlaybackStatus {
    #[default]
    Idle,
    Resolving,
    Buffering,
    Playing,
    Paused,
    Ended,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ClientCommand {
    Hello { protocol_version: u16 },
    Navigate(Route),
    Back,
    UpdateSearch(String),
    SubmitSearch,
    SelectNext,
    SelectPrevious,
    PlaySelected,
    TogglePlayback,
    NextTrack,
    SeekRelative(i64),
    SetVolume(u8),
    RequestSnapshot,
    Shutdown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DaemonEvent {
    Snapshot(Box<AppSnapshot>),
    Acknowledged,
    Rejected(String),
}
