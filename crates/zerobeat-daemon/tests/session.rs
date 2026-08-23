use tempfile::tempdir;
use zerobeat_catalog::{
    AudioQuality, CatalogError, CatalogFuture, MusicCatalog, ResolvedStream, SearchRequest,
};
use zerobeat_core::{Route, SessionMode, Track};
use zerobeat_daemon::{DaemonError, DaemonServer};
use zerobeat_ipc::IpcConnection;
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
    let server = DaemonServer::bind_with_catalog(&socket, TestCatalog)
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
    let DaemonEvent::Snapshot(AppSnapshot {
        session,
        navigation,
        ..
    }) = event
    else {
        panic!("expected snapshot, got {event:?}");
    };

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
        Box::pin(async { Err(CatalogError::Unavailable("not used".into())) })
    }
}
