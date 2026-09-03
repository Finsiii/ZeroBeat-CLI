use std::ffi::OsStr;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
use tempfile::tempdir;
#[cfg(unix)]
use zerobeat_runtime::runtime_dir;
use zerobeat_runtime::{data_dir, socket_path_in};
#[cfg(unix)]
use zerobeat_runtime::{prepare_data_dir, prepare_runtime_dir};

#[cfg(unix)]
#[test]
fn xdg_runtime_directory_is_namespaced() {
    assert_eq!(
        runtime_dir(Some(OsStr::new("/run/user/1000")), 1000),
        std::path::PathBuf::from("/run/user/1000/zerobeat")
    );
}

#[cfg(unix)]
#[test]
fn fallback_directory_is_unique_to_the_user() {
    assert_eq!(
        runtime_dir(None, 1001),
        std::path::PathBuf::from("/tmp/zerobeat-1001")
    );
}

#[cfg(unix)]
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

#[cfg(unix)]
#[test]
fn runtime_directory_is_private() {
    let parent = tempdir().unwrap();
    let path = parent.path().join("zerobeat");

    prepare_runtime_dir(&path).unwrap();

    let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

#[cfg(unix)]
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

#[cfg(unix)]
#[test]
fn nested_data_directory_is_created_privately() {
    let parent = tempdir().unwrap();
    let path = parent.path().join("nested/share/zerobeat");

    prepare_data_dir(&path).unwrap();

    let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

#[cfg(windows)]
#[test]
fn windows_data_directory_uses_local_app_data() {
    assert_eq!(
        data_dir(Some(OsStr::new(r"C:\Users\me\AppData\Local")), None).unwrap(),
        std::path::PathBuf::from(r"C:\Users\me\AppData\Local\ZeroBeat")
    );
}

#[cfg(windows)]
#[test]
fn windows_socket_path_is_a_named_pipe_endpoint() {
    let endpoint = socket_path_in(std::path::Path::new(r"C:\unused"), 11);
    assert_eq!(
        endpoint,
        std::path::PathBuf::from(r"\\.\pipe\ZeroBeat-daemon-v11")
    );
}
