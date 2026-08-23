use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum SessionMode {
    #[default]
    Guest,
    Account,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Capability {
    Playback,
    Search,
    Library,
    Downloads,
    Lyrics,
    Sync,
}

impl SessionMode {
    pub fn supports(self, capability: Capability) -> bool {
        capability != Capability::Sync || self == Self::Account
    }
}
