mod backend;
mod crossfade;
mod dual_deck;
mod error;
#[cfg(target_os = "linux")]
mod native;
mod player;
mod queue;

#[cfg(target_os = "linux")]
pub use backend::CancellationController;
pub use backend::{AudioBackend, BackendTelemetry, SPECTRUM_BAND_COUNT};
pub use crossfade::{CrossfadeConfig, CrossfadeCurve};
pub use dual_deck::DualDeck;
pub use error::{BackendError, PlayerError};
#[cfg(target_os = "linux")]
pub use native::{NativeCancellationHandle, NativeEngine, NativeState};
pub use player::{Player, PlayerState};
pub use queue::{QueueItem, StreamSource};
