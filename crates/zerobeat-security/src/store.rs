use std::{fs::OpenOptions, io::Write, path::Path};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};

use p256::ecdsa::SigningKey;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::{DeviceIdentity, SecurityError};

#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    format_version: u8,
    install_id: String,
    app_version: String,
    private_key: Vec<u8>,
    device_id: Option<String>,
    key_version: u32,
    counter: u64,
}

pub struct IdentityStore;

impl IdentityStore {
    pub fn load_or_create(
        path: impl AsRef<Path>,
        app_version: &str,
    ) -> Result<DeviceIdentity, SecurityError> {
        let path = path.as_ref();
        if path.exists() {
            return Self::load(path);
        }
        let identity = DeviceIdentity::generate(app_version)?;
        Self::save(path, &identity)?;
        Ok(identity)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<DeviceIdentity, SecurityError> {
        let path = path.as_ref();
        let metadata = std::fs::symlink_metadata(path)?;
        if !is_safe_identity_file(&metadata) || !owned_by_current_user(&metadata) {
            return Err(SecurityError::UnsafeIdentity);
        }

        #[cfg(unix)]
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        let stored: StoredIdentity = rmp_serde::from_slice(&std::fs::read(path)?)?;
        if stored.format_version != 1 || stored.private_key.len() != 32 {
            return Err(SecurityError::InvalidKey);
        }
        let signing_key =
            SigningKey::from_slice(&stored.private_key).map_err(|_| SecurityError::InvalidKey)?;
        Ok(DeviceIdentity {
            install_id: stored.install_id,
            app_version: stored.app_version,
            signing_key,
            device_id: stored.device_id,
            key_version: stored.key_version,
            counter: stored.counter,
        })
    }

    pub fn save(path: impl AsRef<Path>, identity: &DeviceIdentity) -> Result<(), SecurityError> {
        let path = path.as_ref();
        let stored = StoredIdentity {
            format_version: 1,
            install_id: identity.install_id.clone(),
            app_version: identity.app_version.clone(),
            private_key: identity.signing_key.to_bytes().to_vec(),
            device_id: identity.device_id.clone(),
            key_version: identity.key_version,
            counter: identity.counter,
        };
        let payload = rmp_serde::to_vec_named(&stored)?;
        let mut suffix = [0_u8; 8];
        OsRng.fill_bytes(&mut suffix);
        let temporary = path.with_extension(format!("tmp-{}", hex::encode(suffix)));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        drop(file);
        atomic_replace(&temporary, path)?;
        let metadata = std::fs::symlink_metadata(path)?;
        if !is_safe_identity_file(&metadata) || !owned_by_current_user(&metadata) {
            return Err(SecurityError::UnsafeIdentity);
        }
        #[cfg(unix)]
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
}

#[cfg(windows)]
fn is_safe_identity_file(metadata: &std::fs::Metadata) -> bool {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return false;
    }
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

#[cfg(unix)]
fn is_safe_identity_file(metadata: &std::fs::Metadata) -> bool {
    metadata.is_file() && !metadata.file_type().is_symlink()
}

#[cfg(unix)]
fn owned_by_current_user(metadata: &std::fs::Metadata) -> bool {
    metadata.uid() == rustix::process::getuid().as_raw()
}

#[cfg(windows)]
fn owned_by_current_user(_metadata: &std::fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn atomic_replace(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(temporary, destination)
}

#[cfg(windows)]
fn atomic_replace(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    let temporary: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let flags = windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
        | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH;
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            temporary.as_ptr(),
            destination.as_ptr(),
            flags,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
