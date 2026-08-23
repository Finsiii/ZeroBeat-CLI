use std::sync::{Arc, Mutex};
use tempfile::tempdir;

use zerobeat_audio::{AudioBackend, BackendError, StreamSource};
use zerobeat_catalog::{AudioQuality, CatalogFuture, MusicCatalog, ResolvedStream, SearchRequest};
use zerobeat_core::{Route, SessionMode, Track};
use zerobeat_daemon::{DaemonError, DaemonServer};
use zerobeat_ipc::IpcConnection;
use zerobeat_protocol::PlaybackStatus;
use zerobeat_protocol::{AppSnapshot, ClientCommand, DaemonEvent, PROTOCOL_VERSION, SearchStatus};

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
    assert_eq!(playing.playback.current.unwrap().title, "Tampar");
    assert_eq!(
        *events.lock().unwrap(),
        ["stop", "load:https://stream.example/tampar", "play"]
    );

    exchange(&mut client, ClientCommand::Shutdown).await;
    server_task.await.unwrap().unwrap();
}

async fn exchange(connection: &mut IpcConnection, command: ClientCommand) -> DaemonEvent {
    connection.send(&command).await.expect("send command");
    connection.receive().await.expect("receive event")
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

impl MusicCatalog for TestCatalog {
    fn search_songs(&self, request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
        Box::pin(async move {
            assert_eq!(request.query, "tampar");
            Ok(vec![Track::new(
                "video-123",
                "Tampar",
                "Juicy Luicy",
                245_000,
            )])
        })
    }

    fn resolve_stream(
        &self,
        _track_id: &str,
        _quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream> {
        Box::pin(async { Ok(ResolvedStream::new("https://stream.example/tampar")) })
    }
}

struct RecordingBackend(Arc<Mutex<Vec<&'static str>>>);

impl AudioBackend for RecordingBackend {
    fn load(&mut self, _source: &StreamSource) -> Result<(), BackendError> {
        self.0
            .lock()
            .unwrap()
            .push("load:https://stream.example/tampar");
        Ok(())
    }

    fn play(&mut self) -> Result<(), BackendError> {
        self.0.lock().unwrap().push("play");
        Ok(())
    }

    fn pause(&mut self) -> Result<(), BackendError> {
        self.0.lock().unwrap().push("pause");
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.0.lock().unwrap().push("stop");
        Ok(())
    }
}
