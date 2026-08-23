use thiserror::Error;

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("IPC I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("IPC protocol failed: {0}")]
    Protocol(#[from] zerobeat_protocol::ProtocolError),
    #[error("IPC frame is too large: {0} bytes")]
    FrameTooLarge(usize),
}
