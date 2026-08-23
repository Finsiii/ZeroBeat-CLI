use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

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
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.uid() != rustix::process::getuid().as_raw()
        {
            return Err(SecurityError::UnsafeIdentity);
        }
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
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
}
