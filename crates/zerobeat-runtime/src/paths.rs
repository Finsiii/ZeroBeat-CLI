use std::{
    ffi::OsStr,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use crate::RuntimeError;

pub fn runtime_dir(xdg_runtime_dir: Option<&OsStr>, uid: u32) -> PathBuf {
    xdg_runtime_dir.map_or_else(
        || PathBuf::from(format!("/tmp/zerobeat-{uid}")),
        |directory| Path::new(directory).join("zerobeat"),
    )
}

pub fn current_runtime_dir() -> PathBuf {
    let xdg = std::env::var_os("XDG_RUNTIME_DIR");
    runtime_dir(xdg.as_deref(), rustix::process::getuid().as_raw())
}

pub fn socket_path() -> PathBuf {
    current_runtime_dir().join("daemon.sock")
}

pub fn data_dir(
    xdg_data_home: Option<&OsStr>,
    home_dir: Option<&OsStr>,
) -> Result<PathBuf, RuntimeError> {
    if let Some(directory) = xdg_data_home {
        return Ok(Path::new(directory).join("zerobeat"));
    }
    home_dir
        .map(|directory| Path::new(directory).join(".local/share/zerobeat"))
        .ok_or(RuntimeError::MissingHomeDirectory)
}

pub fn current_data_dir() -> Result<PathBuf, RuntimeError> {
    let xdg = std::env::var_os("XDG_DATA_HOME");
    let home = std::env::var_os("HOME");
    data_dir(xdg.as_deref(), home.as_deref())
}

pub fn prepare_runtime_dir(path: &Path) -> Result<(), RuntimeError> {
    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }

    secure_private_directory(path)
}

pub fn prepare_data_dir(path: &Path) -> Result<(), RuntimeError> {
    std::fs::create_dir_all(path)?;
    secure_private_directory(path)
}

fn secure_private_directory(path: &Path) -> Result<(), RuntimeError> {
    let metadata = std::fs::symlink_metadata(path)?;
    let owned_by_user = metadata.uid() == rustix::process::getuid().as_raw();
    if !metadata.is_dir() || metadata.file_type().is_symlink() || !owned_by_user {
        return Err(RuntimeError::UnsafeDirectory(path.to_path_buf()));
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}
