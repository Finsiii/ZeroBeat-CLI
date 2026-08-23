use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};
use zerobeat_protocol::{decode, encode};

use crate::IpcError;

const MAX_FRAME_BYTES: usize = 1024 * 1024;

pub struct IpcConnection {
    stream: UnixStream,
}

impl IpcConnection {
    pub async fn connect(path: impl AsRef<std::path::Path>) -> Result<Self, IpcError> {
        let stream = UnixStream::connect(path).await?;
        Ok(Self { stream })
    }

    pub fn from_stream(stream: UnixStream) -> Self {
        Self { stream }
    }

    pub async fn send<T: Serialize>(&mut self, message: &T) -> Result<(), IpcError> {
        let payload = encode(message)?;
        if payload.len() > MAX_FRAME_BYTES {
            return Err(IpcError::FrameTooLarge(payload.len()));
        }

        self.stream.write_u32(payload.len() as u32).await?;
        self.stream.write_all(&payload).await?;
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn receive<T: DeserializeOwned>(&mut self) -> Result<T, IpcError> {
        let length = self.stream.read_u32().await? as usize;
        if length > MAX_FRAME_BYTES {
            return Err(IpcError::FrameTooLarge(length));
        }

        let mut payload = vec![0; length];
        self.stream.read_exact(&mut payload).await?;
        decode(&payload).map_err(IpcError::from)
    }
}
