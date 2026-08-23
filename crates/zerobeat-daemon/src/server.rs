use std::{
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
    sync::Arc,
};

use tokio::{
    net::{UnixListener, UnixStream},
    sync::{Mutex, watch},
};
use zerobeat_catalog::{
    AudioQuality, CatalogError, CatalogFuture, MusicCatalog, ResolvedStream, SearchRequest,
};
use zerobeat_core::Track;
use zerobeat_ipc::IpcConnection;
use zerobeat_protocol::{AppSnapshot, ClientCommand, DaemonEvent, PROTOCOL_VERSION, SearchStatus};

use crate::DaemonError;

pub struct DaemonServer {
    listener: UnixListener,
    socket_path: PathBuf,
    state: Arc<Mutex<AppSnapshot>>,
    catalog: Arc<dyn MusicCatalog>,
}

impl DaemonServer {
    pub async fn bind(path: impl AsRef<Path>) -> Result<Self, DaemonError> {
        Self::bind_with_catalog(path, UnavailableCatalog).await
    }

    pub async fn bind_with_catalog(
        path: impl AsRef<Path>,
        catalog: impl MusicCatalog + 'static,
    ) -> Result<Self, DaemonError> {
        let socket_path = path.as_ref().to_path_buf();
        remove_stale_socket(&socket_path).await?;
        let listener = UnixListener::bind(&socket_path)?;

        Ok(Self {
            listener,
            socket_path,
            state: Arc::new(Mutex::new(AppSnapshot::default())),
            catalog: Arc::new(catalog),
        })
    }

    pub async fn run(self) -> Result<(), DaemonError> {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted?;
                    let state = Arc::clone(&self.state);
                    let catalog = Arc::clone(&self.catalog);
                    let shutdown_tx = shutdown_tx.clone();
                    tokio::spawn(async move {
                        let _ = handle_client(stream, state, catalog, shutdown_tx).await;
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
    catalog: Arc<dyn MusicCatalog>,
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

        let (event, should_shutdown) = apply_command(command, &state, &catalog).await;
        connection.send(&event).await?;
        if should_shutdown {
            let _ = shutdown.send(true);
            return Ok(());
        }
    }
}

async fn apply_command(
    command: ClientCommand,
    state: &Arc<Mutex<AppSnapshot>>,
    catalog: &Arc<dyn MusicCatalog>,
) -> (DaemonEvent, bool) {
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
        ClientCommand::Back => {
            let mut snapshot = state.lock().await;
            snapshot.navigation.back();
            (DaemonEvent::Snapshot(snapshot.clone()), false)
        }
        ClientCommand::UpdateSearch(query) => {
            let mut snapshot = state.lock().await;
            snapshot.navigation.update_search(query);
            (DaemonEvent::Snapshot(snapshot.clone()), false)
        }
        ClientCommand::SubmitSearch => {
            let mut snapshot = state.lock().await;
            let request = match SearchRequest::new(snapshot.navigation.search_query(), 30) {
                Ok(request) => request,
                Err(error) => {
                    snapshot.search.status = SearchStatus::Failed(error.to_string());
                    return (DaemonEvent::Snapshot(snapshot.clone()), false);
                }
            };
            snapshot.search.request_id = snapshot.search.request_id.saturating_add(1);
            let request_id = snapshot.search.request_id;
            snapshot.search.status = SearchStatus::Loading;
            let event = DaemonEvent::Snapshot(snapshot.clone());
            drop(snapshot);

            let state = Arc::clone(state);
            let catalog = Arc::clone(catalog);
            tokio::spawn(async move {
                let result = catalog.search_songs(request).await;
                let mut snapshot = state.lock().await;
                if snapshot.search.request_id != request_id {
                    return;
                }
                match result {
                    Ok(results) => {
                        snapshot.search.results = results;
                        snapshot.search.selected_index = 0;
                        snapshot.search.status = SearchStatus::Ready;
                    }
                    Err(error) => {
                        snapshot.search.status = SearchStatus::Failed(error.to_string());
                    }
                }
            });
            (event, false)
        }
        ClientCommand::SelectNext => {
            let mut snapshot = state.lock().await;
            if !snapshot.search.results.is_empty() {
                snapshot.search.selected_index =
                    (snapshot.search.selected_index + 1) % snapshot.search.results.len();
            }
            (DaemonEvent::Snapshot(snapshot.clone()), false)
        }
        ClientCommand::SelectPrevious => {
            let mut snapshot = state.lock().await;
            if !snapshot.search.results.is_empty() {
                snapshot.search.selected_index = snapshot
                    .search
                    .selected_index
                    .checked_sub(1)
                    .unwrap_or(snapshot.search.results.len() - 1);
            }
            (DaemonEvent::Snapshot(snapshot.clone()), false)
        }
        ClientCommand::Shutdown => (DaemonEvent::Acknowledged, true),
    }
}

struct UnavailableCatalog;

impl MusicCatalog for UnavailableCatalog {
    fn search_songs(&self, _request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
        Box::pin(async { Err(CatalogError::Unavailable("API is not configured".into())) })
    }

    fn resolve_stream(
        &self,
        _track_id: &str,
        _quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream> {
        Box::pin(async { Err(CatalogError::Unavailable("API is not configured".into())) })
    }
}

async fn remove_stale_socket(path: &Path) -> Result<(), DaemonError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };

    if !metadata.file_type().is_socket() {
        return Err(DaemonError::SocketPathOccupied(path.to_path_buf()));
    }

    match UnixStream::connect(path).await {
        Ok(_) => return Err(DaemonError::AlreadyRunning(path.to_path_buf())),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) => {}
        Err(error) => return Err(error.into()),
    }

    std::fs::remove_file(path)?;
    Ok(())
}
