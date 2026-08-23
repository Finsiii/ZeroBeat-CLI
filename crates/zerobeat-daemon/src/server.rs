use std::{
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use tokio::{
    net::{UnixListener, UnixStream},
    sync::{Mutex, watch},
};
use zerobeat_audio::{AudioBackend, BackendError, StreamSource};
use zerobeat_catalog::{
    AudioQuality, CatalogError, CatalogFuture, MusicCatalog, ResolvedStream, SearchRequest,
};
use zerobeat_core::Track;
use zerobeat_ipc::IpcConnection;
use zerobeat_protocol::{
    AppSnapshot, ClientCommand, DaemonEvent, PROTOCOL_VERSION, PlaybackStatus, SearchStatus,
};

use crate::DaemonError;

pub struct DaemonServer {
    listener: UnixListener,
    socket_path: PathBuf,
    state: Arc<Mutex<AppSnapshot>>,
    catalog: Arc<dyn MusicCatalog>,
    audio: SharedAudio,
}

type SharedAudio = Arc<StdMutex<Box<dyn AudioBackend>>>;

impl DaemonServer {
    pub async fn bind(path: impl AsRef<Path>) -> Result<Self, DaemonError> {
        Self::bind_with_catalog(path, UnavailableCatalog).await
    }

    pub async fn bind_with_catalog(
        path: impl AsRef<Path>,
        catalog: impl MusicCatalog + 'static,
    ) -> Result<Self, DaemonError> {
        Self::bind_with_services(path, catalog, UnavailableAudio).await
    }

    pub async fn bind_with_services(
        path: impl AsRef<Path>,
        catalog: impl MusicCatalog + 'static,
        audio: impl AudioBackend + 'static,
    ) -> Result<Self, DaemonError> {
        let socket_path = path.as_ref().to_path_buf();
        remove_stale_socket(&socket_path).await?;
        let listener = UnixListener::bind(&socket_path)?;

        Ok(Self {
            listener,
            socket_path,
            state: Arc::new(Mutex::new(AppSnapshot::default())),
            catalog: Arc::new(catalog),
            audio: Arc::new(StdMutex::new(Box::new(audio))),
        })
    }

    pub async fn run(self) -> Result<(), DaemonError> {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let mut telemetry_tick = tokio::time::interval(Duration::from_millis(250));

        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted?;
                    let state = Arc::clone(&self.state);
                    let catalog = Arc::clone(&self.catalog);
                    let audio = Arc::clone(&self.audio);
                    let shutdown_tx = shutdown_tx.clone();
                    tokio::spawn(async move {
                        let _ = handle_client(stream, state, catalog, audio, shutdown_tx).await;
                    });
                }
                _ = telemetry_tick.tick() => refresh_playback_telemetry(&self.state, &self.audio).await,
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }

        if let Ok(mut audio) = self.audio.lock() {
            let _ = audio.stop();
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
    audio: SharedAudio,
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

        let (event, should_shutdown) = apply_command(command, &state, &catalog, &audio).await;
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
    audio: &SharedAudio,
) -> (DaemonEvent, bool) {
    match command {
        ClientCommand::Hello { protocol_version } if protocol_version != PROTOCOL_VERSION => (
            DaemonEvent::Rejected(format!("unsupported protocol version {protocol_version}")),
            false,
        ),
        ClientCommand::Hello { .. } | ClientCommand::RequestSnapshot => {
            let snapshot = state.lock().await.clone();
            (snapshot_event(snapshot), false)
        }
        ClientCommand::Navigate(route) => {
            let mut snapshot = state.lock().await;
            snapshot.navigation.open(route);
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::Back => {
            let mut snapshot = state.lock().await;
            snapshot.navigation.back();
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::UpdateSearch(query) => {
            let mut snapshot = state.lock().await;
            snapshot.navigation.update_search(query);
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::SubmitSearch => {
            let mut snapshot = state.lock().await;
            let request = match SearchRequest::new(snapshot.navigation.search_query(), 30) {
                Ok(request) => request,
                Err(error) => {
                    snapshot.search.status = SearchStatus::Failed(error.to_string());
                    return (snapshot_event(snapshot.clone()), false);
                }
            };
            snapshot.search.request_id = snapshot.search.request_id.saturating_add(1);
            let request_id = snapshot.search.request_id;
            snapshot.search.status = SearchStatus::Loading;
            let event = snapshot_event(snapshot.clone());
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
            (snapshot_event(snapshot.clone()), false)
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
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::PlaySelected => {
            let mut snapshot = state.lock().await;
            let Some(track) = snapshot
                .search
                .results
                .get(snapshot.search.selected_index)
                .cloned()
            else {
                return (snapshot_event(snapshot.clone()), false);
            };
            snapshot.playback.request_id = snapshot.playback.request_id.saturating_add(1);
            let request_id = snapshot.playback.request_id;
            snapshot.playback.current = Some(track.clone());
            snapshot.playback.position_ms = 0;
            snapshot.playback.duration_ms = track.duration_ms;
            snapshot.playback.buffered_ms = 0;
            snapshot.playback.error = None;
            snapshot.playback.status = PlaybackStatus::Resolving;
            let event = snapshot_event(snapshot.clone());
            drop(snapshot);

            spawn_playback(
                track,
                request_id,
                Arc::clone(state),
                Arc::clone(catalog),
                Arc::clone(audio),
            );
            (event, false)
        }
        ClientCommand::TogglePlayback => {
            let current = state.lock().await.playback.status;
            let result = match current {
                PlaybackStatus::Playing => {
                    run_audio(Arc::clone(audio), |audio| audio.pause()).await
                }
                PlaybackStatus::Paused => run_audio(Arc::clone(audio), |audio| audio.play()).await,
                _ => Ok(()),
            };
            let mut snapshot = state.lock().await;
            if let Err(error) = result {
                set_playback_error(&mut snapshot, error);
            } else if current == PlaybackStatus::Playing {
                snapshot.playback.status = PlaybackStatus::Paused;
            } else if current == PlaybackStatus::Paused {
                snapshot.playback.status = PlaybackStatus::Playing;
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::NextTrack => {
            let result = run_audio(Arc::clone(audio), |audio| audio.stop()).await;
            let mut snapshot = state.lock().await;
            snapshot.playback.request_id = snapshot.playback.request_id.saturating_add(1);
            if let Err(error) = result {
                set_playback_error(&mut snapshot, error);
            } else {
                snapshot.playback.status = PlaybackStatus::Idle;
                snapshot.playback.current = None;
                snapshot.playback.position_ms = 0;
                snapshot.playback.duration_ms = 0;
                snapshot.playback.buffered_ms = 0;
                snapshot.playback.error = None;
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::SeekRelative(offset_ms) => {
            let target = {
                let snapshot = state.lock().await;
                let position = i128::from(snapshot.playback.position_ms) + i128::from(offset_ms);
                let maximum = i128::from(snapshot.playback.duration_ms);
                u64::try_from(position.clamp(0, maximum)).unwrap_or_default()
            };
            let result = run_audio(Arc::clone(audio), move |audio| audio.seek(target)).await;
            let mut snapshot = state.lock().await;
            if let Err(error) = result {
                set_playback_error(&mut snapshot, error);
            } else {
                snapshot.playback.position_ms = target;
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::SetVolume(percent) => {
            let percent = percent.min(100);
            let result = run_audio(Arc::clone(audio), move |audio| {
                audio.set_volume(f32::from(percent) / 100.0)
            })
            .await;
            let mut snapshot = state.lock().await;
            if let Err(error) = result {
                set_playback_error(&mut snapshot, error);
            } else {
                snapshot.playback.volume_percent = percent;
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::Shutdown => (DaemonEvent::Acknowledged, true),
    }
}

fn snapshot_event(snapshot: AppSnapshot) -> DaemonEvent {
    DaemonEvent::Snapshot(Box::new(snapshot))
}

fn spawn_playback(
    track: Track,
    request_id: u64,
    state: Arc<Mutex<AppSnapshot>>,
    catalog: Arc<dyn MusicCatalog>,
    audio: SharedAudio,
) {
    tokio::spawn(async move {
        let stream = match catalog
            .resolve_stream(&track.id, AudioQuality::Automatic)
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                finish_playback_error(&state, request_id, error.to_string()).await;
                return;
            }
        };
        {
            let mut snapshot = state.lock().await;
            if snapshot.playback.request_id != request_id {
                return;
            }
            snapshot.playback.status = PlaybackStatus::Buffering;
        }
        let mut source = StreamSource::new(stream.url);
        source.headers = stream.headers;
        let result = run_audio(audio, move |audio| {
            let _ = audio.stop();
            audio.load(&source)?;
            audio.play()
        })
        .await;
        let mut snapshot = state.lock().await;
        if snapshot.playback.request_id != request_id {
            return;
        }
        match result {
            Ok(()) => snapshot.playback.status = PlaybackStatus::Playing,
            Err(error) => set_playback_error(&mut snapshot, error),
        }
    });
}

async fn run_audio(
    audio: SharedAudio,
    operation: impl FnOnce(&mut dyn AudioBackend) -> Result<(), BackendError> + Send + 'static,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let mut audio = audio
            .lock()
            .map_err(|_| "audio engine lock was poisoned".to_owned())?;
        operation(audio.as_mut()).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("audio worker failed: {error}"))?
}

async fn finish_playback_error(state: &Mutex<AppSnapshot>, request_id: u64, error: String) {
    let mut snapshot = state.lock().await;
    if snapshot.playback.request_id == request_id {
        set_playback_error(&mut snapshot, error);
    }
}

fn set_playback_error(snapshot: &mut AppSnapshot, error: String) {
    snapshot.playback.status = PlaybackStatus::Failed;
    snapshot.playback.error = Some(error);
}

async fn refresh_playback_telemetry(
    state: &Mutex<AppSnapshot>,
    audio: &StdMutex<Box<dyn AudioBackend>>,
) {
    let telemetry = match audio.try_lock() {
        Ok(audio) => audio.telemetry(),
        Err(_) => return,
    };
    let mut snapshot = state.lock().await;
    if matches!(
        snapshot.playback.status,
        PlaybackStatus::Playing | PlaybackStatus::Paused
    ) {
        snapshot.playback.position_ms = telemetry.position_ms;
        snapshot.playback.duration_ms = telemetry.duration_ms.max(snapshot.playback.duration_ms);
        snapshot.playback.buffered_ms = telemetry.buffered_ms;
        if telemetry.ended {
            snapshot.playback.status = PlaybackStatus::Ended;
        }
    }
}

struct UnavailableCatalog;

struct UnavailableAudio;

impl AudioBackend for UnavailableAudio {
    fn load(&mut self, _source: &StreamSource) -> Result<(), BackendError> {
        Err(BackendError::Unavailable("audio is not configured".into()))
    }

    fn play(&mut self) -> Result<(), BackendError> {
        Err(BackendError::Unavailable("audio is not configured".into()))
    }

    fn pause(&mut self) -> Result<(), BackendError> {
        Err(BackendError::Unavailable("audio is not configured".into()))
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
}

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
