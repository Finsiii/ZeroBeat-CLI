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

pub fn prepare_runtime_dir(path: &Path) -> Result<(), RuntimeError> {
    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }

    let metadata = std::fs::symlink_metadata(path)?;
    let owned_by_user = metadata.uid() == rustix::process::getuid().as_raw();
    if !metadata.is_dir() || metadata.file_type().is_symlink() || !owned_by_user {
        return Err(RuntimeError::UnsafeDirectory(path.to_path_buf()));
    }

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}
