use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;

use zerobeat_audio::{AudioBackend, BackendError, BackendTelemetry, StreamSource};
use zerobeat_catalog::{
    AudioQuality, CatalogFuture, Lyrics, LyricsLine, MusicCatalog, RadioPage, RadioRequest,
    ResolvedStream, SearchRequest,
};
use zerobeat_core::{Route, SessionMode, Track};
use zerobeat_daemon::{DaemonError, DaemonServer};
use zerobeat_ipc::IpcConnection;
use zerobeat_protocol::{AppSnapshot, ClientCommand, DaemonEvent, PROTOCOL_VERSION, SearchStatus};
use zerobeat_protocol::{DownloadStatus, LyricsStatus, PlaybackStatus, RepeatMode};
use zerobeat_storage::Database;

#[tokio::test]
async fn state_survives_client_disconnect_and_reconnect() {
    let directory = tempdir().expect("temporary directory");
    let socket = directory.path().join("zerobeat.sock");
    let server = DaemonServer::bind(&socket).await.expect("bind daemon");
    let server_task = tokio::spawn(server.run());

    let mut first = IpcConnection::connect(&socket)
        .await
        .expect("connect first client");
    let initial = exchange(
        &mut first,
        ClientCommand::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await;
    assert_snapshot(&initial, SessionMode::Guest, Route::Home, "");

    exchange(&mut first, ClientCommand::Navigate(Route::Search)).await;
    exchange(&mut first, ClientCommand::UpdateSearch("tampar".into())).await;
    drop(first);

    let mut second = IpcConnection::connect(&socket)
        .await
        .expect("connect second client");
    let restored = exchange(&mut second, ClientCommand::RequestSnapshot).await;
    assert_snapshot(&restored, SessionMode::Guest, Route::Search, "tampar");

    let restored = exchange(&mut second, ClientCommand::Back).await;
    assert_snapshot(&restored, SessionMode::Guest, Route::Home, "tampar");

    let stopped = exchange(&mut second, ClientCommand::Shutdown).await;
    assert_eq!(stopped, DaemonEvent::Acknowledged);
    server_task
        .await
        .expect("server task")
        .expect("server shutdown");
}

#[tokio::test]
async fn second_daemon_cannot_replace_a_live_socket() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let _first = DaemonServer::bind(&socket).await.unwrap();

    let second = DaemonServer::bind(&socket).await;

    assert!(matches!(second, Err(DaemonError::AlreadyRunning(path)) if path == socket));
}

