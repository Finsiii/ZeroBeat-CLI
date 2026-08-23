use serde::{Deserialize, Serialize};
use zerobeat_core::{NavigationState, Route, SessionMode, Track};

pub const PROTOCOL_VERSION: u16 = 2;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppSnapshot {
    pub session: SessionMode,
    pub navigation: NavigationState,
    pub search: SearchSnapshot,
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
pub enum ClientCommand {
    Hello { protocol_version: u16 },
    Navigate(Route),
    Back,
    UpdateSearch(String),
    SubmitSearch,
    SelectNext,
    SelectPrevious,
    RequestSnapshot,
    Shutdown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DaemonEvent {
    Snapshot(AppSnapshot),
    Acknowledged,
    Rejected(String),
}
