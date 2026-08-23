use std::{error::Error, ffi::OsString, path::PathBuf};

use zerobeat_daemon::DaemonServer;
use zerobeat_runtime::{prepare_runtime_dir, socket_path};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let socket = parse_socket(std::env::args_os().skip(1))?;
    let parent = socket.parent().ok_or("socket path has no parent")?;
    prepare_runtime_dir(parent)?;
    DaemonServer::bind(socket).await?.run().await?;
    Ok(())
}

fn parse_socket(mut arguments: impl Iterator<Item = OsString>) -> Result<PathBuf, &'static str> {
    match arguments.next() {
        None => Ok(socket_path()),
        Some(flag) if flag == "--socket" => arguments
            .next()
            .map(PathBuf::from)
            .ok_or("missing socket path"),
        Some(_) => Err("usage: zerobeatd [--socket PATH]"),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_socket;

    #[test]
    fn explicit_socket_path_is_accepted() {
        let arguments = ["--socket".into(), "/tmp/zerobeat-test.sock".into()];
        assert_eq!(
            parse_socket(arguments.into_iter()).unwrap(),
            std::path::PathBuf::from("/tmp/zerobeat-test.sock")
        );
    }
}
