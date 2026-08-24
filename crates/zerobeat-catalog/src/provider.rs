use std::{future::Future, pin::Pin};

use zerobeat_core::Track;

use crate::{AudioQuality, CatalogError, Lyrics, ResolvedStream, SearchRequest};

pub type CatalogFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, CatalogError>> + Send + 'a>>;

pub trait MusicCatalog: Send + Sync {
    fn search_songs(&self, request: SearchRequest) -> CatalogFuture<'_, Vec<Track>>;

    fn resolve_stream(
        &self,
        track_id: &str,
        quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream>;

    fn lyrics(&self, _track: &Track) -> CatalogFuture<'_, Option<Lyrics>> {
        Box::pin(async { Ok(None) })
    }
}
