use tempfile::tempdir;
use zerobeat_ipc::{IpcConnection, IpcListener};

#[tokio::test]
async fn listener_and_connection_exchange_a_frame() {
    let directory = tempdir().unwrap();
    let endpoint = directory.path().join("transport.sock");
    let mut listener = IpcListener::bind(&endpoint).unwrap();
    let server = tokio::spawn(async move {
        let mut connection = listener.accept().await.unwrap();
        let request: String = connection.receive().await.unwrap();
        assert_eq!(request, "ping");
        connection.send(&"pong").await.unwrap();
    });

    let mut client = IpcConnection::connect(&endpoint).await.unwrap();
    client.send(&"ping").await.unwrap();
    assert_eq!(client.receive::<String>().await.unwrap(), "pong");
    server.await.unwrap();
}

#[tokio::test]
async fn listener_accepts_multiple_clients_without_dropping_the_first() {
    let directory = tempdir().unwrap();
    let endpoint = directory.path().join("multi-client.sock");
    let mut listener = IpcListener::bind(&endpoint).unwrap();
    let server = tokio::spawn(async move {
        let mut first = listener.accept().await.unwrap();
        let mut second = listener.accept().await.unwrap();
        assert_eq!(first.receive::<u8>().await.unwrap(), 1);
        assert_eq!(second.receive::<u8>().await.unwrap(), 2);
    });

    let (first, second) = tokio::join!(
        IpcConnection::connect(&endpoint),
        IpcConnection::connect(&endpoint)
    );
    let mut first = first.unwrap();
    let mut second = second.unwrap();
    first.send(&1_u8).await.unwrap();
    second.send(&2_u8).await.unwrap();
    server.await.unwrap();
}
