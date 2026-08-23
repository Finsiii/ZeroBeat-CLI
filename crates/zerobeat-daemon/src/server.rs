use std::{
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::{
    net::{UnixListener, UnixStream},
    sync::{Mutex, watch},
};
use zerobeat_ipc::IpcConnection;
use zerobeat_protocol::{AppSnapshot, ClientCommand, DaemonEvent, PROTOCOL_VERSION};

use crate::DaemonError;

pub struct DaemonServer {
    listener: UnixListener,
    socket_path: PathBuf,
    state: Arc<Mutex<AppSnapshot>>,
}

impl DaemonServer {
    pub async fn bind(path: impl AsRef<Path>) -> Result<Self, DaemonError> {
        let socket_path = path.as_ref().to_path_buf();
        remove_stale_socket(&socket_path)?;
        let listener = UnixListener::bind(&socket_path)?;

        Ok(Self {
            listener,
            socket_path,
            state: Arc::new(Mutex::new(AppSnapshot::default())),
        })
    }

    pub async fn run(self) -> Result<(), DaemonError> {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted?;
                    let state = Arc::clone(&self.state);
                    let shutdown_tx = shutdown_tx.clone();
                    tokio::spawn(async move {
                        let _ = handle_client(stream, state, shutdown_tx).await;
                    });
                }
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }

        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }
        Ok(())
    }
}

async fn handle_client(
    stream: UnixStream,
    state: Arc<Mutex<AppSnapshot>>,
    shutdown: watch::Sender<bool>,
) -> Result<(), DaemonError> {
    let mut connection = IpcConnection::from_stream(stream);

    loop {
        let command: ClientCommand = match connection.receive().await {
            Ok(command) => command,
            Err(zerobeat_ipc::IpcError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionReset
                ) =>
            {
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };

        let (event, should_shutdown) = apply_command(command, &state).await;
        connection.send(&event).await?;
        if should_shutdown {
            let _ = shutdown.send(true);
            return Ok(());
        }
    }
}

async fn apply_command(command: ClientCommand, state: &Mutex<AppSnapshot>) -> (DaemonEvent, bool) {
    match command {
        ClientCommand::Hello { protocol_version } if protocol_version != PROTOCOL_VERSION => (
            DaemonEvent::Rejected(format!("unsupported protocol version {protocol_version}")),
            false,
        ),
        ClientCommand::Hello { .. } | ClientCommand::RequestSnapshot => {
            let snapshot = state.lock().await.clone();
            (DaemonEvent::Snapshot(snapshot), false)
        }
        ClientCommand::Navigate(route) => {
            let mut snapshot = state.lock().await;
            snapshot.navigation.open(route);
            (DaemonEvent::Snapshot(snapshot.clone()), false)
        }
        ClientCommand::UpdateSearch(query) => {
            let mut snapshot = state.lock().await;
            snapshot.navigation.update_search(query);
            (DaemonEvent::Snapshot(snapshot.clone()), false)
        }
        ClientCommand::Shutdown => (DaemonEvent::Acknowledged, true),
    }
}

fn remove_stale_socket(path: &Path) -> Result<(), DaemonError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };

    if !metadata.file_type().is_socket() {
        return Err(DaemonError::SocketPathOccupied(path.to_path_buf()));
    }

    std::fs::remove_file(path)?;
    Ok(())
}
