mod backend;
#[cfg(test)]
mod build_config;
#[cfg(test)]
mod build_config_tests {
    use super::build_config::{
        NativePlatform, classify_platform, has_windows_dependencies, split_path_list,
        windows_triplet,
    };

    #[test]
    fn classifies_supported_native_platforms() {
        assert_eq!(classify_platform("linux"), NativePlatform::Linux);
        assert_eq!(classify_platform("windows"), NativePlatform::Windows);
        assert_eq!(classify_platform("macos"), NativePlatform::Unsupported);
    }

    #[test]
    fn selects_only_x64_msvc_windows_triplet() {
        assert_eq!(windows_triplet("x86_64", "msvc"), Some("x64-windows"));
        assert_eq!(windows_triplet("aarch64", "msvc"), None);
        assert_eq!(windows_triplet("x86_64", "gnu"), None);
    }

    #[test]
    fn splits_windows_search_paths_without_empty_entries() {
        assert_eq!(
            split_path_list(r"C:\\ffmpeg;D:\\curl;;"),
            vec![r"C:\\ffmpeg", r"D:\\curl"]
        );
    }

    #[test]
    fn rejects_incomplete_windows_dependency_layouts() {
        assert!(!has_windows_dependencies(&[], &[]));
    }
}
mod crossfade;
mod dual_deck;
mod error;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod native;
mod player;
mod queue;

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub use backend::CancellationController;
pub use backend::{AudioBackend, BackendTelemetry, SPECTRUM_BAND_COUNT};
pub use crossfade::{CrossfadeConfig, CrossfadeCurve};
pub use dual_deck::DualDeck;
pub use error::{BackendError, PlayerError};
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub use native::{NativeCancellationHandle, NativeEngine, NativeState};
pub use player::{Player, PlayerState};
pub use queue::{QueueItem, StreamSource};
