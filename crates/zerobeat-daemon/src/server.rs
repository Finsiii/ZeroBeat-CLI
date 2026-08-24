use std::{
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
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
    AppSnapshot, ClientCommand, DaemonEvent, DownloadSnapshot, DownloadStatus, LibrarySnapshot,
    LyricsLineSnapshot, LyricsStatus, PROTOCOL_VERSION, PlaybackStatus, SearchStatus,
    SettingsSnapshot,
};
use zerobeat_storage::{Database, DownloadState};

use crate::{DaemonError, download::spawn_download};

pub struct DaemonServer {
    listener: UnixListener,
    socket_path: PathBuf,
    state: Arc<Mutex<AppSnapshot>>,
    catalog: Arc<dyn MusicCatalog>,
    audio: SharedAudio,
    storage: SharedStorage,
    download_directory: Arc<PathBuf>,
}

type SharedAudio = Arc<StdMutex<Box<dyn AudioBackend>>>;
type SharedStorage = Arc<StdMutex<Database>>;

const CROSSFADE_PREPARE_LEAD: Duration = Duration::from_secs(2);

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
        Self::bind_with_services_and_storage(
            path,
            catalog,
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

        Ok(Self {
            listener,
            socket_path,
            state: Arc::new(Mutex::new(state)),
            catalog: Arc::new(catalog),
            audio: Arc::new(StdMutex::new(Box::new(audio))),
            storage: Arc::new(StdMutex::new(storage)),
            download_directory: Arc::new(download_directory),
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
                    let storage = Arc::clone(&self.storage);
                    let download_directory = Arc::clone(&self.download_directory);
                    let shutdown_tx = shutdown_tx.clone();
                    tokio::spawn(async move {
                        let _ = handle_client(
                            stream,
                            state,
                            catalog,
                            audio,
                            storage,
                            download_directory,
                            shutdown_tx,
                        ).await;
                    });
                }
                _ = telemetry_tick.tick() => {
                    refresh_playback_telemetry(
                        &self.state,
                        &self.catalog,
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

async fn handle_client(
    stream: UnixStream,
    state: Arc<Mutex<AppSnapshot>>,
    catalog: Arc<dyn MusicCatalog>,
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
            &state,
            &catalog,
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

async fn apply_command(
    command: ClientCommand,
    state: &Arc<Mutex<AppSnapshot>>,
    catalog: &Arc<dyn MusicCatalog>,
    audio: &SharedAudio,
    storage: &SharedStorage,
    download_directory: &Arc<PathBuf>,
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
            let start = if matches!(
                snapshot.playback.status,
                PlaybackStatus::Playing | PlaybackStatus::Paused
            ) {
                PlaybackStart::Crossfade(Duration::from_millis(500))
            } else {
                PlaybackStart::Replace
            };
            let selected_index = snapshot.search.selected_index;
            let Some(track) = snapshot.search.results.get(selected_index).cloned() else {
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
            snapshot.playback.queue = snapshot
                .search
                .results
                .iter()
                .skip(selected_index + 1)
                .cloned()
                .collect();
            let event = snapshot_event(snapshot.clone());
            drop(snapshot);

            spawn_playback(
                track,
                request_id,
                Arc::clone(state),
                Arc::clone(catalog),
                Arc::clone(audio),
                Arc::clone(storage),
                start,
            );
            (event, false)
        }
        ClientCommand::QueueSelected => {
            let mut snapshot = state.lock().await;
            let Some(track) = snapshot
                .search
                .results
                .get(snapshot.search.selected_index)
                .cloned()
            else {
                return (snapshot_event(snapshot.clone()), false);
            };
            let is_current = snapshot
                .playback
                .current
                .as_ref()
                .is_some_and(|current| current.id == track.id);
            let is_queued = snapshot
                .playback
                .queue
                .iter()
                .any(|queued| queued.id == track.id);
            if !is_current && !is_queued {
                snapshot.playback.queue.push(track);
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::PlayTrack(track) => {
            let mut snapshot = state.lock().await;
            let start = if matches!(
                snapshot.playback.status,
                PlaybackStatus::Playing | PlaybackStatus::Paused
            ) {
                PlaybackStart::Crossfade(Duration::from_millis(500))
            } else {
                PlaybackStart::Replace
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
                Arc::clone(storage),
                start,
            );
            (event, false)
        }
        ClientCommand::QueueTrack(track) => {
            let mut snapshot = state.lock().await;
            let is_current = snapshot
                .playback
                .current
                .as_ref()
                .is_some_and(|current| current.id == track.id);
            if !is_current
                && !snapshot
                    .playback
                    .queue
                    .iter()
                    .any(|queued| queued.id == track.id)
            {
                snapshot.playback.queue.push(track);
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
            } else if current == PlaybackStatus::Paused {
                snapshot.playback.status = PlaybackStatus::Playing;
            }
            (snapshot_event(snapshot.clone()), false)
        }
        ClientCommand::NextTrack => {
            let mut snapshot = state.lock().await;
            snapshot.playback.request_id = snapshot.playback.request_id.saturating_add(1);
            let request_id = snapshot.playback.request_id;
            let Some(track) = snapshot.playback.queue.first().cloned() else {
                drop(snapshot);
                let result = run_audio(Arc::clone(audio), |audio| audio.stop()).await;
                let mut snapshot = state.lock().await;
                if let Err(error) = result {
                    set_playback_error(&mut snapshot, error);
                    return (snapshot_event(snapshot.clone()), false);
                }
                snapshot.playback.status = PlaybackStatus::Idle;
                snapshot.playback.current = None;
                snapshot.playback.position_ms = 0;
                snapshot.playback.duration_ms = 0;
                snapshot.playback.buffered_ms = 0;
                snapshot.playback.error = None;
                return (snapshot_event(snapshot.clone()), false);
            };
            snapshot.playback.queue.remove(0);
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
                Arc::clone(storage),
                PlaybackStart::Crossfade(Duration::from_millis(500)),
            );
            (event, false)
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
    storage: SharedStorage,
    start: PlaybackStart,
) {
    tokio::spawn(async move {
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
            match catalog
                .resolve_stream(&track.id, AudioQuality::Automatic)
                .await
            {
                Ok(stream) => stream,
                Err(error) => {
                    finish_playback_error(&state, request_id, error.to_string()).await;
                    return;
                }
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
        let result = run_audio(audio, move |audio| match start {
            PlaybackStart::Replace => {
                let _ = audio.stop();
                audio.load(&source)?;
                audio.play()
            }
            PlaybackStart::Crossfade(duration) => audio.transition_to(&source, duration),
        })
        .await;
        let mut snapshot = state.lock().await;
        if snapshot.playback.request_id != request_id {
            return;
        }
        match result {
            Ok(()) => {
                snapshot.playback.status = PlaybackStatus::Playing;
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
                        Arc::clone(&state),
                        Arc::clone(&catalog),
                        Arc::clone(&storage),
                    );
                }
            }
            Err(error) => set_playback_error(&mut snapshot, error),
        }
    });
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
    state: &Arc<Mutex<AppSnapshot>>,
    catalog: &Arc<dyn MusicCatalog>,
    audio: &SharedAudio,
    storage: &SharedStorage,
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
        if snapshot.playback.status == PlaybackStatus::Playing {
            let crossfade = Duration::from_secs(u64::from(snapshot.settings.crossfade_seconds));
            let lead_ms =
                u64::try_from((crossfade + CROSSFADE_PREPARE_LEAD).as_millis()).unwrap_or(u64::MAX);
            let should_advance = telemetry.ended
                || (crossfade > Duration::ZERO
                    && !snapshot.playback.queue.is_empty()
                    && telemetry.duration_ms > 0
                    && telemetry.position_ms.saturating_add(lead_ms) >= telemetry.duration_ms);
            if should_advance {
                let Some(track) = snapshot.playback.queue.first().cloned() else {
                    snapshot.playback.status = PlaybackStatus::Ended;
                    return;
                };
                snapshot.playback.queue.remove(0);
                snapshot.playback.request_id = snapshot.playback.request_id.saturating_add(1);
                let request_id = snapshot.playback.request_id;
                snapshot.playback.current = Some(track.clone());
                snapshot.playback.position_ms = 0;
                snapshot.playback.duration_ms = track.duration_ms;
                snapshot.playback.buffered_ms = 0;
                snapshot.playback.error = None;
                snapshot.playback.status = PlaybackStatus::Resolving;
                drop(snapshot);
                spawn_playback(
                    track,
                    request_id,
                    Arc::clone(state),
                    Arc::clone(catalog),
                    Arc::clone(audio),
                    Arc::clone(storage),
                    if telemetry.ended || crossfade == Duration::ZERO {
                        PlaybackStart::Replace
                    } else {
                        PlaybackStart::Crossfade(crossfade)
                    },
                );
            }
        }
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
