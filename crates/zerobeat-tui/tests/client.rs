use tempfile::tempdir;
use zerobeat_core::Route;
use zerobeat_daemon::DaemonServer;
use zerobeat_protocol::ClientCommand;
use zerobeat_tui::DaemonClient;

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
