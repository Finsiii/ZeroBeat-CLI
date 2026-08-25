use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};
use zerobeat_protocol::{decode, encode};

use crate::IpcError;

const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerCredentials {
    pub pid: Option<u32>,
    pub uid: u32,
}

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

    pub fn peer_credentials(&self) -> std::io::Result<PeerCredentials> {
        let credentials = self.stream.peer_cred()?;
        Ok(PeerCredentials {
            pid: credentials.pid().and_then(|pid| u32::try_from(pid).ok()),
            uid: credentials.uid(),
        })
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use tempfile::tempdir;
    use tokio::net::UnixListener;

    use super::IpcConnection;

    #[tokio::test]
    async fn peer_credentials_expose_the_connected_process() {
        let directory = tempdir().unwrap();
        let socket = directory.path().join("peer.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            IpcConnection::from_stream(stream)
        });

        let client = IpcConnection::connect(&socket).await.unwrap();
        let credentials = client.peer_credentials().unwrap();
        assert_eq!(credentials.pid, Some(std::process::id()));
        assert_eq!(
            credentials.uid,
            std::fs::metadata(directory.path()).unwrap().uid()
        );
        drop(client);
        drop(server.await.unwrap());
    }
}
