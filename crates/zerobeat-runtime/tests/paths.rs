use std::{ffi::OsStr, os::unix::fs::PermissionsExt};

use tempfile::tempdir;
use zerobeat_runtime::{prepare_runtime_dir, runtime_dir};

#[test]
fn xdg_runtime_directory_is_namespaced() {
    assert_eq!(
        runtime_dir(Some(OsStr::new("/run/user/1000")), 1000),
        std::path::PathBuf::from("/run/user/1000/zerobeat")
    );
}

#[test]
fn fallback_directory_is_unique_to_the_user() {
    assert_eq!(
        runtime_dir(None, 1001),
        std::path::PathBuf::from("/tmp/zerobeat-1001")
    );
}

#[test]
fn runtime_directory_is_private() {
    let parent = tempdir().unwrap();
    let path = parent.path().join("zerobeat");

    prepare_runtime_dir(&path).unwrap();

    let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}
