mod error;
mod lyrics;
mod provider;
mod request;
mod stream;

pub use error::CatalogError;
pub use lyrics::{Lyrics, LyricsLine};
pub use provider::{CatalogFuture, MusicCatalog};
pub use request::SearchRequest;
pub use stream::{AudioQuality, ResolvedStream};
