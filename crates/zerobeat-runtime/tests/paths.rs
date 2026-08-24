use std::{ffi::OsStr, os::unix::fs::PermissionsExt};

use tempfile::tempdir;
use zerobeat_runtime::{
    data_dir, prepare_data_dir, prepare_runtime_dir, runtime_dir, socket_path_in,
};

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
fn daemon_socket_is_namespaced_by_protocol_version() {
    let runtime = std::path::Path::new("/run/user/1000/zerobeat");

    assert_eq!(
        socket_path_in(runtime, 9),
        std::path::PathBuf::from("/run/user/1000/zerobeat/daemon-v9.sock")
    );
    assert_ne!(socket_path_in(runtime, 8), socket_path_in(runtime, 9));
    assert_eq!(
        socket_path_in(runtime, 11),
        std::path::PathBuf::from("/run/user/1000/zerobeat/daemon-v11.sock")
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

#[test]
fn data_directory_follows_xdg_and_home_fallback() {
    assert_eq!(
        data_dir(Some(OsStr::new("/data/me")), None).unwrap(),
        std::path::PathBuf::from("/data/me/zerobeat")
    );
    assert_eq!(
        data_dir(None, Some(OsStr::new("/home/me"))).unwrap(),
        std::path::PathBuf::from("/home/me/.local/share/zerobeat")
    );
}

#[test]
fn nested_data_directory_is_created_privately() {
    let parent = tempdir().unwrap();
    let path = parent.path().join("nested/share/zerobeat");

    prepare_data_dir(&path).unwrap();

    let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}
