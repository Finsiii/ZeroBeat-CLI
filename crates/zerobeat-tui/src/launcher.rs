use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use zerobeat_ipc::IpcError;
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
