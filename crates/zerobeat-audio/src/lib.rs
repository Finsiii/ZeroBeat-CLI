mod backend;
mod crossfade;
mod error;
mod player;
mod queue;

pub use backend::AudioBackend;
pub use crossfade::{CrossfadeConfig, CrossfadeCurve};
pub use error::{BackendError, PlayerError};
pub use player::{Player, PlayerState};
pub use queue::{QueueItem, StreamSource};
