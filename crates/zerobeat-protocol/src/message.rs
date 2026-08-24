use serde::{Deserialize, Serialize};
use zerobeat_core::{NavigationState, Route, SessionMode, Track};

pub const PROTOCOL_VERSION: u16 = 8;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppSnapshot {
    pub session: SessionMode,
    pub navigation: NavigationState,
    pub search: SearchSnapshot,
    pub playback: PlaybackSnapshot,
    pub library: LibrarySnapshot,
    pub lyrics: LyricsSnapshot,
    pub settings: SettingsSnapshot,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SettingsSnapshot {
    pub crossfade_seconds: u8,
}

impl Default for SettingsSnapshot {
    fn default() -> Self {
        Self {
            crossfade_seconds: 6,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LyricsSnapshot {
    pub visible: bool,
    pub track_id: Option<String>,
    pub status: LyricsStatus,
    pub synced: bool,
    pub lines: Vec<LyricsLineSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LyricsLineSnapshot {
    pub start_ms: Option<u64>,
    pub words: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum LyricsStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Unavailable,
    Failed(String),
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LibrarySnapshot {
    pub liked: Vec<Track>,
    pub recent: Vec<Track>,
    pub downloads: Vec<DownloadSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DownloadSnapshot {
    pub track: Track,
    pub status: DownloadStatus,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Available,
    Failed,
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
    pub queue: Vec<Track>,
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
            queue: Vec::new(),
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
    QueueSelected,
    PlayTrack(Track),
    QueueTrack(Track),
    ToggleLike(Track),
    DownloadTrack(Track),
    ToggleLyrics,
    SetCrossfadeSeconds(u8),
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
