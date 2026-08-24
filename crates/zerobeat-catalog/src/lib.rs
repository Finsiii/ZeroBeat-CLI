mod error;
mod lyrics;
mod provider;
mod queue;
mod request;
mod stream;

pub use error::CatalogError;
pub use lyrics::{Lyrics, LyricsLine};
pub use provider::{CatalogFuture, MusicCatalog};
pub use queue::{MusicQueue, QueueFuture, QueueRepeatMode, QueueSession, QueueStart};
pub use request::SearchRequest;
pub use stream::{AudioQuality, ResolvedStream};
