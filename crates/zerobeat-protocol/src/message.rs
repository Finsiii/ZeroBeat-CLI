use serde::{Deserialize, Serialize};
use zerobeat_core::{NavigationState, Route, SessionMode};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppSnapshot {
    pub session: SessionMode,
    pub navigation: NavigationState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ClientCommand {
    Hello { protocol_version: u16 },
    Navigate(Route),
    UpdateSearch(String),
    RequestSnapshot,
    Shutdown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DaemonEvent {
    Snapshot(AppSnapshot),
    Acknowledged,
    Rejected(String),
}
