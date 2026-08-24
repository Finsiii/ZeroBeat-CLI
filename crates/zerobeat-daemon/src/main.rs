use std::{error::Error, ffi::OsString, path::PathBuf};

use zerobeat_api::{ApiCatalog, ApiConfig};
use zerobeat_audio::{DualDeck, NativeEngine};
use zerobeat_daemon::DaemonServer;
use zerobeat_runtime::{current_data_dir, prepare_data_dir, prepare_runtime_dir, socket_path};
use zerobeat_storage::Database;

const DEFAULT_API_URL: &str = "https://api.zerobits.tech/music";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let socket = parse_socket(std::env::args_os().skip(1))?;
    let parent = socket.parent().ok_or("socket path has no parent")?;
    prepare_runtime_dir(parent)?;
    let data_directory = current_data_dir()?;
    prepare_data_dir(&data_directory)?;
    let api_url = std::env::var("ZEROBEAT_API_URL").unwrap_or_else(|_| DEFAULT_API_URL.to_owned());
    let config = ApiConfig::new(
        api_url,
        "/music",
        data_directory.join("device.identity"),
        format!("cli/{}+1", env!("CARGO_PKG_VERSION")),
    )?;
    let audio = DualDeck::new(NativeEngine::new()?, NativeEngine::new()?);
    let database = Database::open(data_directory.join("guest.db"))?;
    DaemonServer::bind_with_services_and_storage(
        socket,
        ApiCatalog::new(config),
        audio,
        database,
        data_directory.join("downloads"),
    )
    .await?
    .run()
    .await?;
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
