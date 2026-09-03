#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativePlatform {
    Linux,
    Windows,
    Unsupported,
}

pub(crate) fn classify_platform(target_os: &str) -> NativePlatform {
    match target_os {
        "linux" => NativePlatform::Linux,
        "windows" => NativePlatform::Windows,
        _ => NativePlatform::Unsupported,
    }
}

pub(crate) fn windows_triplet(target_arch: &str, target_env: &str) -> Option<&'static str> {
    (target_arch == "x86_64" && target_env == "msvc").then_some("x64-windows")
}

pub(crate) fn split_path_list(value: &str) -> Vec<&str> {
    value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect()
}

pub(crate) fn has_windows_dependencies(include_dirs: &[PathBuf], lib_dirs: &[PathBuf]) -> bool {
    ["libavformat/avformat.h", "curl/curl.h"]
        .iter()
        .all(|header| {
            include_dirs
                .iter()
                .any(|directory| directory.join(header).is_file())
        })
        && [
            "avformat.lib",
            "avcodec.lib",
            "avutil.lib",
            "swresample.lib",
            "libcurl.lib",
        ]
        .iter()
        .all(|library| {
            lib_dirs
                .iter()
                .any(|directory| directory.join(library).is_file())
        })
}
use std::path::PathBuf;
