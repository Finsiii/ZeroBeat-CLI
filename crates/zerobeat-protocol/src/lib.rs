mod codec;
mod message;

pub use codec::{ProtocolError, decode, encode};
pub use message::{
    AppSnapshot, ClientCommand, DaemonEvent, DownloadSnapshot, DownloadStatus, LibrarySnapshot,
    LyricsLineSnapshot, LyricsSnapshot, LyricsStatus, PROTOCOL_VERSION, PlaybackSnapshot,
    PlaybackStatus, SearchSnapshot, SearchStatus, SettingsSnapshot,
};
