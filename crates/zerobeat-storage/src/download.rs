use std::str::FromStr;

use zerobeat_core::Track;

use crate::StorageError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadState {
    Queued,
    Downloading,
    Available,
    Failed,
}

impl DownloadState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Downloading => "downloading",
            Self::Available => "available",
            Self::Failed => "failed",
        }
    }
}

impl FromStr for DownloadState {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "queued" => Ok(Self::Queued),
            "downloading" => Ok(Self::Downloading),
            "available" => Ok(Self::Available),
            "failed" => Ok(Self::Failed),
            other => Err(StorageError::InvalidDownloadState(other.to_owned())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    pub track: Track,
    pub state: DownloadState,
    pub local_path: Option<String>,
    pub error: Option<String>,
}
