mod connection;
mod error;

pub use connection::{IpcConnection, IpcListener, PeerCredentials};
pub use error::IpcError;
