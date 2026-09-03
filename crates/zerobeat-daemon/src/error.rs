use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("daemon I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("daemon IPC failed: {0}")]
    Ipc(#[from] zerobeat_ipc::IpcError),
    #[error("daemon runtime preparation failed: {0}")]
    Runtime(#[from] zerobeat_runtime::RuntimeError),
    #[error("guest database failed: {0}")]
    Storage(#[from] zerobeat_storage::StorageError),
    #[error("socket path is occupied by a non-socket file: {0}")]
    SocketPathOccupied(PathBuf),
    #[error("another daemon is already listening on: {0}")]
    AlreadyRunning(PathBuf),
}
