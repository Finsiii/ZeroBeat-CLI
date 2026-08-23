use std::path::Path;

use zerobeat_ipc::IpcConnection;
use zerobeat_protocol::{AppSnapshot, ClientCommand, DaemonEvent, PROTOCOL_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("daemon IPC failed: {0}")]
    Ipc(#[from] zerobeat_ipc::IpcError),
    #[error("daemon rejected the request: {0}")]
    Rejected(String),
    #[error("daemon sent an unexpected response")]
    UnexpectedResponse,
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
        client
            .execute(ClientCommand::Hello {
                protocol_version: PROTOCOL_VERSION,
            })
            .await?;
        Ok(client)
    }

    pub fn snapshot(&self) -> &AppSnapshot {
        &self.snapshot
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
        self.connection.send(&ClientCommand::Shutdown).await?;
        match self.connection.receive().await? {
            DaemonEvent::Acknowledged => Ok(()),
            DaemonEvent::Rejected(reason) => Err(ClientError::Rejected(reason)),
            DaemonEvent::Snapshot(_) => Err(ClientError::UnexpectedResponse),
        }
    }
}
