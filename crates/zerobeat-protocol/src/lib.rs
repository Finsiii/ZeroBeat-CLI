mod codec;
mod message;

pub use codec::{ProtocolError, decode, encode};
pub use message::{
    AppSnapshot, ClientCommand, DaemonEvent, PROTOCOL_VERSION, PlaybackSnapshot, PlaybackStatus,
    SearchSnapshot, SearchStatus,
};
