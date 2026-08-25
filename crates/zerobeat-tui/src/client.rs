use std::{path::Path, time::Duration};

use zerobeat_ipc::{IpcConnection, PeerCredentials};
use zerobeat_protocol::{AppSnapshot, ClientCommand, DaemonEvent, PROTOCOL_VERSION};

const DAEMON_HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(500);
const DAEMON_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("daemon IPC failed: {0}")]
    Ipc(#[from] zerobeat_ipc::IpcError),
    #[error("daemon rejected the request: {0}")]
    Rejected(String),
    #[error("daemon sent an unexpected response")]
    UnexpectedResponse,
    #[error("daemon IPC timed out during {0}")]
    Timeout(&'static str),
}

pub struct DaemonClient {
    connection: IpcConnection,
    snapshot: AppSnapshot,
}

impl DaemonClient {
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, ClientError> {
        let connection = IpcConnection::connect(path).await?;
        let mut client = Self {
            connection,
            snapshot: AppSnapshot::default(),
        };
        tokio::time::timeout(
            DAEMON_HANDSHAKE_TIMEOUT,
            client.execute(ClientCommand::Hello {
                protocol_version: PROTOCOL_VERSION,
            }),
        )
        .await
        .map_err(|_| ClientError::Timeout("hello"))??;
        Ok(client)
    }

    pub fn snapshot(&self) -> &AppSnapshot {
        &self.snapshot
    }

    pub(crate) fn peer_credentials(&self) -> Result<PeerCredentials, ClientError> {
        self.connection
            .peer_credentials()
            .map_err(zerobeat_ipc::IpcError::Io)
            .map_err(ClientError::Ipc)
    }

    pub async fn execute(&mut self, command: ClientCommand) -> Result<AppSnapshot, ClientError> {
        self.connection.send(&command).await?;
        match self.connection.receive().await? {
            DaemonEvent::Snapshot(snapshot) => {
                self.snapshot = *snapshot;
                Ok(self.snapshot.clone())
            }
            DaemonEvent::Rejected(reason) => Err(ClientError::Rejected(reason)),
            DaemonEvent::Acknowledged => Err(ClientError::UnexpectedResponse),
        }
    }

    pub async fn shutdown(&mut self) -> Result<(), ClientError> {
        tokio::time::timeout(DAEMON_SHUTDOWN_TIMEOUT, async {
            self.connection.send(&ClientCommand::Shutdown).await?;
            match self.connection.receive().await? {
                DaemonEvent::Acknowledged => Ok(()),
                DaemonEvent::Rejected(reason) => Err(ClientError::Rejected(reason)),
                DaemonEvent::Snapshot(_) => Err(ClientError::UnexpectedResponse),
            }
        })
        .await
        .map_err(|_| ClientError::Timeout("shutdown"))?
    }
}
