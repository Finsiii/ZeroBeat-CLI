use tokio::sync::OnceCell;
use zerobeat_catalog::{
    AudioQuality, CatalogFuture, Lyrics, MusicCatalog, RadioPage, RadioRequest, ResolvedStream,
    SearchRequest,
};
use zerobeat_core::Track;

use crate::{ApiClient, ApiConfig, ApiError, client::catalog_error};

pub struct ApiCatalog {
    config: ApiConfig,
    client: OnceCell<ApiClient>,
}

impl ApiCatalog {
    pub fn new(config: ApiConfig) -> Self {
        Self {
            config,
            client: OnceCell::new(),
        }
    }

    async fn client(&self) -> Result<&ApiClient, ApiError> {
        self.client
            .get_or_try_init(|| ApiClient::connect(self.config.clone()))
            .await
    }
}

impl MusicCatalog for ApiCatalog {
    fn search_songs(&self, request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .search_songs(request)
                .await
        })
    }

    fn radio_tracks(&self, request: RadioRequest) -> CatalogFuture<'_, RadioPage> {
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .radio_tracks(request)
                .await
        })
    }

    fn resolve_stream(
        &self,
        track_id: &str,
        quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream> {
        let track_id = track_id.to_owned();
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .resolve_stream(&track_id, quality)
                .await
        })
    }

    fn lyrics(&self, track: &Track) -> CatalogFuture<'_, Option<Lyrics>> {
        let track = track.clone();
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .lyrics(&track)
                .await
        })
    }
}