#[tokio::test]
async fn search_runs_in_background_and_updates_the_snapshot() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let events = Arc::new(Mutex::new(Vec::new()));
    let server = DaemonServer::bind_with_services(
        &socket,
        TestCatalog,
        RecordingBackend(Arc::clone(&events)),
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();

    exchange(&mut client, ClientCommand::UpdateSearch("tampar".into())).await;
    let loading = exchange(&mut client, ClientCommand::SubmitSearch).await;
    let DaemonEvent::Snapshot(loading) = loading else {
        panic!("expected loading snapshot");
    };
    assert_eq!(loading.search.status, SearchStatus::Loading);

    let ready = loop {
        let event = exchange(&mut client, ClientCommand::RequestSnapshot).await;
        let DaemonEvent::Snapshot(snapshot) = event else {
            panic!("expected snapshot");
        };
        if snapshot.search.status == SearchStatus::Ready {
            break snapshot;
        }
        tokio::task::yield_now().await;
    };
    assert_eq!(ready.search.results[0].title, "Tampar");

    let resolving = exchange(&mut client, ClientCommand::PlaySelected).await;
    let DaemonEvent::Snapshot(resolving) = resolving else {
        panic!("expected playback snapshot");
    };
    assert_eq!(resolving.playback.status, PlaybackStatus::Resolving);

    let playing = loop {
        let event = exchange(&mut client, ClientCommand::RequestSnapshot).await;
        let DaemonEvent::Snapshot(snapshot) = event else {
            panic!("expected snapshot");
        };
        if snapshot.playback.status == PlaybackStatus::Playing {
            break snapshot;
        }
        tokio::task::yield_now().await;
    };
    assert_eq!(playing.playback.current.as_ref().unwrap().title, "Tampar");
    assert_eq!(
        playing
            .playback
            .queue
            .iter()
            .map(|track| track.title.as_str())
            .collect::<Vec<_>>(),
        ["Sialan", "Lampu Kuning"]
    );
    assert_eq!(
        *events.lock().unwrap(),
        ["stop", "load:https://stream.example/tampar", "play"]
    );
    let lyrics = wait_for(&mut client, |snapshot| {
        snapshot.lyrics.status == LyricsStatus::Ready
    })
    .await;
    assert!(!lyrics.lyrics.visible);
    let lyrics = exchange(&mut client, ClientCommand::ToggleLyrics).await;
    let DaemonEvent::Snapshot(lyrics) = lyrics else {
        panic!("expected lyrics snapshot");
    };
    assert!(lyrics.lyrics.visible);
    assert_eq!(lyrics.lyrics.lines[0].words, "Entah sudah selasa");

    let navigated = exchange(&mut client, ClientCommand::Navigate(Route::Library)).await;
    let DaemonEvent::Snapshot(navigated) = navigated else {
        panic!("expected navigation snapshot");
    };
    assert_eq!(navigated.navigation.active_route(), Route::Library);
    assert!(!navigated.lyrics.visible);

    let next = exchange(&mut client, ClientCommand::NextTrack).await;
    let DaemonEvent::Snapshot(next) = next else {
        panic!("expected next snapshot");
    };
    assert_eq!(next.playback.current.as_ref().unwrap().title, "Sialan");
    assert_eq!(next.playback.status, PlaybackStatus::Resolving);
    assert_eq!(next.playback.queue.len(), 1);

    let playing_next = loop {
        let event = exchange(&mut client, ClientCommand::RequestSnapshot).await;
        let DaemonEvent::Snapshot(snapshot) = event else {
            panic!("expected snapshot");
        };
        if snapshot.playback.status == PlaybackStatus::Playing {
            break snapshot;
        }
        tokio::task::yield_now().await;
    };
    assert_eq!(
        playing_next.playback.current.as_ref().unwrap().title,
        "Sialan"
    );
    assert_eq!(
        *events.lock().unwrap(),
        [
            "stop",
            "load:https://stream.example/tampar",
            "play",
            "stop",
            "load:https://stream.example/sialan",
            "play",
        ]
    );

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn playback_modes_history_mute_and_queue_controls_are_consistent() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let events = Arc::new(Mutex::new(Vec::new()));
    let server = DaemonServer::bind_with_services(
        &socket,
        TestCatalog,
        RecordingBackend(Arc::clone(&events)),
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();

    exchange(&mut client, ClientCommand::UpdateSearch("tampar".into())).await;
    exchange(&mut client, ClientCommand::SubmitSearch).await;
    wait_for(&mut client, |snapshot| {
        snapshot.search.status == SearchStatus::Ready
    })
    .await;
    exchange(&mut client, ClientCommand::PlaySelected).await;
    wait_for(&mut client, |snapshot| {
        snapshot.playback.status == PlaybackStatus::Playing
    })
    .await;

    let shuffled = exchange(&mut client, ClientCommand::ToggleShuffle).await;
    let DaemonEvent::Snapshot(shuffled) = shuffled else {
        panic!("expected snapshot");
    };
    assert!(shuffled.playback.shuffle);
    let mut queued_ids = shuffled
        .playback
        .queue
        .iter()
        .map(|track| track.id.as_str())
        .collect::<Vec<_>>();
    queued_ids.sort_unstable();
    assert_eq!(queued_ids, ["video-456", "video-789"]);

    for expected in [RepeatMode::All, RepeatMode::One, RepeatMode::Off] {
        let event = exchange(&mut client, ClientCommand::CycleRepeat).await;
        let DaemonEvent::Snapshot(snapshot) = event else {
            panic!("expected snapshot");
        };
        assert_eq!(snapshot.playback.repeat_mode, expected);
    }

    exchange(&mut client, ClientCommand::SetVolume(35)).await;
    let muted = exchange(&mut client, ClientCommand::ToggleMute).await;
    let DaemonEvent::Snapshot(muted) = muted else {
        panic!("expected snapshot");
    };
    assert!(muted.playback.muted);
    assert_eq!(muted.playback.volume_percent, 0);
    let restored = exchange(&mut client, ClientCommand::ToggleMute).await;
    let DaemonEvent::Snapshot(restored) = restored else {
        panic!("expected snapshot");
    };
    assert!(!restored.playback.muted);
    assert_eq!(restored.playback.volume_percent, 35);

    let next = exchange(&mut client, ClientCommand::NextTrack).await;
    let DaemonEvent::Snapshot(next) = next else {
        panic!("expected snapshot");
    };
    assert_eq!(next.playback.history.len(), 1);
    let next_title = next.playback.current.as_ref().unwrap().title.clone();
    let previous = exchange(&mut client, ClientCommand::PreviousTrack).await;
    let DaemonEvent::Snapshot(previous) = previous else {
        panic!("expected snapshot");
    };
    assert_eq!(previous.playback.current.as_ref().unwrap().title, "Tampar");
    assert_eq!(previous.playback.queue[0].title, next_title);
    assert!(previous.playback.history.is_empty());

    wait_for(&mut client, |snapshot| {
        snapshot.playback.status == PlaybackStatus::Playing
    })
    .await;
    let direct = Track::new("direct", "Direct Pick", "ZeroBeat", 180_000);
    exchange(&mut client, ClientCommand::PlayTrack(direct)).await;
    wait_for(&mut client, |snapshot| {
        snapshot.playback.status == PlaybackStatus::Playing
    })
    .await;
    let previous = exchange(&mut client, ClientCommand::PreviousTrack).await;
    let DaemonEvent::Snapshot(previous) = previous else {
        panic!("expected snapshot");
    };
    assert_eq!(previous.playback.current.as_ref().unwrap().title, "Tampar");
    let queue_len = previous.playback.queue.len();

    let removed = exchange(&mut client, ClientCommand::RemoveQueueIndex(0)).await;
    let DaemonEvent::Snapshot(removed) = removed else {
        panic!("expected snapshot");
    };
    assert_eq!(removed.playback.queue.len(), queue_len - 1);
    let cleared = exchange(&mut client, ClientCommand::ClearQueue).await;
    let DaemonEvent::Snapshot(cleared) = cleared else {
        panic!("expected snapshot");
    };
    assert!(cleared.playback.queue.is_empty());

    assert!(events.lock().unwrap().contains(&"volume:0.350".to_owned()));
    assert!(events.lock().unwrap().contains(&"volume:0.000".to_owned()));
    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn playback_auto_advances_before_the_current_track_ends() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let events = Arc::new(Mutex::new(Vec::new()));
    let near_end = Arc::new(AtomicBool::new(false));
    let server = DaemonServer::bind_with_services(
        &socket,
        TestCatalog,
        AutoAdvanceBackend {
            events: Arc::clone(&events),
            near_end: Arc::clone(&near_end),
        },
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();

    exchange(&mut client, ClientCommand::UpdateSearch("tampar".into())).await;
    exchange(&mut client, ClientCommand::SubmitSearch).await;
    wait_for(&mut client, |snapshot| {
        snapshot.search.status == SearchStatus::Ready
    })
    .await;
    exchange(&mut client, ClientCommand::PlaySelected).await;
    wait_for(&mut client, |snapshot| {
        snapshot.playback.status == PlaybackStatus::Playing
    })
    .await;

    wait_for(&mut client, |snapshot| {
        snapshot.playback.spectrum == [48; 24]
    })
    .await;
    let paused = exchange(&mut client, ClientCommand::TogglePlayback).await;
    let DaemonEvent::Snapshot(paused) = paused else {
        panic!("expected snapshot");
    };
    assert_eq!(paused.playback.status, PlaybackStatus::Paused);
    assert_eq!(paused.playback.spectrum, [0; 24]);
    exchange(&mut client, ClientCommand::TogglePlayback).await;

    near_end.store(true, Ordering::Release);
    let advanced = wait_for(&mut client, |snapshot| {
        snapshot.playback.status == PlaybackStatus::Playing
            && snapshot
                .playback
                .current
                .as_ref()
                .is_some_and(|track| track.title == "Sialan")
            && snapshot.playback.spectrum == [48; 24]
    })
    .await;
    assert_eq!(advanced.playback.queue.len(), 1);
    assert_eq!(advanced.playback.spectrum, [48; 24]);
    assert!(
        events
            .lock()
            .unwrap()
            .contains(&"load:https://stream.example/sialan")
    );

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn endless_radio_keeps_the_visible_queue_unique_and_capped_at_twelve() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let events = Arc::new(Mutex::new(Vec::new()));
    let radio_calls = Arc::new(AtomicUsize::new(0));
    let server = DaemonServer::bind_with_services(
        &socket,
        EndlessCatalog {
            radio_calls: Arc::clone(&radio_calls),
        },
        RecordingBackend(Arc::clone(&events)),
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();

    exchange(&mut client, ClientCommand::UpdateSearch("seed".into())).await;
    exchange(&mut client, ClientCommand::SubmitSearch).await;
    wait_for(&mut client, |snapshot| {
        snapshot.search.status == SearchStatus::Ready
    })
    .await;
    let started = exchange(&mut client, ClientCommand::PlaySelected).await;
    let DaemonEvent::Snapshot(started) = started else {
        panic!("expected playback snapshot");
    };
    assert_eq!(started.playback.queue.len(), 12);

    wait_for(&mut client, |_| radio_calls.load(Ordering::Acquire) > 0).await;
    for _ in 0..8 {
        let advanced = exchange(&mut client, ClientCommand::NextTrack).await;
        let DaemonEvent::Snapshot(advanced) = advanced else {
            panic!("expected next snapshot");
        };
        assert_eq!(advanced.playback.queue.len(), 12);
        let unique = advanced
            .playback
            .queue
            .iter()
            .map(|track| track.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), 12);
    }

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn endless_radio_retries_transient_failure_with_backoff() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let radio_calls = Arc::new(AtomicUsize::new(0));
    let server = DaemonServer::bind_with_services(
        &socket,
        RetryingRadioCatalog {
            radio_calls: Arc::clone(&radio_calls),
        },
        RecordingBackend(Arc::new(Mutex::new(Vec::new()))),
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();

    exchange(
        &mut client,
        ClientCommand::PlayTrack(Track::new("seed", "Seed", "ZeroBeat", 180_000)),
    )
    .await;
    wait_for_calls(&radio_calls, 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(radio_calls.load(Ordering::Acquire), 1);

    let refilled = wait_for(&mut client, |snapshot| snapshot.playback.queue.len() == 12).await;
    assert_eq!(refilled.playback.queue.len(), 12);
    assert_eq!(radio_calls.load(Ordering::Acquire), 2);

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn endless_radio_stops_when_continuation_makes_no_progress() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let radio_calls = Arc::new(AtomicUsize::new(0));
    let server = DaemonServer::bind_with_services(
        &socket,
        NoProgressRadioCatalog {
            radio_calls: Arc::clone(&radio_calls),
        },
        RecordingBackend(Arc::new(Mutex::new(Vec::new()))),
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();

    exchange(
        &mut client,
        ClientCommand::PlayTrack(Track::new("seed", "Seed", "ZeroBeat", 180_000)),
    )
    .await;
    wait_for_calls(&radio_calls, 2).await;
    tokio::time::sleep(std::time::Duration::from_millis(750)).await;
    assert_eq!(radio_calls.load(Ordering::Acquire), 2);

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn clearing_queue_invalidates_an_inflight_radio_response() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let radio_started = Arc::new(Notify::new());
    let release_radio = Arc::new(Notify::new());
    let radio_calls = Arc::new(AtomicUsize::new(0));
    let server = DaemonServer::bind_with_services(
        &socket,
        BlockingRadioCatalog {
            radio_started: Arc::clone(&radio_started),
            release_radio: Arc::clone(&release_radio),
            radio_calls: Arc::clone(&radio_calls),
        },
        RecordingBackend(Arc::new(Mutex::new(Vec::new()))),
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();

    exchange(
        &mut client,
        ClientCommand::PlayTrack(Track::new("seed", "Seed", "ZeroBeat", 180_000)),
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(1), radio_started.notified())
        .await
        .expect("radio request did not start");

    let cleared = exchange(&mut client, ClientCommand::ClearQueue).await;
    let DaemonEvent::Snapshot(cleared) = cleared else {
        panic!("expected clear snapshot");
    };
    assert!(cleared.playback.queue.is_empty());

    release_radio.notify_one();
    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    let settled = exchange(&mut client, ClientCommand::RequestSnapshot).await;
    let DaemonEvent::Snapshot(settled) = settled else {
        panic!("expected settled snapshot");
    };
    assert!(settled.playback.queue.is_empty());
    assert_eq!(radio_calls.load(Ordering::Acquire), 1);

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn repeat_one_restarts_the_current_track_without_consuming_the_queue() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let events = Arc::new(Mutex::new(Vec::new()));
    let ended = Arc::new(AtomicBool::new(false));
    let server = DaemonServer::bind_with_services(
        &socket,
        TestCatalog,
        RepeatBackend {
            events: Arc::clone(&events),
            ended: Arc::clone(&ended),
        },
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();

    exchange(&mut client, ClientCommand::UpdateSearch("tampar".into())).await;
    exchange(&mut client, ClientCommand::SubmitSearch).await;
    wait_for(&mut client, |snapshot| {
        snapshot.search.status == SearchStatus::Ready
    })
    .await;
    exchange(&mut client, ClientCommand::PlaySelected).await;
    wait_for(&mut client, |snapshot| {
        snapshot.playback.status == PlaybackStatus::Playing
    })
    .await;
    exchange(&mut client, ClientCommand::CycleRepeat).await;
    exchange(&mut client, ClientCommand::CycleRepeat).await;

    ended.store(true, Ordering::Release);
    let repeated = wait_for(&mut client, |snapshot| {
        snapshot.playback.status == PlaybackStatus::Playing
            && events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| **event == "load:https://stream.example/tampar")
                .count()
                >= 2
    })
    .await;
    assert_eq!(repeated.playback.current.as_ref().unwrap().title, "Tampar");
    assert_eq!(repeated.playback.queue.len(), 2);
    assert!(repeated.playback.history.is_empty());

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn skipping_a_resolving_track_with_an_empty_queue_cancels_its_start() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let events = Arc::new(Mutex::new(Vec::new()));
    let resolve_started = Arc::new(Notify::new());
    let release_resolve = Arc::new(Notify::new());
    let server = DaemonServer::bind_with_services(
        &socket,
        DelayedCatalog {
            resolve_started: Arc::clone(&resolve_started),
            release_resolve: Arc::clone(&release_resolve),
        },
        RecordingBackend(Arc::clone(&events)),
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();

    exchange(
        &mut client,
        ClientCommand::PlayTrack(Track::new("delayed", "Delayed", "ZeroBeat", 60_000)),
    )
    .await;
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        resolve_started.notified(),
    )
    .await
    .expect("stream resolution did not start");

    let stopped = exchange(&mut client, ClientCommand::NextTrack).await;
    let DaemonEvent::Snapshot(stopped) = stopped else {
        panic!("expected snapshot");
    };
    assert_eq!(stopped.playback.status, PlaybackStatus::Idle);
    assert!(stopped.playback.current.is_none());

    release_resolve.notify_waiters();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let settled = exchange(&mut client, ClientCommand::RequestSnapshot).await;
    let DaemonEvent::Snapshot(settled) = settled else {
        panic!("expected snapshot");
    };
    assert_eq!(settled.playback.status, PlaybackStatus::Idle);
    assert!(settled.playback.current.is_none());
    assert_eq!(*events.lock().unwrap(), ["stop"]);

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn a_track_superseded_during_blocking_load_never_starts() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let events = Arc::new(Mutex::new(Vec::new()));
    let first_load_started = Arc::new(AtomicBool::new(false));
    let release_first_load = Arc::new(AtomicBool::new(false));
    let server = DaemonServer::bind_with_services(
        &socket,
        TestCatalog,
        BlockingLoadBackend {
            events: Arc::clone(&events),
            loaded: String::new(),
            first_load_started: Arc::clone(&first_load_started),
            release_first_load: Arc::clone(&release_first_load),
        },
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();

    exchange(
        &mut client,
        ClientCommand::PlayTrack(Track::new("video-123", "First", "ZeroBeat", 60_000)),
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !first_load_started.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("first load did not start");

    exchange(
        &mut client,
        ClientCommand::PlayTrack(Track::new("video-456", "Second", "ZeroBeat", 60_000)),
    )
    .await;
    release_first_load.store(true, Ordering::Release);

    let playing = wait_for(&mut client, |snapshot| {
        snapshot.playback.status == PlaybackStatus::Playing
            && snapshot
                .playback
                .current
                .as_ref()
                .is_some_and(|track| track.id == "video-456")
    })
    .await;
    assert_eq!(playing.playback.current.as_ref().unwrap().title, "Second");
    {
        let events = events.lock().unwrap();
        assert!(!events.iter().any(|event| event == "play:video-123"));
        assert!(events.iter().any(|event| event == "play:video-456"));
    }

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn guest_library_survives_daemon_restart() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let database_path = directory.path().join("guest.db");
    let server = DaemonServer::bind_with_services_and_storage(
        &socket,
        TestCatalog,
        RecordingBackend(Arc::new(Mutex::new(Vec::new()))),
        Database::open(&database_path).unwrap(),
        directory.path().join("downloads"),
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();

    exchange(&mut client, ClientCommand::UpdateSearch("tampar".into())).await;
    exchange(&mut client, ClientCommand::SubmitSearch).await;
    let ready = wait_for(&mut client, |snapshot| {
        snapshot.search.status == SearchStatus::Ready
    })
    .await;
    let song = ready.search.results[0].clone();
    exchange(&mut client, ClientCommand::PlaySelected).await;
    let playing = wait_for(&mut client, |snapshot| {
        snapshot.playback.status == PlaybackStatus::Playing
            && snapshot.library.recent.first() == Some(&song)
    })
    .await;
    assert!(playing.library.liked.is_empty());
    let liked = exchange(&mut client, ClientCommand::ToggleLike(song.clone())).await;
    let DaemonEvent::Snapshot(liked) = liked else {
        panic!("expected snapshot");
    };
    assert_eq!(liked.library.liked, vec![song.clone()]);
    let settings = exchange(&mut client, ClientCommand::SetCrossfadeSeconds(9)).await;
    let DaemonEvent::Snapshot(settings) = settings else {
        panic!("expected snapshot");
    };
    assert_eq!(settings.settings.crossfade_seconds, 9);

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();

    let server = DaemonServer::bind_with_services_and_storage(
        &socket,
        TestCatalog,
        RecordingBackend(Arc::new(Mutex::new(Vec::new()))),
        Database::open(&database_path).unwrap(),
        directory.path().join("downloads"),
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();
    let restored = exchange(&mut client, ClientCommand::RequestSnapshot).await;
    let DaemonEvent::Snapshot(restored) = restored else {
        panic!("expected snapshot");
    };
    assert_eq!(restored.library.liked, vec![song.clone()]);
    assert_eq!(restored.library.recent, vec![song]);
    assert_eq!(restored.settings.crossfade_seconds, 9);

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn guest_download_uses_query_range_and_becomes_playable_offline() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let downloads = directory.path().join("downloads");
    let media = Arc::new(vec![0x5a; 600_000]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_media = Arc::clone(&media);
    let range_server = tokio::spawn(async move {
        for _ in 0..1 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let length = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..length]);
            let range = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|target| target.split("range=").nth(1))
                .and_then(|range| range.split('&').next())
                .unwrap();
            assert!(!request.to_ascii_lowercase().contains("\r\nrange:"));
            let (start, end) = range.split_once('-').unwrap();
            let start: usize = start.parse().unwrap();
            let end = end.parse::<usize>().unwrap().min(server_media.len() - 1);
            assert_eq!((start, end), (0, server_media.len() - 1));
            let body = &server_media[start..=end];
            let headers = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nConnection: close\r\n\r\n",
                body.len(),
                body.len() - 1,
                body.len(),
            );
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
        }
    });
    let server = DaemonServer::bind_with_services_and_storage(
        &socket,
        DownloadCatalog {
            url: format!(
                "http://{address}/audio?clen={}&range=0-{}",
                media.len(),
                media.len()
            ),
            resolve_count: Arc::new(AtomicUsize::new(0)),
        },
        RecordingBackend(Arc::new(Mutex::new(Vec::new()))),
        Database::open(directory.path().join("guest.db")).unwrap(),
        &downloads,
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();
    let song = Track::new("video-download", "Offline", "ZeroBeat", 20_000);

    exchange(&mut client, ClientCommand::DownloadTrack(song.clone())).await;
    let ready = wait_for(&mut client, |snapshot| {
        snapshot.library.downloads.first().is_some_and(|download| {
            download.track == song && download.status == DownloadStatus::Available
        })
    })
    .await;
    assert_eq!(ready.library.downloads.len(), 1);
    let downloaded = std::fs::read(downloads.join("video-download.audio")).unwrap();
    assert_eq!(downloaded.len(), media.len());
    assert!(downloaded.iter().all(|byte| *byte == 0x5a));
    range_server.await.unwrap();

    exchange(&mut client, ClientCommand::DownloadTrack(song.clone())).await;
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    let duplicate = exchange(&mut client, ClientCommand::RequestSnapshot).await;
    let DaemonEvent::Snapshot(duplicate) = duplicate else {
        panic!("expected snapshot");
    };
    assert_eq!(
        duplicate.library.downloads[0].status,
        DownloadStatus::Available
    );

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn available_download_plays_without_resolving_the_network() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("zerobeat.sock");
    let local_path = directory.path().join("offline.audio");
    std::fs::write(&local_path, b"fixture").unwrap();
    let song = Track::new("video-offline", "Offline", "ZeroBeat", 20_000);
    let database = Database::open(directory.path().join("guest.db")).unwrap();
    database
        .set_download(
            &song,
            zerobeat_storage::DownloadState::Available,
            local_path.to_str(),
        )
        .unwrap();
    database
        .save_lyrics(
            &song,
            &Lyrics {
                synced: true,
                lines: vec![LyricsLine {
                    start_ms: Some(1_000),
                    words: "Cached offline lyric".into(),
                }],
            },
        )
        .unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let server = DaemonServer::bind_with_services_and_storage(
        &socket,
        FailingCatalog,
        RecordingBackend(Arc::clone(&events)),
        database,
        directory.path().join("downloads"),
    )
    .await
    .unwrap();
    let server_task = tokio::spawn(server.run());
    let mut client = IpcConnection::connect(&socket).await.unwrap();

    exchange(&mut client, ClientCommand::PlayTrack(song)).await;
    let playing = wait_for(&mut client, |snapshot| {
        snapshot.playback.status == PlaybackStatus::Playing
            && snapshot.lyrics.status == LyricsStatus::Ready
    })
    .await;
    assert_eq!(playing.lyrics.lines[0].words, "Cached offline lyric");
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "stop".to_owned(),
            format!("load:{}", local_path.display()),
            "play".to_owned(),
        ]
    );

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

async fn exchange(connection: &mut IpcConnection, command: ClientCommand) -> DaemonEvent {
    connection.send(&command).await.expect("send command");
    connection.receive().await.expect("receive event")
}

async fn wait_for(
    connection: &mut IpcConnection,
    predicate: impl Fn(&AppSnapshot) -> bool,
) -> Box<AppSnapshot> {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let event = exchange(connection, ClientCommand::RequestSnapshot).await;
            let DaemonEvent::Snapshot(snapshot) = event else {
                panic!("expected snapshot");
            };
            if predicate(&snapshot) {
                return snapshot;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("snapshot condition timed out")
}

async fn wait_for_calls(calls: &AtomicUsize, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while calls.load(Ordering::Acquire) < expected {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("catalog call condition timed out");
}

fn assert_snapshot(
    event: &DaemonEvent,
    expected_session: SessionMode,
    expected_route: Route,
    expected_query: &str,
) {
    let DaemonEvent::Snapshot(snapshot) = event else {
        panic!("expected snapshot, got {event:?}");
    };
    let AppSnapshot {
        session,
        navigation,
        ..
    } = snapshot.as_ref();

    assert_eq!(*session, expected_session);
    assert_eq!(navigation.active_route(), expected_route);
    assert_eq!(navigation.search_query(), expected_query);
}

struct TestCatalog;

struct DownloadCatalog {
    url: String,
    resolve_count: Arc<AtomicUsize>,
}

struct FailingCatalog;

struct DelayedCatalog {
    resolve_started: Arc<Notify>,
    release_resolve: Arc<Notify>,
}

impl MusicCatalog for DelayedCatalog {
    fn search_songs(&self, _request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resolve_stream(
        &self,
        track_id: &str,
        _quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream> {
        let track_id = track_id.to_owned();
        let resolve_started = Arc::clone(&self.resolve_started);
        let release_resolve = Arc::clone(&self.release_resolve);
        Box::pin(async move {
            resolve_started.notify_one();
            release_resolve.notified().await;
            Ok(ResolvedStream::new(format!(
                "https://stream.example/{track_id}"
            )))
        })
    }
}

impl MusicCatalog for FailingCatalog {
    fn search_songs(&self, _request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
        Box::pin(async {
            Err(zerobeat_catalog::CatalogError::Unavailable(
                "offline".into(),
            ))
        })
    }

    fn resolve_stream(
        &self,
        _track_id: &str,
        _quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream> {
        Box::pin(async { panic!("downloaded playback must not resolve the network") })
    }

    fn lyrics(&self, _track: &Track) -> CatalogFuture<'_, Option<Lyrics>> {
        Box::pin(async { panic!("cached lyrics must not resolve the network") })
    }
}

impl MusicCatalog for DownloadCatalog {
    fn search_songs(&self, _request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn resolve_stream(
        &self,
        _track_id: &str,
        _quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream> {
        self.resolve_count.fetch_add(1, Ordering::Relaxed);
        let url = self.url.clone();
        Box::pin(async move { Ok(ResolvedStream::new(url)) })
    }
}

impl MusicCatalog for TestCatalog {
    fn search_songs(&self, request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
        Box::pin(async move {
            assert_eq!(request.query, "tampar");
            Ok(vec![
                Track::new("video-123", "Tampar", "Juicy Luicy", 245_000),
                Track::new("video-456", "Sialan", "Juicy Luicy", 242_000),
                Track::new("video-789", "Lampu Kuning", "Juicy Luicy", 240_000),
            ])
        })
    }

    fn resolve_stream(
        &self,
        _track_id: &str,
        _quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream> {
        let track_id = _track_id.to_owned();
        Box::pin(async move {
            Ok(ResolvedStream::new(format!(
                "https://stream.example/{track_id}"
            )))
        })
    }

    fn lyrics(&self, _track: &Track) -> CatalogFuture<'_, Option<Lyrics>> {
        Box::pin(async {
            Ok(Some(Lyrics {
                synced: true,
                lines: vec![
                    LyricsLine {
                        start_ms: Some(1_000),
                        words: "Entah sudah selasa".into(),
                    },
                    LyricsLine {
                        start_ms: Some(5_000),
                        words: "Masih saja kau ada".into(),
                    },
                ],
            }))
        })
    }
}

struct EndlessCatalog {
    radio_calls: Arc<AtomicUsize>,
}

struct RetryingRadioCatalog {
    radio_calls: Arc<AtomicUsize>,
}

struct NoProgressRadioCatalog {
    radio_calls: Arc<AtomicUsize>,
}

struct BlockingRadioCatalog {
    radio_started: Arc<Notify>,
    release_radio: Arc<Notify>,
    radio_calls: Arc<AtomicUsize>,
}

impl MusicCatalog for EndlessCatalog {
    fn search_songs(&self, request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
        Box::pin(async move {
            assert_eq!(request.query, "seed");
            Ok((0..20)
                .map(|index| {
                    Track::new(
                        format!("search-{index}"),
                        format!("Search {index}"),
                        "ZeroBeat",
                        180_000,
                    )
                })
                .collect())
        })
    }

    fn radio_tracks(&self, request: RadioRequest) -> CatalogFuture<'_, RadioPage> {
        self.radio_calls.fetch_add(1, Ordering::Release);
        Box::pin(async move {
            assert_eq!(request.limit, 24);
            Ok(RadioPage {
                tracks: (0..24)
                    .map(|index| {
                        Track::new(
                            format!("radio-{index}"),
                            format!("Radio {index}"),
                            "ZeroBeat",
                            180_000,
                        )
                    })
                    .collect(),
                continuation: Some("next-radio-page".into()),
            })
        })
    }

    fn resolve_stream(
        &self,
        track_id: &str,
        _quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream> {
        let track_id = track_id.to_owned();
        Box::pin(async move {
            Ok(ResolvedStream::new(format!(
                "https://stream.example/{track_id}"
            )))
        })
    }
}

impl MusicCatalog for RetryingRadioCatalog {
    fn search_songs(&self, _request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn radio_tracks(&self, request: RadioRequest) -> CatalogFuture<'_, RadioPage> {
        let call = self.radio_calls.fetch_add(1, Ordering::AcqRel) + 1;
        Box::pin(async move {
            assert_eq!(request.limit, 24);
            if call == 1 {
                return Err(zerobeat_catalog::CatalogError::Unavailable(
                    "temporary radio failure".into(),
                ));
            }
            Ok(RadioPage {
                tracks: (0..24)
                    .map(|index| {
                        Track::new(
                            format!("retry-radio-{index}"),
                            format!("Retry Radio {index}"),
                            "ZeroBeat",
                            180_000,
                        )
                    })
                    .collect(),
                continuation: None,
            })
        })
    }

    fn resolve_stream(
        &self,
        track_id: &str,
        _quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream> {
        let track_id = track_id.to_owned();
        Box::pin(async move {
            Ok(ResolvedStream::new(format!(
                "https://stream.example/{track_id}"
            )))
        })
    }
}

impl MusicCatalog for NoProgressRadioCatalog {
    fn search_songs(&self, _request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn radio_tracks(&self, request: RadioRequest) -> CatalogFuture<'_, RadioPage> {
        self.radio_calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(async move {
            assert_eq!(request.limit, 24);
            Ok(RadioPage {
                tracks: Vec::new(),
                continuation: Some("next".into()),
            })
        })
    }

    fn resolve_stream(
        &self,
        track_id: &str,
        _quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream> {
        let track_id = track_id.to_owned();
        Box::pin(async move {
            Ok(ResolvedStream::new(format!(
                "https://stream.example/{track_id}"
            )))
        })
    }
}

impl MusicCatalog for BlockingRadioCatalog {
    fn search_songs(&self, _request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn radio_tracks(&self, request: RadioRequest) -> CatalogFuture<'_, RadioPage> {
        let radio_started = Arc::clone(&self.radio_started);
        let release_radio = Arc::clone(&self.release_radio);
        self.radio_calls.fetch_add(1, Ordering::AcqRel);
        Box::pin(async move {
            assert_eq!(request.limit, 24);
            radio_started.notify_one();
            release_radio.notified().await;
            Ok(RadioPage {
                tracks: (0..24)
                    .map(|index| {
                        Track::new(
                            format!("blocked-radio-{index}"),
                            format!("Blocked Radio {index}"),
                            "ZeroBeat",
                            180_000,
                        )
                    })
                    .collect(),
                continuation: None,
            })
        })
    }

    fn resolve_stream(
        &self,
        track_id: &str,
        _quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream> {
        let track_id = track_id.to_owned();
        Box::pin(async move {
            Ok(ResolvedStream::new(format!(
                "https://stream.example/{track_id}"
            )))
        })
    }
}

struct RecordingBackend(Arc<Mutex<Vec<String>>>);

struct AutoAdvanceBackend {
    events: Arc<Mutex<Vec<&'static str>>>,
    near_end: Arc<AtomicBool>,
}

struct RepeatBackend {
    events: Arc<Mutex<Vec<&'static str>>>,
    ended: Arc<AtomicBool>,
}

struct BlockingLoadBackend {
    events: Arc<Mutex<Vec<String>>>,
    loaded: String,
    first_load_started: Arc<AtomicBool>,
    release_first_load: Arc<AtomicBool>,
}

impl AudioBackend for BlockingLoadBackend {
    fn load(&mut self, source: &StreamSource) -> Result<(), BackendError> {
        self.loaded = source.url.rsplit('/').next().unwrap_or_default().to_owned();
        self.events
            .lock()
            .unwrap()
            .push(format!("load:{}", self.loaded));
        if self.loaded == "video-123" {
            self.first_load_started.store(true, Ordering::Release);
            while !self.release_first_load.load(Ordering::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        Ok(())
    }

    fn play(&mut self) -> Result<(), BackendError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("play:{}", self.loaded));
        Ok(())
    }

    fn pause(&mut self) -> Result<(), BackendError> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.events.lock().unwrap().push("stop".to_owned());
        Ok(())
    }
}

impl AudioBackend for RecordingBackend {
    fn load(&mut self, source: &StreamSource) -> Result<(), BackendError> {
        let event = match source.url.rsplit('/').next() {
            Some("video-123") => "load:https://stream.example/tampar".to_owned(),
            Some("video-456") => "load:https://stream.example/sialan".to_owned(),
            _ => format!("load:{}", source.url),
        };
        self.0.lock().unwrap().push(event);
        Ok(())
    }

    fn play(&mut self) -> Result<(), BackendError> {
        self.0.lock().unwrap().push("play".to_owned());
        Ok(())
    }

    fn pause(&mut self) -> Result<(), BackendError> {
        self.0.lock().unwrap().push("pause".to_owned());
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.0.lock().unwrap().push("stop".to_owned());
        Ok(())
    }

    fn seek(&mut self, position_ms: u64) -> Result<(), BackendError> {
        self.0.lock().unwrap().push(format!("seek:{position_ms}"));
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), BackendError> {
        self.0.lock().unwrap().push(format!("volume:{volume:.3}"));
        Ok(())
    }
}

impl AudioBackend for AutoAdvanceBackend {
    fn load(&mut self, source: &StreamSource) -> Result<(), BackendError> {
        let event = match source.url.rsplit('/').next() {
            Some("video-123") => "load:https://stream.example/tampar",
            Some("video-456") => "load:https://stream.example/sialan",
            _ => "load:https://stream.example/other",
        };
        self.events.lock().unwrap().push(event);
        Ok(())
    }

    fn play(&mut self) -> Result<(), BackendError> {
        self.events.lock().unwrap().push("play");
        Ok(())
    }

    fn pause(&mut self) -> Result<(), BackendError> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.events.lock().unwrap().push("stop");
        Ok(())
    }

    fn telemetry(&self) -> BackendTelemetry {
        BackendTelemetry {
            position_ms: if self.near_end.swap(false, Ordering::AcqRel) {
                240_000
            } else {
                1_000
            },
            duration_ms: 245_000,
            buffered_ms: 245_000,
            spectrum: [48; 24],
            ..BackendTelemetry::default()
        }
    }
}

impl AudioBackend for RepeatBackend {
    fn load(&mut self, source: &StreamSource) -> Result<(), BackendError> {
        let event = match source.url.rsplit('/').next() {
            Some("video-123") => "load:https://stream.example/tampar",
            Some("video-456") => "load:https://stream.example/sialan",
            _ => "load:https://stream.example/other",
        };
        self.events.lock().unwrap().push(event);
        Ok(())
    }

    fn play(&mut self) -> Result<(), BackendError> {
        self.events.lock().unwrap().push("play");
        Ok(())
    }

    fn pause(&mut self) -> Result<(), BackendError> {
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.events.lock().unwrap().push("stop");
        Ok(())
    }

    fn telemetry(&self) -> BackendTelemetry {
        BackendTelemetry {
            position_ms: 245_000,
            duration_ms: 245_000,
            buffered_ms: 245_000,
            ended: self.ended.swap(false, Ordering::AcqRel),
            ..BackendTelemetry::default()
        }
    }
}
