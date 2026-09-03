use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

use crate::RuntimeError;

#[cfg(windows)]
pub fn runtime_dir(xdg_runtime_dir: Option<&OsStr>, uid: u32) -> PathBuf {
    let _ = (xdg_runtime_dir, uid);
    PathBuf::from(r"\\.\pipe")
}

#[cfg(not(windows))]
pub fn runtime_dir(xdg_runtime_dir: Option<&OsStr>, uid: u32) -> PathBuf {
    xdg_runtime_dir.map_or_else(
        || PathBuf::from(format!("/tmp/zerobeat-{uid}")),
        |directory| Path::new(directory).join("zerobeat"),
    )
}

#[cfg(windows)]
pub fn current_runtime_dir() -> PathBuf {
    runtime_dir(None, 0)
}

#[cfg(not(windows))]
pub fn current_runtime_dir() -> PathBuf {
    let xdg = std::env::var_os("XDG_RUNTIME_DIR");
    runtime_dir(xdg.as_deref(), rustix::process::getuid().as_raw())
}

pub fn socket_path(protocol_version: u16) -> PathBuf {
    socket_path_in(&current_runtime_dir(), protocol_version)
}

#[cfg(windows)]
pub fn socket_path_in(runtime_directory: &Path, protocol_version: u16) -> PathBuf {
    let _ = runtime_directory;
    PathBuf::from(format!(r"\\.\pipe\ZeroBeat-daemon-v{protocol_version}"))
}

#[cfg(not(windows))]
pub fn socket_path_in(runtime_directory: &Path, protocol_version: u16) -> PathBuf {
    runtime_directory.join(format!("daemon-v{protocol_version}.sock"))
}

#[cfg(windows)]
pub fn data_dir(
    xdg_data_home: Option<&OsStr>,
    home_dir: Option<&OsStr>,
) -> Result<PathBuf, RuntimeError> {
    let local_app_data = xdg_data_home
        .or(home_dir)
        .ok_or(RuntimeError::MissingHomeDirectory)?;
    let local_app_data = Path::new(local_app_data);
    Ok(if xdg_data_home.is_some() {
        local_app_data.join("ZeroBeat")
    } else {
        local_app_data
            .join("AppData")
            .join("Local")
            .join("ZeroBeat")
    })
}

#[cfg(not(windows))]
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

#[cfg(windows)]
pub fn current_data_dir() -> Result<PathBuf, RuntimeError> {
    let local_app_data = std::env::var_os("LOCALAPPDATA");
    let user_profile = std::env::var_os("USERPROFILE");
    data_dir(local_app_data.as_deref(), user_profile.as_deref())
}

#[cfg(not(windows))]
pub fn current_data_dir() -> Result<PathBuf, RuntimeError> {
    let xdg = std::env::var_os("XDG_DATA_HOME");
    let home = std::env::var_os("HOME");
    data_dir(xdg.as_deref(), home.as_deref())
}

pub fn prepare_runtime_dir(path: &Path) -> Result<(), RuntimeError> {
    #[cfg(windows)]
    if is_named_pipe_namespace(path) {
        return Ok(());
    }

    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }

    secure_private_directory(path)
}

pub fn prepare_data_dir(path: &Path) -> Result<(), RuntimeError> {
    #[cfg(windows)]
    reject_reparse_ancestors(path)?;
    std::fs::create_dir_all(path)?;
    secure_private_directory(path)
}

#[cfg(windows)]
fn reject_reparse_ancestors(path: &Path) -> Result<(), RuntimeError> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        match std::fs::symlink_metadata(candidate) {
            Ok(metadata) if metadata.file_attributes() & 0x400 != 0 => {
                return Err(RuntimeError::UnsafeDirectory(candidate.to_path_buf()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        current = candidate.parent();
    }
    Ok(())
}

#[cfg(windows)]
fn secure_private_directory(path: &Path) -> Result<(), RuntimeError> {
    let metadata = std::fs::symlink_metadata(path)?;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(RuntimeError::UnsafeDirectory(path.to_path_buf()));
    }
    Ok(())
}

#[cfg(unix)]
fn secure_private_directory(path: &Path) -> Result<(), RuntimeError> {
    let metadata = std::fs::symlink_metadata(path)?;
    let owned_by_user = metadata.uid() == rustix::process::getuid().as_raw();
    if !metadata.is_dir() || metadata.file_type().is_symlink() || !owned_by_user {
        return Err(RuntimeError::UnsafeDirectory(path.to_path_buf()));
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(windows)]
fn is_named_pipe_namespace(path: &Path) -> bool {
    path.to_string_lossy().eq_ignore_ascii_case(r"\\.\pipe")
}
