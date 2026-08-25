use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use zerobeat_ipc::{IpcError, PeerCredentials};
use zerobeat_protocol::{ClientCommand, DaemonEvent, PROTOCOL_VERSION};
use zerobeat_runtime::prepare_runtime_dir;

use crate::{ClientError, DaemonClient};

const LEGACY_DAEMON_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("failed to prepare runtime directory: {0}")]
    Runtime(#[from] zerobeat_runtime::RuntimeError),
    #[error("failed to locate or start zerobeatd: {0}")]
    Process(#[from] std::io::Error),
    #[error("failed to connect to zerobeatd: {0}")]
    Client(#[from] ClientError),
    #[error("daemon identity could not be verified: {0}")]
    Security(String),
}

pub async fn connect_or_spawn(socket: &Path) -> Result<DaemonClient, LaunchError> {
    if let Some(parent) = socket.parent() {
        prepare_runtime_dir(parent)?;
    }
    let daemon = daemon_executable()?;
    retire_legacy_daemon(socket, &daemon).await?;
    if let Some(client) = connect_existing_or_retire(socket, &daemon).await? {
        return Ok(client);
    }

    Command::new(&daemon)
        .arg("--socket")
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let mut last_error = None;
    for _ in 0..40 {
        match connect_existing_or_retire(socket, &daemon).await? {
            Some(client) => return Ok(client),
            None => {
                last_error = Some(ClientError::Ipc(IpcError::Io(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "daemon unavailable",
                ))));
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    Err(last_error.expect("connection retry must run").into())
}

async fn connect_existing_or_retire(
    socket: &Path,
    daemon: &Path,
) -> Result<Option<DaemonClient>, LaunchError> {
    match DaemonClient::connect(socket).await {
        Ok(mut client) => {
            if connected_daemon_is_current(&client, socket, daemon)? {
                return Ok(Some(client));
            }
            client.shutdown().await?;
            wait_for_socket_release(socket, daemon).await
        }
        Err(error) if daemon_is_unavailable(&error) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn wait_for_socket_release(
    socket: &Path,
    daemon: &Path,
) -> Result<Option<DaemonClient>, LaunchError> {
    for _ in 0..40 {
        match DaemonClient::connect(socket).await {
            Ok(client) => {
                if connected_daemon_is_current(&client, socket, daemon)? {
                    return Ok(Some(client));
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(ClientError::Ipc(IpcError::Io(error)))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                return Ok(None);
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AddrInUse,
        "stale daemon did not release its socket",
    )
    .into())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExecutableIdentity {
    device: u64,
    inode: u64,
}

fn daemon_executable_identity(path: &Path) -> std::io::Result<ExecutableIdentity> {
    let metadata = fs::metadata(path)?;
    Ok(ExecutableIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn peer_executable_identity(pid: u32) -> std::io::Result<ExecutableIdentity> {
    daemon_executable_identity(&PathBuf::from(format!("/proc/{pid}/exe")))
}

fn connected_daemon_is_current(
    client: &DaemonClient,
    socket: &Path,
    daemon: &Path,
) -> Result<bool, LaunchError> {
    let credentials = client.peer_credentials().map_err(|error| {
        LaunchError::Security(format!("failed to read daemon peer credentials: {error}"))
    })?;
    let Some(parent) = socket.parent() else {
        return Err(LaunchError::Security(
            "daemon socket has no parent directory".into(),
        ));
    };
    verify_peer_identity(credentials, parent, daemon)
}

fn verify_peer_identity(
    credentials: PeerCredentials,
    socket_parent: &Path,
    daemon: &Path,
) -> Result<bool, LaunchError> {
    let owner = fs::metadata(socket_parent).map_err(|error| {
        LaunchError::Security(format!("failed to inspect runtime directory: {error}"))
    })?;
    if credentials.uid != owner.uid() {
        return Err(LaunchError::Security(format!(
            "daemon peer UID {} does not own runtime directory (owner UID {})",
            credentials.uid,
            owner.uid()
        )));
    }
    let Some(pid) = credentials.pid else {
        return Err(LaunchError::Security(
            "daemon peer did not provide a PID".into(),
        ));
    };
    let expected = daemon_executable_identity(daemon).map_err(|error| {
        LaunchError::Security(format!(
            "failed to inspect current daemon executable: {error}"
        ))
    })?;
    let actual = peer_executable_identity(pid).map_err(|error| {
        LaunchError::Security(format!(
            "failed to inspect executable for daemon PID {pid}: {error}"
        ))
    })?;
    Ok(actual == expected)
}

async fn retire_legacy_daemon(current_socket: &Path, daemon: &Path) -> Result<(), LaunchError> {
    let Some(parent) = current_socket.parent() else {
        return Ok(());
    };
    let legacy_socket = parent.join("daemon.sock");
    if legacy_socket == current_socket {
        return Ok(());
    }
    let mut connection = match zerobeat_ipc::IpcConnection::connect(&legacy_socket).await {
        Ok(connection) => connection,
        Err(IpcError::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(());
        }
        Err(error) => return Err(ClientError::Ipc(error).into()),
    };
    let credentials = connection.peer_credentials().map_err(|error| {
        LaunchError::Security(format!(
            "failed to read legacy daemon peer credentials: {error}"
        ))
    })?;
    if verify_peer_identity(credentials, parent, daemon)? {
        return Ok(());
    }
    tokio::time::timeout(
        LEGACY_DAEMON_TIMEOUT,
        connection.send(&ClientCommand::Hello {
            protocol_version: PROTOCOL_VERSION,
        }),
    )
    .await
    .map_err(|_| LaunchError::Client(ClientError::Timeout("legacy hello")))?
    .map_err(|error| LaunchError::Client(ClientError::Ipc(error)))?;
    let response = tokio::time::timeout(LEGACY_DAEMON_TIMEOUT, connection.receive::<DaemonEvent>())
        .await
        .map_err(|_| LaunchError::Client(ClientError::Timeout("legacy hello response")))?
        .map_err(|error| LaunchError::Client(ClientError::Ipc(error)))?;
    let DaemonEvent::Rejected(reason) = response else {
        return Ok(());
    };
    if !reason.starts_with("unsupported protocol version") {
        return Ok(());
    }
    tokio::time::timeout(
        LEGACY_DAEMON_TIMEOUT,
        connection.send(&ClientCommand::Shutdown),
    )
    .await
    .map_err(|_| LaunchError::Client(ClientError::Timeout("legacy shutdown")))?
    .map_err(|error| LaunchError::Client(ClientError::Ipc(error)))?;
    tokio::time::timeout(LEGACY_DAEMON_TIMEOUT, connection.receive::<DaemonEvent>())
        .await
        .map_err(|_| LaunchError::Client(ClientError::Timeout("legacy shutdown response")))?
        .map_err(|error| LaunchError::Client(ClientError::Ipc(error)))?;
    Ok(())
}

fn daemon_is_unavailable(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::Ipc(IpcError::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            )
    )
}

fn daemon_executable() -> Result<PathBuf, std::io::Error> {
    if let Some(path) = std::env::var_os("ZEROBEATD_PATH") {
        return Ok(path.into());
    }
    let current = std::env::current_exe()?;
    Ok(current.with_file_name("zerobeatd"))
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::MetadataExt, process};

    use tempfile::tempdir;
    use tokio::net::UnixListener;
    use zerobeat_ipc::{IpcConnection, PeerCredentials};
    use zerobeat_protocol::{ClientCommand, DaemonEvent, PROTOCOL_VERSION};

    use super::{
        connect_existing_or_retire, daemon_executable_identity, peer_executable_identity,
        retire_legacy_daemon, verify_peer_identity,
    };
    use crate::{ClientError, LaunchError};

    #[test]
    fn peer_executable_identity_matches_the_current_test_process() {
        let current = std::env::current_exe().unwrap();
        assert_eq!(
            daemon_executable_identity(&current).unwrap(),
            peer_executable_identity(process::id()).unwrap()
        );
    }

    #[test]
    fn executable_identity_detects_a_replaced_inode() {
        let directory = tempdir().unwrap();
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        assert_ne!(
            daemon_executable_identity(&first).unwrap(),
            daemon_executable_identity(&second).unwrap()
        );
    }

    #[tokio::test]
    async fn same_protocol_stale_daemon_is_gracefully_retired() {
        let directory = tempdir().unwrap();
        let socket = directory.path().join("daemon.sock");
        let daemon = directory.path().join("zerobeatd");
        fs::write(&daemon, b"new daemon").unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        let stale = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut connection = IpcConnection::from_stream(stream);
            assert_eq!(
                connection.receive::<ClientCommand>().await.unwrap(),
                ClientCommand::Hello {
                    protocol_version: PROTOCOL_VERSION
                }
            );
            connection
                .send(&DaemonEvent::Snapshot(Box::default()))
                .await
                .unwrap();
            assert_eq!(
                connection.receive::<ClientCommand>().await.unwrap(),
                ClientCommand::Shutdown
            );
            connection.send(&DaemonEvent::Acknowledged).await.unwrap();
        });

        assert!(
            connect_existing_or_retire(&socket, &daemon)
                .await
                .unwrap()
                .is_none()
        );
        stale.await.unwrap();
    }

    #[tokio::test]
    async fn silent_legacy_daemon_returns_a_bounded_timeout_error() {
        let directory = tempdir().unwrap();
        let current_socket = directory.path().join("daemon-v11.sock");
        let daemon = directory.path().join("zerobeatd");
        fs::write(&daemon, b"new daemon").unwrap();
        let legacy_socket = directory.path().join("daemon.sock");
        let listener = UnixListener::bind(&legacy_socket).unwrap();
        let silent = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        });

        let started = tokio::time::Instant::now();
        let error = retire_legacy_daemon(&current_socket, &daemon)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            LaunchError::Client(ClientError::Timeout(_))
        ));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        silent.await.unwrap();
    }

    #[tokio::test]
    async fn legacy_daemon_io_failure_is_propagated() {
        let directory = tempdir().unwrap();
        let current_socket = directory.path().join("daemon-v11.sock");
        let daemon = directory.path().join("zerobeatd");
        fs::write(&daemon, b"new daemon").unwrap();
        let legacy_socket = directory.path().join("daemon.sock");
        let listener = UnixListener::bind(&legacy_socket).unwrap();
        let closed = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            drop(stream);
        });

        let error = retire_legacy_daemon(&current_socket, &daemon)
            .await
            .unwrap_err();
        assert!(matches!(error, LaunchError::Client(ClientError::Ipc(_))));
        closed.await.unwrap();
    }

    #[tokio::test]
    async fn spawned_reconnect_never_returns_a_same_protocol_stale_daemon() {
        let directory = tempdir().unwrap();
        let socket = directory.path().join("daemon.sock");
        let daemon = directory.path().join("zerobeatd");
        fs::write(&daemon, b"new daemon").unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        let stale = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut connection = IpcConnection::from_stream(stream);
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
            connection.send(&DaemonEvent::Acknowledged).await.unwrap();
        });

        assert!(
            connect_existing_or_retire(&socket, &daemon)
                .await
                .unwrap()
                .is_none()
        );
        stale.await.unwrap();
    }

    #[test]
    fn peer_uid_mismatch_is_a_security_error() {
        let directory = tempdir().unwrap();
        let socket_parent = directory.path();
        let daemon = directory.path().join("zerobeatd");
        fs::write(&daemon, b"daemon").unwrap();
        let result = verify_peer_identity(
            PeerCredentials {
                pid: Some(process::id()),
                uid: u32::MAX,
            },
            socket_parent,
            &daemon,
        );
        assert!(matches!(result, Err(LaunchError::Security(_))));
    }

    #[test]
    fn missing_peer_identity_is_a_security_error() {
        let directory = tempdir().unwrap();
        let daemon = directory.path().join("zerobeatd");
        fs::write(&daemon, b"daemon").unwrap();
        let result = verify_peer_identity(
            PeerCredentials {
                pid: None,
                uid: fs::metadata(directory.path()).unwrap().uid(),
            },
            directory.path(),
            &daemon,
        );
        assert!(matches!(result, Err(LaunchError::Security(_))));
    }

    #[test]
    fn unavailable_daemon_executable_is_a_security_error() {
        let directory = tempdir().unwrap();
        let result = verify_peer_identity(
            PeerCredentials {
                pid: Some(process::id()),
                uid: fs::metadata(directory.path()).unwrap().uid(),
            },
            directory.path(),
            &directory.path().join("missing-zerobeatd"),
        );
        assert!(matches!(result, Err(LaunchError::Security(_))));
    }

    #[tokio::test]
    async fn incompatible_legacy_daemon_is_shut_down_before_upgrade() {
        let directory = tempdir().unwrap();
        let legacy_socket = directory.path().join("daemon.sock");
        let listener = UnixListener::bind(&legacy_socket).unwrap();
        let legacy = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut connection = IpcConnection::from_stream(stream);
            assert_eq!(
                connection.receive::<ClientCommand>().await.unwrap(),
                ClientCommand::Hello {
                    protocol_version: PROTOCOL_VERSION
                }
            );
            connection
                .send(&DaemonEvent::Rejected(format!(
                    "unsupported protocol version {PROTOCOL_VERSION}"
                )))
                .await
                .unwrap();
            assert_eq!(
                connection.receive::<ClientCommand>().await.unwrap(),
                ClientCommand::Shutdown
            );
            connection.send(&DaemonEvent::Acknowledged).await.unwrap();
        });

        let daemon = directory.path().join("zerobeatd");
        fs::write(&daemon, b"new daemon").unwrap();
        retire_legacy_daemon(&directory.path().join("daemon-v9.sock"), &daemon)
            .await
            .unwrap();

        legacy.await.unwrap();
    }
}
