use std::{
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use tokio::{
    net::{UnixListener, UnixStream},
    sync::{Mutex, watch},
};
#[cfg(target_os = "linux")]
use zerobeat_audio::CancellationController;
use zerobeat_audio::{AudioBackend, BackendError, StreamSource};
use zerobeat_catalog::{
    AudioQuality, CatalogError, CatalogFuture, MusicCatalog, MusicQueue, QueueFuture,
    QueueRepeatMode, QueueSession, QueueStart, ResolvedStream, SearchRequest,
};
use zerobeat_core::Track;
use zerobeat_ipc::IpcConnection;
use zerobeat_protocol::{
    AppSnapshot, ClientCommand, DaemonEvent, DownloadSnapshot, DownloadStatus, LibrarySnapshot,
    LyricsLineSnapshot, LyricsStatus, PROTOCOL_VERSION, PlaybackStatus, RepeatMode, SearchStatus,
    SettingsSnapshot,
};
use zerobeat_storage::{Database, DownloadState};

use crate::{DaemonError, download::spawn_download, stream_cache::StreamCache};

pub struct DaemonServer {
    listener: UnixListener,
    socket_path: PathBuf,
    playback: PlaybackCoordinator,
    catalog: Arc<dyn MusicCatalog>,
    queue: Arc<dyn MusicQueue>,
    audio: SharedAudio,
    storage: SharedStorage,
    download_directory: Arc<PathBuf>,
}

type SharedAudio = Arc<StdMutex<Box<dyn AudioBackend>>>;
type SharedStorage = Arc<StdMutex<Database>>;

#[derive(Clone)]
struct PlaybackCoordinator {
    state: Arc<Mutex<AppSnapshot>>,
    generation: Arc<AtomicU64>,
    pending_transition: Arc<AtomicU64>,
    queue_session: Arc<StdMutex<Option<(String, i64)>>>,
    queue_endless: Arc<StdMutex<bool>>,
    queue_current_index: Arc<StdMutex<usize>>,
    queue_projection: Arc<StdMutex<Option<QueueProjection>>>,
    queue_refill_marker: Arc<StdMutex<Option<RefillMarker>>>,
    queue_refill_backoff: Arc<StdMutex<Option<RefillBackoff>>>,
    queue_refill_in_flight: Arc<AtomicBool>,
    auto_advance_marker: Arc<StdMutex<Option<AutoAdvanceMarker>>>,
    queue_mutation: Arc<Mutex<()>>,
    stream_cache: Arc<StreamCache>,
    #[cfg(target_os = "linux")]
    cancellation: Option<CancellationController>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QueueProjection {
    session_id: String,
    revision: i64,
    indices: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RefillMarker {
    session_id: String,
    revision: i64,
    visible_count: usize,
}

struct RefillBackoff {
    session_id: String,
    revision: i64,
    attempts: u32,
    retry_at: Instant,
}

struct AutoAdvanceMarker {
    session_id: String,
    revision: i64,
    current_index: usize,
    attempts: u32,
    terminal: bool,
    retry_at: Instant,
}

const MAX_VISIBLE_QUEUE: usize = 12;

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
        let download_directory =
            std::env::temp_dir().join(format!("zerobeat-{}-downloads", std::process::id()));
        Self::bind_with_services_and_storage_and_queue(
            path,
            catalog,
            UnavailableQueue,
            audio,
            Database::open_in_memory()?,
            download_directory,
        )
        .await
    }

    pub async fn bind_with_services_and_queue(
        path: impl AsRef<Path>,
        catalog: impl MusicCatalog + 'static,
        queue: impl MusicQueue + 'static,
        audio: impl AudioBackend + 'static,
    ) -> Result<Self, DaemonError> {
        let download_directory =
            std::env::temp_dir().join(format!("zerobeat-{}-downloads", std::process::id()));
        Self::bind_with_services_and_storage_and_queue(
            path,
            catalog,
            queue,
            audio,
            Database::open_in_memory()?,
            download_directory,
        )
        .await
    }

    pub async fn bind_with_services_and_storage(
        path: impl AsRef<Path>,
        catalog: impl MusicCatalog + 'static,
        audio: impl AudioBackend + 'static,
        storage: Database,
        download_directory: impl Into<PathBuf>,
    ) -> Result<Self, DaemonError> {
        Self::bind_with_services_and_storage_and_queue(
            path,
            catalog,
            UnavailableQueue,
            audio,
            storage,
            download_directory,
        )
        .await
    }

    pub async fn bind_with_services_and_storage_and_queue(
        path: impl AsRef<Path>,
        catalog: impl MusicCatalog + 'static,
        queue: impl MusicQueue + 'static,
        audio: impl AudioBackend + 'static,
        storage: Database,
        download_directory: impl Into<PathBuf>,
    ) -> Result<Self, DaemonError> {
        let socket_path = path.as_ref().to_path_buf();
        remove_stale_socket(&socket_path).await?;
        let listener = UnixListener::bind(&socket_path)?;
        let download_directory = download_directory.into();
        std::fs::create_dir_all(&download_directory)?;
        std::fs::set_permissions(&download_directory, std::fs::Permissions::from_mode(0o700))?;
        let library = library_snapshot(&storage)?;
        let settings = SettingsSnapshot {
            crossfade_seconds: storage.crossfade_seconds()?,
        };
        let state = AppSnapshot {
            library,
            settings,
            ..AppSnapshot::default()
        };
        let audio_backend: Box<dyn AudioBackend> = Box::new(audio);
        #[cfg(target_os = "linux")]
        let cancellation = audio_backend.cancellation_controller();

        Ok(Self {
            listener,
            socket_path,
            playback: PlaybackCoordinator {
                state: Arc::new(Mutex::new(state)),
                generation: Arc::new(AtomicU64::new(0)),
                pending_transition: Arc::new(AtomicU64::new(0)),
                queue_session: Arc::new(StdMutex::new(None)),
                queue_endless: Arc::new(StdMutex::new(false)),
                queue_current_index: Arc::new(StdMutex::new(0)),
                queue_projection: Arc::new(StdMutex::new(None)),
                queue_refill_marker: Arc::new(StdMutex::new(None)),
                queue_refill_backoff: Arc::new(StdMutex::new(None)),
                queue_refill_in_flight: Arc::new(AtomicBool::new(false)),
                auto_advance_marker: Arc::new(StdMutex::new(None)),
                queue_mutation: Arc::new(Mutex::new(())),
                stream_cache: Arc::new(StreamCache::new()),
                #[cfg(target_os = "linux")]
                cancellation,
            },
            catalog: Arc::new(catalog),
            queue: Arc::new(queue),
            audio: Arc::new(StdMutex::new(audio_backend)),
            storage: Arc::new(StdMutex::new(storage)),
            download_directory: Arc::new(download_directory),
        })
    }

    pub async fn run(self) -> Result<(), DaemonError> {
        if let Ok(Some(session)) = self.queue.active_queue().await
            && accept_queue_session(&self.playback, &session)
        {
            let mut snapshot = self.playback.state.lock().await;
            project_queue_for(&self.playback, &mut snapshot, &session);
        }
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let mut telemetry_tick = tokio::time::interval(Duration::from_millis(250));

        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted?;
                    let playback = self.playback.clone();
                    let catalog = Arc::clone(&self.catalog);
                    let queue = Arc::clone(&self.queue);
                    let audio = Arc::clone(&self.audio);
                    let storage = Arc::clone(&self.storage);
                    let download_directory = Arc::clone(&self.download_directory);
                    let shutdown_tx = shutdown_tx.clone();
                    tokio::spawn(async move {
                        let _ = handle_client(
                            stream,
                            playback,
                            catalog,
                            queue,
                            audio,
                            storage,
                            download_directory,
                            shutdown_tx,
                        ).await;
                    });
                }
                _ = telemetry_tick.tick() => {
                    refresh_playback_telemetry(
                        &self.playback,
                        &self.catalog,
                        &self.queue,
                        &self.audio,
                        &self.storage,
                    ).await;
                }
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

#[allow(clippy::too_many_arguments)]
async fn handle_client(
    stream: UnixStream,
    playback: PlaybackCoordinator,
    catalog: Arc<dyn MusicCatalog>,
    queue: Arc<dyn MusicQueue>,
    audio: SharedAudio,
    storage: SharedStorage,
    download_directory: Arc<PathBuf>,
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

        let (event, should_shutdown) = apply_command(
            command,
            &playback,
            &catalog,
            &queue,
            &audio,
            &storage,
            &download_directory,
        )
        .await;
        connection.send(&event).await?;
        if should_shutdown {
            let _ = shutdown.send(true);
            return Ok(());
        }
    }
}

#[allow(clippy::collapsible_if)]
async fn apply_command(
    command: ClientCommand,
    playback: &PlaybackCoordinator,
    catalog: &Arc<dyn MusicCatalog>,
    queue: &Arc<dyn MusicQueue>,
    audio: &SharedAudio,
    storage: &SharedStorage,
    download_directory: &Arc<PathBuf>,
) -> (DaemonEvent, bool) {
    let state = &playback.state;
    let playback_generation = &playback.generation;
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
            snapshot.lyrics.visible = false;
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::Back => {
            let mut snapshot = state.lock().await;
            snapshot.navigation.back();
            snapshot.lyrics.visible = false;
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
            let _queue_guard = playback.queue_mutation.lock().await;
            let (selected_index, tracks, start) = {
                let snapshot = state.lock().await;
                let selected_index = snapshot.search.selected_index;
                let Some(_) = snapshot.search.results.get(selected_index) else {
                    return (snapshot_event(snapshot.clone()), false);
                };
                let start = if matches!(
                    snapshot.playback.status,
                    PlaybackStatus::Playing | PlaybackStatus::Paused
                ) {
                    PlaybackStart::Crossfade(Duration::from_millis(500))
                } else {
                    PlaybackStart::Replace
                };
                (selected_index, snapshot.search.results.clone(), start)
            };
            let queue_request = QueueStart {
                tracks: tracks.clone(),
                current_index: selected_index,
                endless_queue: true,
                ..QueueStart::default()
            };
            let backend_session = match queue.start_queue(queue_request).await {
                Ok(session) => session,
                Err(error) => {
                    let mut snapshot = state.lock().await;
                    snapshot.playback.error = Some(error.to_string());
                    snapshot.playback.status = PlaybackStatus::Failed;
                    return (snapshot_event(snapshot.clone()), false);
                }
            };
            let mut snapshot = state.lock().await;
            let track = backend_session
                .tracks
                .get(backend_session.current_index)
                .cloned()
                .or_else(|| tracks.get(selected_index).cloned());
            let Some(track) = track else {
                return (snapshot_event(snapshot.clone()), false);
            };
            cancel_incoming(playback);
            let request_id = prepare_track(
                &mut snapshot,
                playback_generation,
                &playback.pending_transition,
                &track,
            );
            if !replace_queue_session(playback, &backend_session) {
                clear_pending_transition(playback, request_id);
                return (snapshot_event(snapshot.clone()), false);
            }
            let event = snapshot_event(snapshot.clone());
            drop(snapshot);

            spawn_playback(
                track,
                request_id,
                playback.clone(),
                Arc::clone(catalog),
                Arc::clone(audio),
                Arc::clone(storage),
                start,
                Some(backend_session),
            );
            (event, false)
        }
        ClientCommand::QueueSelected => {
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let _queue_guard = playback.queue_mutation.lock().await;
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let track = {
                let snapshot = state.lock().await;
                snapshot
                    .search
                    .results
                    .get(snapshot.search.selected_index)
                    .cloned()
            };
            let Some(track) = track else {
                return (snapshot_event(state.lock().await.clone()), false);
            };
            let Some(session_id) = queue_session_id(playback) else {
                return queue_error_snapshot(state, "no active backend queue session").await;
            };
            let session = match queue.add_queue(&session_id, track).await {
                Ok(session) => session,
                Err(error) => return queue_error_snapshot(state, &error.to_string()).await,
            };
            let mut snapshot = state.lock().await;
            if accept_queue_session(playback, &session) {
                project_queue_for(playback, &mut snapshot, &session);
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::PlayTrack(track) => {
            let _queue_guard = playback.queue_mutation.lock().await;
            let (start, queue_candidates) = {
                let snapshot = state.lock().await;
                let start = if matches!(
                    snapshot.playback.status,
                    PlaybackStatus::Playing | PlaybackStatus::Paused
                ) {
                    PlaybackStart::Crossfade(Duration::from_millis(500))
                } else {
                    PlaybackStart::Replace
                };
                (start, snapshot.playback.queue.clone())
            };
            let backend_session = match queue
                .start_queue(QueueStart {
                    tracks: queue_candidates.clone(),
                    track: Some(track.clone()),
                    current_index: 0,
                    endless_queue: true,
                    ..QueueStart::default()
                })
                .await
            {
                Ok(session) => session,
                Err(error) => {
                    let mut snapshot = state.lock().await;
                    snapshot.playback.error = Some(error.to_string());
                    snapshot.playback.status = PlaybackStatus::Failed;
                    return (snapshot_event(snapshot.clone()), false);
                }
            };
            let mut snapshot = state.lock().await;
            cancel_incoming(playback);
            let request_id = prepare_track(
                &mut snapshot,
                playback_generation,
                &playback.pending_transition,
                &track,
            );
            if !replace_queue_session(playback, &backend_session) {
                clear_pending_transition(playback, request_id);
                return (snapshot_event(snapshot.clone()), false);
            }
            let event = snapshot_event(snapshot.clone());
            drop(snapshot);
            spawn_playback(
                track,
                request_id,
                playback.clone(),
                Arc::clone(catalog),
                Arc::clone(audio),
                Arc::clone(storage),
                start,
                Some(backend_session),
            );
            (event, false)
        }
        ClientCommand::QueueTrack(track) => {
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let _queue_guard = playback.queue_mutation.lock().await;
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let Some(session_id) = queue_session_id(playback) else {
                return queue_error_snapshot(state, "no active backend queue session").await;
            };
            let session = match queue.add_queue(&session_id, track).await {
                Ok(session) => session,
                Err(error) => return queue_error_snapshot(state, &error.to_string()).await,
            };
            let mut snapshot = state.lock().await;
            if accept_queue_session(playback, &session) {
                project_queue_for(playback, &mut snapshot, &session);
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::ToggleLike(track) => {
            let result = storage
                .lock()
                .map_err(|_| "guest database lock was poisoned".to_owned())
                .and_then(|database| {
                    let liked = database
                        .is_liked(&track.id)
                        .map_err(|error| error.to_string())?;
                    database
                        .set_liked(&track, !liked)
                        .map_err(|error| error.to_string())?;
                    library_snapshot(&database).map_err(|error| error.to_string())
                });
            let mut snapshot = state.lock().await;
            match result {
                Ok(library) => snapshot.library = library,
                Err(error) => snapshot.playback.error = Some(error),
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::DownloadTrack(track) => {
            let result = storage
                .lock()
                .map_err(|_| "guest database lock was poisoned".to_owned())
                .and_then(|database| {
                    let existing = database
                        .download(&track.id)
                        .map_err(|error| error.to_string())?;
                    let should_spawn = !existing.is_some_and(|download| match download.state {
                        DownloadState::Queued | DownloadState::Downloading => true,
                        DownloadState::Available => download
                            .local_path
                            .as_deref()
                            .is_some_and(|path| Path::new(path).is_file()),
                        DownloadState::Failed => false,
                    });
                    if should_spawn {
                        database
                            .set_download(&track, DownloadState::Queued, None)
                            .map_err(|error| error.to_string())?;
                    }
                    library_snapshot(&database)
                        .map(|library| (library, should_spawn))
                        .map_err(|error| error.to_string())
                });
            let mut snapshot = state.lock().await;
            match result {
                Ok((library, should_spawn)) => {
                    snapshot.library = library;
                    if should_spawn {
                        spawn_download(
                            track,
                            Arc::clone(state),
                            Arc::clone(catalog),
                            Arc::clone(storage),
                            Arc::clone(download_directory),
                        );
                    }
                }
                Err(error) => snapshot.playback.error = Some(error),
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::ToggleLyrics => {
            let mut snapshot = state.lock().await;
            let Some(track) = snapshot.playback.current.clone() else {
                return (snapshot_event(snapshot.clone()), false);
            };
            snapshot.lyrics.visible = !snapshot.lyrics.visible;
            if snapshot.lyrics.visible
                && snapshot.lyrics.track_id.as_deref() != Some(track.id.as_str())
            {
                snapshot.lyrics.track_id = Some(track.id.clone());
                snapshot.lyrics.status = LyricsStatus::Loading;
                snapshot.lyrics.lines.clear();
                spawn_lyrics(
                    track,
                    Arc::clone(state),
                    Arc::clone(catalog),
                    Arc::clone(storage),
                );
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::SetCrossfadeSeconds(seconds) => {
            let seconds = seconds.min(12);
            let result = storage
                .lock()
                .map_err(|_| "guest database lock was poisoned".to_owned())
                .and_then(|database| {
                    database
                        .set_crossfade_seconds(seconds)
                        .map_err(|error| error.to_string())
                });
            let mut snapshot = state.lock().await;
            match result {
                Ok(()) => snapshot.settings.crossfade_seconds = seconds,
                Err(error) => snapshot.playback.error = Some(error),
            }
            (snapshot_event(snapshot.clone()), false)
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
                snapshot.playback.spectrum = [0; 24];
            } else if current == PlaybackStatus::Paused {
                snapshot.playback.status = PlaybackStatus::Playing;
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::NextTrack => {
            let _queue_guard = playback.queue_mutation.lock().await;
            let Some(session_id) = queue_session_id(playback) else {
                return queue_error_snapshot(state, "no active backend queue session").await;
            };
            let session = match queue.next_queue(&session_id).await {
                Ok(session) => session,
                Err(error) => {
                    cancel_incoming(playback);
                    let mut snapshot = state.lock().await;
                    let superseded = snapshot.playback.request_id;
                    snapshot.playback.request_id = snapshot.playback.request_id.saturating_add(1);
                    playback
                        .generation
                        .store(snapshot.playback.request_id, Ordering::Release);
                    clear_pending_transition(playback, superseded);
                    drop(snapshot);
                    return queue_error_snapshot(state, &error.to_string()).await;
                }
            };
            let Some(track) = session.tracks.get(session.current_index).cloned() else {
                return queue_error_snapshot(state, "backend queue returned no current track")
                    .await;
            };
            let mut snapshot = state.lock().await;
            if !accept_queue_session(playback, &session) {
                return (snapshot_event(snapshot.clone()), false);
            }
            let start = if matches!(
                snapshot.playback.status,
                PlaybackStatus::Playing | PlaybackStatus::Paused
            ) {
                PlaybackStart::Crossfade(Duration::from_millis(500))
            } else {
                PlaybackStart::Replace
            };
            cancel_incoming(playback);
            let request_id = prepare_track(
                &mut snapshot,
                playback_generation,
                &playback.pending_transition,
                &track,
            );
            let event = snapshot_event(snapshot.clone());
            drop(snapshot);
            spawn_playback(
                track,
                request_id,
                playback.clone(),
                Arc::clone(catalog),
                Arc::clone(audio),
                Arc::clone(storage),
                start,
                Some(session),
            );
            (event, false)
        }
        ClientCommand::PreviousTrack => {
            let _queue_guard = playback.queue_mutation.lock().await;
            let Some(session_id) = queue_session_id(playback) else {
                return queue_error_snapshot(state, "no active backend queue session").await;
            };
            let session = match queue.previous_queue(&session_id).await {
                Ok(session) => session,
                Err(error) => {
                    cancel_incoming(playback);
                    let mut snapshot = state.lock().await;
                    let superseded = snapshot.playback.request_id;
                    snapshot.playback.request_id = snapshot.playback.request_id.saturating_add(1);
                    playback
                        .generation
                        .store(snapshot.playback.request_id, Ordering::Release);
                    clear_pending_transition(playback, superseded);
                    drop(snapshot);
                    return queue_error_snapshot(state, &error.to_string()).await;
                }
            };
            let Some(track) = session.tracks.get(session.current_index).cloned() else {
                return queue_error_snapshot(state, "backend queue returned no current track")
                    .await;
            };
            let mut snapshot = state.lock().await;
            if !accept_queue_session(playback, &session) {
                return (snapshot_event(snapshot.clone()), false);
            }
            let start = if matches!(
                snapshot.playback.status,
                PlaybackStatus::Playing | PlaybackStatus::Paused
            ) {
                PlaybackStart::Crossfade(Duration::from_millis(500))
            } else {
                PlaybackStart::Replace
            };
            cancel_incoming(playback);
            let request_id = prepare_track(
                &mut snapshot,
                playback_generation,
                &playback.pending_transition,
                &track,
            );
            let event = snapshot_event(snapshot.clone());
            drop(snapshot);
            spawn_playback(
                track,
                request_id,
                playback.clone(),
                Arc::clone(catalog),
                Arc::clone(audio),
                Arc::clone(storage),
                start,
                Some(session),
            );
            (event, false)
        }
        ClientCommand::ToggleShuffle => {
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let _queue_guard = playback.queue_mutation.lock().await;
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let Some(session_id) = queue_session_id(playback) else {
                return queue_error_snapshot(state, "no active backend queue session").await;
            };
            let enabled = !state.lock().await.playback.shuffle;
            let session = match queue.set_shuffle_queue(&session_id, enabled).await {
                Ok(session) => session,
                Err(error) => return queue_error_snapshot(state, &error.to_string()).await,
            };
            let mut snapshot = state.lock().await;
            if accept_queue_session(playback, &session) {
                project_queue_for(playback, &mut snapshot, &session);
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::CycleRepeat => {
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let _queue_guard = playback.queue_mutation.lock().await;
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let Some(session_id) = queue_session_id(playback) else {
                return queue_error_snapshot(state, "no active backend queue session").await;
            };
            let mode = match state.lock().await.playback.repeat_mode {
                RepeatMode::Off => RepeatMode::All,
                RepeatMode::All => RepeatMode::One,
                RepeatMode::One => RepeatMode::Off,
            };
            let mode = match mode {
                RepeatMode::All => QueueRepeatMode::All,
                RepeatMode::One => QueueRepeatMode::One,
                RepeatMode::Off => QueueRepeatMode::None,
            };
            let session = match queue.set_repeat_queue(&session_id, mode).await {
                Ok(session) => session,
                Err(error) => return queue_error_snapshot(state, &error.to_string()).await,
            };
            let mut snapshot = state.lock().await;
            if accept_queue_session(playback, &session) {
                project_queue_for(playback, &mut snapshot, &session);
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::ToggleMute => {
            let (percent, muted) = {
                let snapshot = state.lock().await;
                if snapshot.playback.muted {
                    (snapshot.playback.volume_before_mute.max(1), false)
                } else {
                    (0, true)
                }
            };
            let result = run_audio(Arc::clone(audio), move |audio| {
                audio.set_volume(f32::from(percent) / 100.0)
            })
            .await;
            let mut snapshot = state.lock().await;
            if let Err(error) = result {
                set_playback_error(&mut snapshot, error);
            } else {
                if muted && snapshot.playback.volume_percent > 0 {
                    snapshot.playback.volume_before_mute = snapshot.playback.volume_percent;
                }
                snapshot.playback.volume_percent = percent;
                snapshot.playback.muted = muted;
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::SeekTo(position_ms) => seek_to(position_ms, state, audio).await,
        ClientCommand::PlayQueueIndex(index) => {
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let _queue_guard = playback.queue_mutation.lock().await;
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let Some((session_id, revision)) = queue_session_info(playback) else {
                return queue_error_snapshot(state, "no active backend queue session").await;
            };
            let Some(backend_index) = visible_backend_index(playback, &session_id, revision, index)
            else {
                return queue_error_snapshot(state, "queue projection is stale").await;
            };
            let session = match queue.play_index_queue(&session_id, backend_index).await {
                Ok(session) => session,
                Err(error) => return queue_error_snapshot(state, &error.to_string()).await,
            };
            let Some(track) = session.tracks.get(session.current_index).cloned() else {
                return queue_error_snapshot(state, "backend queue returned no current track")
                    .await;
            };
            let mut snapshot = state.lock().await;
            if !accept_queue_session(playback, &session) {
                return (snapshot_event(snapshot.clone()), false);
            }
            cancel_incoming(playback);
            let request_id = prepare_track(
                &mut snapshot,
                playback_generation,
                &playback.pending_transition,
                &track,
            );
            let event = snapshot_event(snapshot.clone());
            drop(snapshot);
            spawn_playback(
                track,
                request_id,
                playback.clone(),
                Arc::clone(catalog),
                Arc::clone(audio),
                Arc::clone(storage),
                PlaybackStart::Crossfade(Duration::from_millis(500)),
                Some(session),
            );
            (event, false)
        }
        ClientCommand::RemoveQueueIndex(index) => {
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let _queue_guard = playback.queue_mutation.lock().await;
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let Some((session_id, revision)) = queue_session_info(playback) else {
                return queue_error_snapshot(state, "no active backend queue session").await;
            };
            let Some(backend_index) = visible_backend_index(playback, &session_id, revision, index)
            else {
                return queue_error_snapshot(state, "queue projection is stale").await;
            };
            let session = match queue.remove_queue(&session_id, backend_index).await {
                Ok(session) => session,
                Err(error) => return queue_error_snapshot(state, &error.to_string()).await,
            };
            let mut snapshot = state.lock().await;
            if accept_queue_session(playback, &session) {
                project_queue_for(playback, &mut snapshot, &session);
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::ClearQueue => {
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let _queue_guard = playback.queue_mutation.lock().await;
            if transition_pending(playback) {
                return queue_error_snapshot(state, "playback transition is pending").await;
            }
            let Some(session_id) = queue_session_id(playback) else {
                return queue_error_snapshot(state, "no active backend queue session").await;
            };
            let session = match queue.clear_upcoming_queue(&session_id).await {
                Ok(session) => session,
                Err(error) => return queue_error_snapshot(state, &error.to_string()).await,
            };
            let mut snapshot = state.lock().await;
            if accept_queue_session(playback, &session) {
                project_queue_for(playback, &mut snapshot, &session);
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
                if percent > 0 {
                    snapshot.playback.volume_before_mute = percent;
                    snapshot.playback.muted = false;
                } else {
                    snapshot.playback.muted = true;
                }
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::Shutdown => (DaemonEvent::Acknowledged, true),
    }
}

async fn seek_to(
    position_ms: u64,
    state: &Arc<Mutex<AppSnapshot>>,
    audio: &SharedAudio,
) -> (DaemonEvent, bool) {
    let target = {
        let snapshot = state.lock().await;
        position_ms.min(snapshot.playback.duration_ms)
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

async fn queue_error_snapshot(state: &Arc<Mutex<AppSnapshot>>, error: &str) -> (DaemonEvent, bool) {
    let mut snapshot = state.lock().await;
    snapshot.playback.error = Some(error.to_owned());
    if snapshot.playback.current.is_none() {
        snapshot.playback.status = PlaybackStatus::Failed;
    }
    (snapshot_event(snapshot.clone()), false)
}

fn prepare_track(
    snapshot: &mut AppSnapshot,
    playback_generation: &AtomicU64,
    pending_transition: &AtomicU64,
    track: &Track,
) -> u64 {
    snapshot.playback.request_id = snapshot.playback.request_id.saturating_add(1);
    playback_generation.store(snapshot.playback.request_id, Ordering::Release);
    pending_transition.store(snapshot.playback.request_id, Ordering::Release);
    if snapshot.playback.current.is_none() {
        snapshot.playback.duration_ms = track.duration_ms;
        snapshot.playback.error = None;
        snapshot.playback.status = PlaybackStatus::Resolving;
    }
    snapshot.playback.request_id
}

fn clear_pending_transition(playback: &PlaybackCoordinator, request_id: u64) {
    let _ = playback.pending_transition.compare_exchange(
        request_id,
        0,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

fn transition_pending(playback: &PlaybackCoordinator) -> bool {
    playback.pending_transition.load(Ordering::Acquire) != 0
}

fn queue_is_endless(playback: &PlaybackCoordinator) -> bool {
    playback.queue_endless.lock().is_ok_and(|endless| *endless)
}

fn refill_allowed(
    playback: &PlaybackCoordinator,
    session_id: &str,
    revision: i64,
    visible_count: usize,
) -> bool {
    let Ok(mut marker) = playback.queue_refill_marker.lock() else {
        return false;
    };
    let blocked = marker.as_ref().is_some_and(|marker| {
        marker.session_id == session_id
            && marker.revision == revision
            && visible_count == marker.visible_count
    });
    if !blocked {
        *marker = None;
    }
    if blocked {
        return false;
    }
    let Ok(mut backoff) = playback.queue_refill_backoff.lock() else {
        return false;
    };
    if let Some(current) = backoff.as_ref()
        && current.session_id == session_id
        && current.revision == revision
    {
        if current.retry_at > Instant::now() {
            return false;
        }
        *backoff = None;
    }
    true
}

fn record_refill_result(
    playback: &PlaybackCoordinator,
    session_id: &str,
    revision: i64,
    previous_count: usize,
    updated_count: usize,
) {
    let Ok(mut marker) = playback.queue_refill_marker.lock() else {
        return;
    };
    if updated_count > previous_count || updated_count >= MAX_VISIBLE_QUEUE {
        *marker = None;
    } else {
        *marker = Some(RefillMarker {
            session_id: session_id.to_owned(),
            revision,
            visible_count: updated_count,
        });
    }
}

fn record_refill_error(playback: &PlaybackCoordinator, session_id: &str, revision: i64) {
    if let Ok(mut marker) = playback.queue_refill_backoff.lock() {
        let attempts = marker
            .as_ref()
            .filter(|marker| marker.session_id == session_id && marker.revision == revision)
            .map_or(1, |marker| marker.attempts.saturating_add(1));
        let delay_seconds = 2_u64.saturating_pow(attempts.saturating_sub(1).min(5));
        *marker = Some(RefillBackoff {
            session_id: session_id.to_owned(),
            revision,
            attempts,
            retry_at: Instant::now() + Duration::from_secs(delay_seconds.min(8)),
        });
    }
}

fn clear_queue_refill_markers(playback: &PlaybackCoordinator) {
    if let Ok(mut marker) = playback.queue_refill_marker.lock() {
        *marker = None;
    }
    if let Ok(mut marker) = playback.queue_refill_backoff.lock() {
        *marker = None;
    }
}

fn auto_advance_allowed(
    playback: &PlaybackCoordinator,
    session_id: &str,
    revision: i64,
    current_index: usize,
) -> bool {
    let Ok(marker) = playback.auto_advance_marker.lock() else {
        return false;
    };
    let Some(marker) = marker.as_ref() else {
        return true;
    };
    if marker.session_id != session_id
        || marker.revision != revision
        || marker.current_index != current_index
    {
        return true;
    }
    !marker.terminal && marker.retry_at <= Instant::now()
}

fn record_auto_advance_error(
    playback: &PlaybackCoordinator,
    session_id: &str,
    revision: i64,
    current_index: usize,
    terminal: bool,
) {
    if let Ok(mut marker) = playback.auto_advance_marker.lock() {
        let attempts = marker
            .as_ref()
            .filter(|marker| {
                marker.session_id == session_id
                    && marker.revision == revision
                    && marker.current_index == current_index
            })
            .map_or(1, |marker| marker.attempts.saturating_add(1));
        let terminal = terminal || attempts >= 6;
        let delay_seconds = 2_u64.saturating_pow(attempts.saturating_sub(1).min(5));
        *marker = Some(AutoAdvanceMarker {
            session_id: session_id.to_owned(),
            revision,
            current_index,
            attempts,
            terminal,
            retry_at: Instant::now() + Duration::from_secs(delay_seconds.min(8)),
        });
    }
}

fn clear_auto_advance_marker(playback: &PlaybackCoordinator) {
    if let Ok(mut marker) = playback.auto_advance_marker.lock() {
        *marker = None;
    }
}

fn is_terminal_queue_error(error: &CatalogError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("ended")
        || message.contains("not found")
        || message.contains("missing")
        || message.contains("empty")
}

fn queue_session_id(playback: &PlaybackCoordinator) -> Option<String> {
    playback
        .queue_session
        .lock()
        .ok()
        .and_then(|session| session.as_ref().map(|(id, _)| id.clone()))
}

fn queue_session_info(playback: &PlaybackCoordinator) -> Option<(String, i64)> {
    playback
        .queue_session
        .lock()
        .ok()
        .and_then(|session| session.clone())
}

fn cancel_incoming(playback: &PlaybackCoordinator) {
    #[cfg(target_os = "linux")]
    if let Some(handle) = &playback.cancellation {
        handle();
    }
}

fn accept_queue_session(playback: &PlaybackCoordinator, session: &QueueSession) -> bool {
    let Ok(mut current) = playback.queue_session.lock() else {
        return false;
    };
    let new_session = current.as_ref().is_none_or(|(id, _)| id != &session.id);
    let newer_revision = current
        .as_ref()
        .is_some_and(|(id, revision)| id == &session.id && session.revision > *revision);
    if current
        .as_ref()
        .is_some_and(|(id, revision)| id != &session.id || session.revision < *revision)
    {
        return false;
    }
    *current = Some((session.id.clone(), session.revision));
    if (new_session || newer_revision) && !playback.queue_refill_in_flight.load(Ordering::Acquire) {
        clear_queue_refill_markers(playback);
        clear_auto_advance_marker(playback);
    }
    if let Ok(mut endless) = playback.queue_endless.lock() {
        *endless = session.endless_queue;
    }
    if let Ok(mut current_index) = playback.queue_current_index.lock() {
        *current_index = session.current_index;
    }
    true
}

fn replace_queue_session(playback: &PlaybackCoordinator, session: &QueueSession) -> bool {
    let Ok(mut current) = playback.queue_session.lock() else {
        return false;
    };
    clear_queue_refill_markers(playback);
    clear_auto_advance_marker(playback);
    *current = Some((session.id.clone(), session.revision));
    if let Ok(mut endless) = playback.queue_endless.lock() {
        *endless = session.endless_queue;
    }
    if let Ok(mut current_index) = playback.queue_current_index.lock() {
        *current_index = session.current_index;
    }
    true
}

fn project_queue(snapshot: &mut AppSnapshot, session: &QueueSession) {
    let current_index = session
        .current_index
        .min(session.tracks.len().saturating_sub(1));
    let order = if session.shuffle && session.play_order.len() == session.tracks.len() {
        session.play_order.clone()
    } else {
        (0..session.tracks.len()).collect::<Vec<_>>()
    };
    snapshot.playback.shuffle = session.shuffle;
    snapshot.playback.repeat_mode = match session.repeat_mode {
        QueueRepeatMode::All => RepeatMode::All,
        QueueRepeatMode::One => RepeatMode::One,
        QueueRepeatMode::None => RepeatMode::Off,
    };
    snapshot.playback.queue = order
        .iter()
        .skip_while(|index| **index != current_index)
        .skip(1)
        .filter_map(|index| session.tracks.get(*index).cloned())
        .take(MAX_VISIBLE_QUEUE)
        .collect();
}

fn project_queue_for(
    playback: &PlaybackCoordinator,
    snapshot: &mut AppSnapshot,
    session: &QueueSession,
) {
    project_queue(snapshot, session);
    let current_index = session
        .current_index
        .min(session.tracks.len().saturating_sub(1));
    let order = if session.shuffle && session.play_order.len() == session.tracks.len() {
        session.play_order.clone()
    } else {
        (0..session.tracks.len()).collect::<Vec<_>>()
    };
    let indices = order
        .iter()
        .skip_while(|index| **index != current_index)
        .skip(1)
        .take(MAX_VISIBLE_QUEUE)
        .copied()
        .collect::<Vec<_>>();
    if let Ok(mut projection) = playback.queue_projection.lock() {
        *projection = Some(QueueProjection {
            session_id: session.id.clone(),
            revision: session.revision,
            indices,
        });
    }
}

fn visible_backend_index(
    playback: &PlaybackCoordinator,
    session_id: &str,
    revision: i64,
    visible_index: usize,
) -> Option<usize> {
    let projection = playback.queue_projection.lock().ok()?;
    let projection = projection.as_ref()?;
    if projection.session_id != session_id || projection.revision != revision {
        return None;
    }
    projection.indices.get(visible_index).copied()
}

fn commit_playback_current(snapshot: &mut AppSnapshot, track: &Track) {
    let previous = snapshot.playback.current.replace(track.clone());
    if let Some(previous) = previous
        && previous.id != track.id
        && snapshot.playback.history.last().map(|item| &item.id) != Some(&previous.id)
    {
        snapshot.playback.history.push(previous);
        if snapshot.playback.history.len() > 100 {
            snapshot.playback.history.remove(0);
        }
    }
}

fn snapshot_event(snapshot: AppSnapshot) -> DaemonEvent {
    DaemonEvent::Snapshot(Box::new(snapshot))
}

#[allow(clippy::too_many_arguments)]
fn spawn_playback(
    track: Track,
    request_id: u64,
    playback: PlaybackCoordinator,
    catalog: Arc<dyn MusicCatalog>,
    audio: SharedAudio,
    storage: SharedStorage,
    start: PlaybackStart,
    queue_session: Option<QueueSession>,
) {
    tokio::spawn(async move {
        let state = &playback.state;
        spawn_prewarm(
            playback.clone(),
            Arc::clone(&catalog),
            Arc::clone(&storage),
            &track,
            queue_session.as_ref(),
        );
        let local_path = storage.lock().ok().and_then(|database| {
            database
                .download(&track.id)
                .ok()
                .flatten()
                .and_then(|download| {
                    (download.state == DownloadState::Available)
                        .then_some(download.local_path)
                        .flatten()
                        .filter(|path| Path::new(path).is_file())
                })
        });
        let stream = if let Some(path) = local_path {
            ResolvedStream::new(path)
        } else {
            match playback
                .stream_cache
                .resolve(catalog.as_ref(), &track.id, AudioQuality::Automatic)
                .await
            {
                Ok(stream) => stream,
                Err(error) => {
                    finish_playback_error(&playback, request_id, error.to_string()).await;
                    return;
                }
            }
        };
        {
            let mut snapshot = state.lock().await;
            if snapshot.playback.request_id != request_id {
                clear_pending_transition(&playback, request_id);
                return;
            }
            if snapshot.playback.current.is_none() {
                snapshot.playback.status = PlaybackStatus::Buffering;
            }
        }
        let mut source = StreamSource::new(stream.url);
        source.headers = stream.headers;
        let operation_generation = Arc::clone(&playback.generation);
        let result = run_playback_audio(
            audio,
            Arc::clone(&playback.generation),
            request_id,
            move |audio| match start {
                PlaybackStart::Replace => {
                    if operation_generation.load(Ordering::Acquire) != request_id {
                        return Ok(());
                    }
                    audio.stop()?;
                    audio.load(&source)?;
                    if operation_generation.load(Ordering::Acquire) != request_id {
                        return audio.stop();
                    }
                    audio.play()?;
                    if operation_generation.load(Ordering::Acquire) != request_id {
                        return audio.stop();
                    }
                    Ok(())
                }
                PlaybackStart::Crossfade(duration) => {
                    audio.transition_to_guarded(&source, duration, &|| {
                        operation_generation.load(Ordering::Acquire) == request_id
                    })
                }
            },
        )
        .await;
        let mut snapshot = state.lock().await;
        if snapshot.playback.request_id != request_id {
            clear_pending_transition(&playback, request_id);
            return;
        }
        match result {
            PlaybackRun::Completed(Ok(())) => {
                if let Some(session) = queue_session.as_ref() {
                    project_queue_for(&playback, &mut snapshot, session);
                }
                commit_playback_current(&mut snapshot, &track);
                snapshot.playback.status = PlaybackStatus::Playing;
                snapshot.playback.error = None;
                if let Ok(database) = storage.lock()
                    && database.record_play(&track, unix_time()).is_ok()
                    && let Ok(library) = library_snapshot(&database)
                {
                    snapshot.library = library;
                }
                if snapshot.lyrics.track_id.as_deref() != Some(track.id.as_str()) {
                    snapshot.lyrics.track_id = Some(track.id.clone());
                    snapshot.lyrics.status = LyricsStatus::Loading;
                    snapshot.lyrics.lines.clear();
                    spawn_lyrics(
                        track,
                        Arc::clone(state),
                        Arc::clone(&catalog),
                        Arc::clone(&storage),
                    );
                }
                clear_pending_transition(&playback, request_id);
            }
            PlaybackRun::Completed(Err(error)) => {
                set_playback_error(&mut snapshot, error);
                clear_pending_transition(&playback, request_id);
            }
            PlaybackRun::Stale => clear_pending_transition(&playback, request_id),
        }
    });
}

fn spawn_prewarm(
    playback: PlaybackCoordinator,
    catalog: Arc<dyn MusicCatalog>,
    storage: SharedStorage,
    current: &Track,
    session: Option<&QueueSession>,
) {
    let mut tracks = vec![current.clone()];
    if let Some(session) = session {
        let current_index = session
            .current_index
            .min(session.tracks.len().saturating_sub(1));
        let order = if session.shuffle && session.play_order.len() == session.tracks.len() {
            session.play_order.clone()
        } else {
            (0..session.tracks.len()).collect::<Vec<_>>()
        };
        tracks.extend(
            order
                .iter()
                .skip_while(|index| **index != current_index)
                .skip(1)
                .filter_map(|index| session.tracks.get(*index).cloned())
                .take(3),
        );
    } else if let Ok(snapshot) = playback.state.try_lock() {
        tracks.extend(snapshot.playback.queue.iter().take(3).cloned());
    }
    let mut seen = std::collections::HashSet::new();
    for track in tracks {
        if !seen.insert(track.id.clone()) {
            continue;
        }
        let local = storage.lock().ok().and_then(|database| {
            database
                .download(&track.id)
                .ok()
                .flatten()
                .and_then(|download| {
                    (download.state == DownloadState::Available)
                        .then_some(download.local_path)
                        .flatten()
                        .filter(|path| Path::new(path).is_file())
                })
        });
        if local.is_some() {
            continue;
        }
        let cache = Arc::clone(&playback.stream_cache);
        let catalog = Arc::clone(&catalog);
        tokio::spawn(async move {
            let _ = cache
                .resolve(catalog.as_ref(), &track.id, AudioQuality::Automatic)
                .await;
        });
    }
}

fn spawn_lyrics(
    track: Track,
    state: Arc<Mutex<AppSnapshot>>,
    catalog: Arc<dyn MusicCatalog>,
    storage: SharedStorage,
) {
    tokio::spawn(async move {
        let cached = storage
            .lock()
            .ok()
            .and_then(|database| database.lyrics(&track.id).ok().flatten());
        let result = if let Some(lyrics) = cached {
            Ok(Some(lyrics))
        } else {
            let result = catalog.lyrics(&track).await;
            if let Ok(Some(lyrics)) = &result
                && let Ok(database) = storage.lock()
            {
                let _ = database.save_lyrics(&track, lyrics);
            }
            result
        };
        let mut snapshot = state.lock().await;
        if snapshot.lyrics.track_id.as_deref() != Some(track.id.as_str()) {
            return;
        }
        match result {
            Ok(Some(lyrics)) => {
                snapshot.lyrics.synced = lyrics.synced;
                snapshot.lyrics.lines = lyrics
                    .lines
                    .into_iter()
                    .map(|line| LyricsLineSnapshot {
                        start_ms: line.start_ms,
                        words: line.words,
                    })
                    .collect();
                snapshot.lyrics.status = LyricsStatus::Ready;
            }
            Ok(None) => snapshot.lyrics.status = LyricsStatus::Unavailable,
            Err(error) => snapshot.lyrics.status = LyricsStatus::Failed(error.to_string()),
        }
    });
}

#[derive(Clone, Copy)]
enum PlaybackStart {
    Replace,
    Crossfade(Duration),
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

enum PlaybackRun {
    Completed(Result<(), String>),
    Stale,
}

async fn run_playback_audio(
    audio: SharedAudio,
    playback_generation: Arc<AtomicU64>,
    request_id: u64,
    operation: impl FnOnce(&mut dyn AudioBackend) -> Result<(), BackendError> + Send + 'static,
) -> PlaybackRun {
    match tokio::task::spawn_blocking(move || {
        let mut audio = audio
            .lock()
            .map_err(|_| "audio engine lock was poisoned".to_owned())?;
        if playback_generation.load(Ordering::Acquire) != request_id {
            return Ok(None);
        }
        operation(audio.as_mut())
            .map_err(|error| error.to_string())
            .map(Some)
    })
    .await
    {
        Ok(Ok(Some(()))) => PlaybackRun::Completed(Ok(())),
        Ok(Ok(None)) => PlaybackRun::Stale,
        Ok(Err(error)) => PlaybackRun::Completed(Err(error)),
        Err(error) => PlaybackRun::Completed(Err(format!("audio worker failed: {error}"))),
    }
}

async fn finish_playback_error(playback: &PlaybackCoordinator, request_id: u64, error: String) {
    let mut snapshot = playback.state.lock().await;
    if snapshot.playback.request_id == request_id {
        set_playback_error(&mut snapshot, error);
        clear_pending_transition(playback, request_id);
    }
}

fn set_playback_error(snapshot: &mut AppSnapshot, error: String) {
    if snapshot.playback.current.is_none() {
        snapshot.playback.status = PlaybackStatus::Failed;
    }
    snapshot.playback.error = Some(error);
}

async fn refresh_playback_telemetry(
    playback: &PlaybackCoordinator,
    catalog: &Arc<dyn MusicCatalog>,
    queue: &Arc<dyn MusicQueue>,
    audio: &SharedAudio,
    storage: &SharedStorage,
) {
    let state = &playback.state;
    let telemetry = match audio.try_lock() {
        Ok(audio) => audio.telemetry(),
        Err(_) => return,
    };
    let _queue_guard = playback.queue_mutation.lock().await;
    if let Some((session_id, session_revision)) = queue_session_info(playback) {
        let current_index = playback
            .queue_current_index
            .lock()
            .map_or(0, |index| *index);
        let visible_count = state.lock().await.playback.queue.len();
        let current_track_id = state
            .lock()
            .await
            .playback
            .current
            .as_ref()
            .map(|track| track.id.clone());
        let should_refill = !transition_pending(playback)
            && queue_is_endless(playback)
            && visible_count < MAX_VISIBLE_QUEUE
            && refill_allowed(playback, &session_id, session_revision, visible_count);
        if should_refill {
            playback
                .queue_refill_in_flight
                .store(true, Ordering::Release);
            match queue.load_more_queue(&session_id).await {
                Ok(session) => {
                    let mut snapshot = state.lock().await;
                    let mut updated_count = visible_count;
                    let result_revision = session.revision;
                    if accept_queue_session(playback, &session) {
                        project_queue_for(playback, &mut snapshot, &session);
                        updated_count = snapshot.playback.queue.len();
                    }
                    record_refill_result(
                        playback,
                        &session_id,
                        result_revision,
                        visible_count,
                        updated_count,
                    );
                }
                Err(_) => record_refill_error(playback, &session_id, session_revision),
            }
            playback
                .queue_refill_in_flight
                .store(false, Ordering::Release);
        }
        let should_advance = {
            let snapshot = state.lock().await;
            !transition_pending(playback)
                && snapshot.playback.status == PlaybackStatus::Playing
                && snapshot.playback.error.is_none()
                && (telemetry.ended
                    || (snapshot.settings.crossfade_seconds > 0
                        && !snapshot.playback.queue.is_empty()
                        && telemetry.duration_ms > 0
                        && telemetry.position_ms.saturating_add(
                            u64::from(snapshot.settings.crossfade_seconds)
                                .saturating_add(2)
                                .saturating_mul(1_000),
                        ) >= telemetry.duration_ms))
        };
        if should_advance
            && auto_advance_allowed(playback, &session_id, session_revision, current_index)
        {
            match queue.next_queue(&session_id).await {
                Ok(session) => {
                    let canonical_track_id = session
                        .tracks
                        .get(session.current_index)
                        .map(|track| track.id.as_str());
                    if session.repeat_mode == QueueRepeatMode::None
                        && session.current_index == current_index
                        && canonical_track_id == current_track_id.as_deref()
                    {
                        record_auto_advance_error(
                            playback,
                            &session_id,
                            session_revision,
                            current_index,
                            true,
                        );
                        return;
                    }
                    clear_auto_advance_marker(playback);
                    let Some(track) = session.tracks.get(session.current_index).cloned() else {
                        return;
                    };
                    let mut snapshot = state.lock().await;
                    if accept_queue_session(playback, &session) {
                        cancel_incoming(playback);
                        let request_id = prepare_track(
                            &mut snapshot,
                            &playback.generation,
                            &playback.pending_transition,
                            &track,
                        );
                        drop(snapshot);
                        spawn_playback(
                            track,
                            request_id,
                            playback.clone(),
                            Arc::clone(catalog),
                            Arc::clone(audio),
                            Arc::clone(storage),
                            if telemetry.ended {
                                PlaybackStart::Replace
                            } else {
                                PlaybackStart::Crossfade(Duration::from_secs(1))
                            },
                            Some(session),
                        );
                    }
                }
                Err(error) => record_auto_advance_error(
                    playback,
                    &session_id,
                    session_revision,
                    current_index,
                    is_terminal_queue_error(&error),
                ),
            }
            if telemetry.ended {
                return;
            }
        }
    }
    let mut snapshot = state.lock().await;
    if matches!(
        snapshot.playback.status,
        PlaybackStatus::Playing | PlaybackStatus::Paused
    ) {
        snapshot.playback.position_ms = telemetry.position_ms;
        snapshot.playback.duration_ms = telemetry.duration_ms.max(snapshot.playback.duration_ms);
        snapshot.playback.buffered_ms = telemetry.buffered_ms;
        snapshot.playback.spectrum = telemetry.spectrum;
        snapshot.playback.underrun_count = telemetry.underrun_count;
    }
}

pub(crate) fn library_snapshot(
    database: &Database,
) -> Result<LibrarySnapshot, zerobeat_storage::StorageError> {
    Ok(LibrarySnapshot {
        liked: database.liked_tracks()?,
        recent: database.recent_tracks(50)?,
        downloads: database
            .downloads()?
            .into_iter()
            .map(|download| DownloadSnapshot {
                track: download.track,
                status: match download.state {
                    DownloadState::Queued => DownloadStatus::Queued,
                    DownloadState::Downloading => DownloadStatus::Downloading,
                    DownloadState::Available => DownloadStatus::Available,
                    DownloadState::Failed => DownloadStatus::Failed,
                },
                error: download.error,
            })
            .collect(),
    })
}

fn unix_time() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

struct UnavailableCatalog;

struct UnavailableAudio;

struct UnavailableQueue;

fn unavailable_queue<T>() -> QueueFuture<'static, T> {
    Box::pin(async {
        Err(CatalogError::Unavailable(
            "player queue is not configured".into(),
        ))
    })
}

impl MusicQueue for UnavailableQueue {
    fn active_queue(&self) -> QueueFuture<'_, Option<QueueSession>> {
        Box::pin(async { Ok(None) })
    }

    fn start_queue(&self, _request: QueueStart) -> QueueFuture<'_, QueueSession> {
        unavailable_queue()
    }
    fn get_queue(&self, _session_id: &str) -> QueueFuture<'_, QueueSession> {
        unavailable_queue()
    }
    fn delete_queue(&self, _session_id: &str) -> QueueFuture<'_, ()> {
        unavailable_queue()
    }
    fn next_queue(&self, _session_id: &str) -> QueueFuture<'_, QueueSession> {
        unavailable_queue()
    }
    fn previous_queue(&self, _session_id: &str) -> QueueFuture<'_, QueueSession> {
        unavailable_queue()
    }
    fn load_more_queue(&self, _session_id: &str) -> QueueFuture<'_, QueueSession> {
        unavailable_queue()
    }
    fn play_next_queue(&self, _session_id: &str, _track: Track) -> QueueFuture<'_, QueueSession> {
        unavailable_queue()
    }
    fn add_queue(&self, _session_id: &str, _track: Track) -> QueueFuture<'_, QueueSession> {
        unavailable_queue()
    }
    fn play_index_queue(&self, _session_id: &str, _index: usize) -> QueueFuture<'_, QueueSession> {
        unavailable_queue()
    }
    fn remove_queue(&self, _session_id: &str, _index: usize) -> QueueFuture<'_, QueueSession> {
        unavailable_queue()
    }
    fn clear_upcoming_queue(&self, _session_id: &str) -> QueueFuture<'_, QueueSession> {
        unavailable_queue()
    }
    fn set_shuffle_queue(
        &self,
        _session_id: &str,
        _enabled: bool,
    ) -> QueueFuture<'_, QueueSession> {
        unavailable_queue()
    }
    fn set_repeat_queue(
        &self,
        _session_id: &str,
        _mode: QueueRepeatMode,
    ) -> QueueFuture<'_, QueueSession> {
        unavailable_queue()
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    fn session(revision: i64, shuffle: bool) -> QueueSession {
        let tracks = (0..16)
            .map(|index| Track::new(index.to_string(), format!("Track {index}"), "Artist", 1_000))
            .collect::<Vec<_>>();
        QueueSession {
            id: "session".into(),
            tracks,
            current_index: 4,
            play_order: if shuffle {
                vec![4, 9, 2, 8, 1, 3, 5, 6, 7, 10, 11, 12, 13, 14, 15, 0]
            } else {
                Vec::new()
            },
            shuffle,
            revision,
            ..QueueSession::default()
        }
    }

    #[test]
    fn queue_projection_follows_backend_order_and_caps_visible_upcoming() {
        let mut snapshot = AppSnapshot::default();
        project_queue(&mut snapshot, &session(2, true));
        assert_eq!(snapshot.playback.queue.len(), MAX_VISIBLE_QUEUE);
        assert_eq!(snapshot.playback.queue[0].id, "9");
        assert_eq!(snapshot.playback.queue[11].id, "13");
    }

    #[test]
    fn visible_queue_index_resolves_to_authoritative_backend_index() {
        let playback = PlaybackCoordinator {
            state: Arc::new(Mutex::new(AppSnapshot::default())),
            generation: Arc::new(AtomicU64::new(0)),
            pending_transition: Arc::new(AtomicU64::new(0)),
            queue_session: Arc::new(StdMutex::new(None)),
            queue_endless: Arc::new(StdMutex::new(false)),
            queue_current_index: Arc::new(StdMutex::new(0)),
            queue_projection: Arc::new(StdMutex::new(None)),
            queue_refill_marker: Arc::new(StdMutex::new(None)),
            queue_refill_backoff: Arc::new(StdMutex::new(None)),
            queue_refill_in_flight: Arc::new(AtomicBool::new(false)),
            auto_advance_marker: Arc::new(StdMutex::new(None)),
            queue_mutation: Arc::new(Mutex::new(())),
            stream_cache: Arc::new(StreamCache::new()),
            #[cfg(target_os = "linux")]
            cancellation: None,
        };
        let mut snapshot = AppSnapshot::default();
        let authoritative = session(2, true);
        project_queue_for(&playback, &mut snapshot, &authoritative);
        assert_eq!(snapshot.playback.queue[0].id, "9");
        assert_eq!(visible_backend_index(&playback, "session", 2, 0), Some(9));
        assert_eq!(visible_backend_index(&playback, "session", 2, 11), Some(13));
        assert_eq!(visible_backend_index(&playback, "session", 1, 0), None);
    }

    #[test]
    fn stale_queue_revision_is_ignored() {
        let playback = PlaybackCoordinator {
            state: Arc::new(Mutex::new(AppSnapshot::default())),
            generation: Arc::new(AtomicU64::new(0)),
            pending_transition: Arc::new(AtomicU64::new(0)),
            queue_session: Arc::new(StdMutex::new(Some(("session".into(), 5)))),
            queue_endless: Arc::new(StdMutex::new(false)),
            queue_current_index: Arc::new(StdMutex::new(0)),
            queue_projection: Arc::new(StdMutex::new(None)),
            queue_refill_marker: Arc::new(StdMutex::new(None)),
            queue_refill_backoff: Arc::new(StdMutex::new(None)),
            queue_refill_in_flight: Arc::new(AtomicBool::new(false)),
            auto_advance_marker: Arc::new(StdMutex::new(None)),
            queue_mutation: Arc::new(Mutex::new(())),
            stream_cache: Arc::new(StreamCache::new()),
            #[cfg(target_os = "linux")]
            cancellation: None,
        };
        assert!(!accept_queue_session(&playback, &session(4, false)));
        assert!(accept_queue_session(&playback, &session(6, false)));
        let mut wrong_session = session(7, false);
        wrong_session.id = "other".into();
        assert!(!accept_queue_session(&playback, &wrong_session));
    }

    struct PrewarmCatalog {
        calls: Arc<StdMutex<Vec<String>>>,
    }

    impl MusicCatalog for PrewarmCatalog {
        fn search_songs(&self, _request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn resolve_stream(
            &self,
            track_id: &str,
            _quality: AudioQuality,
        ) -> CatalogFuture<'_, ResolvedStream> {
            self.calls.lock().unwrap().push(track_id.to_owned());
            let track_id = track_id.to_owned();
            Box::pin(async move { Ok(ResolvedStream::new(format!("https://stream/{track_id}"))) })
        }
    }

    #[tokio::test]
    async fn initial_candidate_session_prewarms_current_and_next_three() {
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let catalog: Arc<dyn MusicCatalog> = Arc::new(PrewarmCatalog {
            calls: Arc::clone(&calls),
        });
        let playback = PlaybackCoordinator {
            state: Arc::new(Mutex::new(AppSnapshot::default())),
            generation: Arc::new(AtomicU64::new(0)),
            pending_transition: Arc::new(AtomicU64::new(0)),
            queue_session: Arc::new(StdMutex::new(None)),
            queue_endless: Arc::new(StdMutex::new(false)),
            queue_current_index: Arc::new(StdMutex::new(0)),
            queue_projection: Arc::new(StdMutex::new(None)),
            queue_refill_marker: Arc::new(StdMutex::new(None)),
            queue_refill_backoff: Arc::new(StdMutex::new(None)),
            queue_refill_in_flight: Arc::new(AtomicBool::new(false)),
            auto_advance_marker: Arc::new(StdMutex::new(None)),
            queue_mutation: Arc::new(Mutex::new(())),
            stream_cache: Arc::new(StreamCache::new()),
            #[cfg(target_os = "linux")]
            cancellation: None,
        };
        let storage = Arc::new(StdMutex::new(Database::open_in_memory().unwrap()));
        let tracks = (0..4)
            .map(|index| Track::new(format!("track-{index}"), "Track", "Artist", 1_000))
            .collect::<Vec<_>>();
        let session = QueueSession {
            id: "candidate".into(),
            tracks: tracks.clone(),
            current_index: 0,
            play_order: (0..4).collect(),
            revision: 1,
            ..QueueSession::default()
        };

        spawn_prewarm(playback, catalog, storage, &tracks[0], Some(&session));
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if calls.lock().unwrap().len() == 4 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("prewarm did not resolve current plus next three");
        let mut resolved = calls.lock().unwrap().clone();
        resolved.sort();
        assert_eq!(
            resolved,
            vec![
                "track-0".to_owned(),
                "track-1".to_owned(),
                "track-2".to_owned(),
                "track-3".to_owned(),
            ]
        );
    }
}
