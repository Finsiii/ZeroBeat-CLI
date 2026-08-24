use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use zerobeat_ipc::IpcError;
use zerobeat_protocol::{ClientCommand, DaemonEvent, PROTOCOL_VERSION};
use zerobeat_runtime::prepare_runtime_dir;

use crate::{ClientError, DaemonClient};

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("failed to prepare runtime directory: {0}")]
    Runtime(#[from] zerobeat_runtime::RuntimeError),
    #[error("failed to locate or start zerobeatd: {0}")]
    Process(#[from] std::io::Error),
    #[error("failed to connect to zerobeatd: {0}")]
    Client(#[from] ClientError),
}

pub async fn connect_or_spawn(socket: &Path) -> Result<DaemonClient, LaunchError> {
    retire_legacy_daemon(socket).await;
    match DaemonClient::connect(socket).await {
        Ok(client) => return Ok(client),
        Err(error) if daemon_is_unavailable(&error) => {}
        Err(error) => return Err(error.into()),
    }

    if let Some(parent) = socket.parent() {
        prepare_runtime_dir(parent)?;
    }
    let daemon = daemon_executable()?;
    Command::new(daemon)
        .arg("--socket")
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let mut last_error = None;
    for _ in 0..40 {
        match DaemonClient::connect(socket).await {
            Ok(client) => return Ok(client),
            Err(error) if daemon_is_unavailable(&error) => last_error = Some(error),
            Err(error) => return Err(error.into()),
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    Err(last_error.expect("connection retry must run").into())
}

async fn retire_legacy_daemon(current_socket: &Path) {
    let Some(parent) = current_socket.parent() else {
        return;
    };
    let legacy_socket = parent.join("daemon.sock");
    if legacy_socket == current_socket {
        return;
    }
    let Ok(mut connection) = zerobeat_ipc::IpcConnection::connect(&legacy_socket).await else {
        return;
    };
    if connection
        .send(&ClientCommand::Hello {
            protocol_version: PROTOCOL_VERSION,
        })
        .await
        .is_err()
    {
        return;
    }
    let Ok(DaemonEvent::Rejected(reason)) = connection.receive::<DaemonEvent>().await else {
        return;
    };
    if !reason.starts_with("unsupported protocol version") {
        return;
    }
    if connection.send(&ClientCommand::Shutdown).await.is_ok() {
        let _ = connection.receive::<DaemonEvent>().await;
    }
}

fn daemon_is_unavailable(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::Ipc(IpcError::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            )
    )
}

fn daemon_executable() -> Result<PathBuf, std::io::Error> {
    if let Some(path) = std::env::var_os("ZEROBEATD_PATH") {
        return Ok(path.into());
    }
    let current = std::env::current_exe()?;
    Ok(current.with_file_name("zerobeatd"))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use tokio::net::UnixListener;
    use zerobeat_ipc::IpcConnection;
    use zerobeat_protocol::{ClientCommand, DaemonEvent, PROTOCOL_VERSION};

    use super::retire_legacy_daemon;

    #[tokio::test]
    async fn incompatible_legacy_daemon_is_shut_down_before_upgrade() {
        let directory = tempdir().unwrap();
        let legacy_socket = directory.path().join("daemon.sock");
        let listener = UnixListener::bind(&legacy_socket).unwrap();
        let legacy = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut connection = IpcConnection::from_stream(stream);
            assert_eq!(
                connection.receive::<ClientCommand>().await.unwrap(),
                ClientCommand::Hello {
                    protocol_version: PROTOCOL_VERSION
                }
            );
            connection
                .send(&DaemonEvent::Rejected(format!(
                    "unsupported protocol version {PROTOCOL_VERSION}"
                )))
                .await
                .unwrap();
            assert_eq!(
                connection.receive::<ClientCommand>().await.unwrap(),
                ClientCommand::Shutdown
            );
            connection.send(&DaemonEvent::Acknowledged).await.unwrap();
        });

        retire_legacy_daemon(&directory.path().join("daemon-v9.sock")).await;

        legacy.await.unwrap();
    }
}
