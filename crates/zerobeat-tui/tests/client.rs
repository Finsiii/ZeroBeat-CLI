use std::time::Duration;

use tempfile::tempdir;
use tokio::net::UnixListener;
use zerobeat_core::Route;
use zerobeat_daemon::DaemonServer;
use zerobeat_protocol::{ClientCommand, DaemonEvent};
use zerobeat_tui::{ClientError, DaemonClient};

#[tokio::test]
async fn client_connect_times_out_on_a_silent_hello_peer() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });

    let started = tokio::time::Instant::now();
    let error = match DaemonClient::connect(&socket).await {
        Ok(_) => panic!("silent peer unexpectedly completed Hello"),
        Err(error) => error,
    };
    assert!(matches!(error, ClientError::Timeout("hello")));
    assert!(started.elapsed() < Duration::from_secs(1));
    server.await.unwrap();
}

#[tokio::test]
async fn client_shutdown_times_out_on_a_silent_shutdown_peer() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut connection = zerobeat_ipc::IpcConnection::from_stream(stream);
        assert!(matches!(
            connection.receive::<ClientCommand>().await.unwrap(),
            ClientCommand::Hello { .. }
        ));
        connection
            .send(&DaemonEvent::Snapshot(Box::default()))
            .await
            .unwrap();
        assert_eq!(
            connection.receive::<ClientCommand>().await.unwrap(),
            ClientCommand::Shutdown
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    });

    let mut client = DaemonClient::connect(&socket).await.unwrap();
    let started = tokio::time::Instant::now();
    let error = client.shutdown().await.unwrap_err();
    assert!(matches!(error, ClientError::Timeout("shutdown")));
    assert!(started.elapsed() < Duration::from_secs(1));
    server.await.unwrap();
}

#[tokio::test]
async fn client_exchanges_snapshots_with_daemon() {
    let directory = tempdir().unwrap();
    let socket = directory.path().join("daemon.sock");
    let server = DaemonServer::bind(&socket).await.unwrap();
    let task = tokio::spawn(server.run());

    let mut client = DaemonClient::connect(&socket).await.unwrap();
    assert_eq!(client.snapshot().navigation.active_route(), Route::Home);

    let snapshot = client
        .execute(ClientCommand::Navigate(Route::Search))
        .await
        .unwrap();
    assert_eq!(snapshot.navigation.active_route(), Route::Search);

    client.shutdown().await.unwrap();
    task.await.unwrap().unwrap();
}
