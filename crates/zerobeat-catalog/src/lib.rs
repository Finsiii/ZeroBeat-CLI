mod error;
mod provider;
mod request;
mod stream;

pub use error::CatalogError;
pub use provider::{CatalogFuture, MusicCatalog};
pub use request::SearchRequest;
pub use stream::{AudioQuality, ResolvedStream};
