mod backend;
mod crossfade;
mod error;
#[cfg(target_os = "linux")]
mod native;
mod player;
mod queue;

pub use backend::{AudioBackend, BackendTelemetry};
pub use crossfade::{CrossfadeConfig, CrossfadeCurve};
pub use error::{BackendError, PlayerError};
#[cfg(target_os = "linux")]
pub use native::{NativeEngine, NativeState};
pub use player::{Player, PlayerState};
pub use queue::{QueueItem, StreamSource};
