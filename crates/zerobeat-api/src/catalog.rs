use std::sync::Arc;
use tokio::sync::OnceCell;
use zerobeat_catalog::{
    AudioQuality, CatalogFuture, Lyrics, MusicCatalog, MusicQueue, QueueFuture, QueueRepeatMode,
    QueueSession, QueueStart, ResolvedStream, SearchRequest,
};
use zerobeat_core::Track;

use crate::{ApiClient, ApiConfig, ApiError, client::catalog_error};

#[derive(Clone)]
pub struct ApiCatalog {
    config: ApiConfig,
    client: Arc<OnceCell<ApiClient>>,
}

impl ApiCatalog {
    pub fn new(config: ApiConfig) -> Self {
        Self {
            config,
            client: Arc::new(OnceCell::new()),
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

impl MusicQueue for ApiCatalog {
    fn active_queue(&self) -> QueueFuture<'_, Option<QueueSession>> {
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .active_queue()
                .await
                .map_err(catalog_error)
        })
    }

    fn start_queue(&self, request: QueueStart) -> QueueFuture<'_, QueueSession> {
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .start_queue(request)
                .await
                .map_err(catalog_error)
        })
    }

    fn get_queue(&self, session_id: &str) -> QueueFuture<'_, QueueSession> {
        let session_id = session_id.to_owned();
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .get_queue(&session_id)
                .await
        })
    }

    fn delete_queue(&self, session_id: &str) -> QueueFuture<'_, ()> {
        let session_id = session_id.to_owned();
        Box::pin(async move {
            let client = self.client().await.map_err(catalog_error)?;
            client
                .delete_queue(&session_id)
                .await
                .map_err(catalog_error)
        })
    }

    fn next_queue(&self, session_id: &str) -> QueueFuture<'_, QueueSession> {
        self.forward_queue(session_id, QueueAction::Next)
    }

    fn previous_queue(&self, session_id: &str) -> QueueFuture<'_, QueueSession> {
        self.forward_queue(session_id, QueueAction::Previous)
    }

    fn load_more_queue(&self, session_id: &str) -> QueueFuture<'_, QueueSession> {
        self.forward_queue(session_id, QueueAction::LoadMore)
    }

    fn play_next_queue(
        &self,
        session_id: &str,
        track: zerobeat_core::Track,
    ) -> QueueFuture<'_, QueueSession> {
        let session_id = session_id.to_owned();
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .play_next_queue(&session_id, track)
                .await
        })
    }

    fn add_queue(
        &self,
        session_id: &str,
        track: zerobeat_core::Track,
    ) -> QueueFuture<'_, QueueSession> {
        let session_id = session_id.to_owned();
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .add_queue(&session_id, track)
                .await
        })
    }

    fn play_index_queue(&self, session_id: &str, index: usize) -> QueueFuture<'_, QueueSession> {
        let session_id = session_id.to_owned();
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .play_index_queue(&session_id, index)
                .await
        })
    }

    fn remove_queue(&self, session_id: &str, index: usize) -> QueueFuture<'_, QueueSession> {
        let session_id = session_id.to_owned();
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .remove_queue(&session_id, index)
                .await
        })
    }

    fn clear_upcoming_queue(&self, session_id: &str) -> QueueFuture<'_, QueueSession> {
        self.forward_queue(session_id, QueueAction::ClearUpcoming)
    }

    fn set_shuffle_queue(&self, session_id: &str, enabled: bool) -> QueueFuture<'_, QueueSession> {
        let session_id = session_id.to_owned();
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .set_shuffle_queue(&session_id, enabled)
                .await
        })
    }

    fn set_repeat_queue(
        &self,
        session_id: &str,
        mode: QueueRepeatMode,
    ) -> QueueFuture<'_, QueueSession> {
        let session_id = session_id.to_owned();
        Box::pin(async move {
            self.client()
                .await
                .map_err(catalog_error)?
                .set_repeat_queue(&session_id, mode)
                .await
        })
    }
}

impl ApiCatalog {
    fn forward_queue(
        &self,
        session_id: &str,
        action: QueueAction,
    ) -> QueueFuture<'_, QueueSession> {
        let session_id = session_id.to_owned();
        Box::pin(async move {
            let client = self.client().await.map_err(catalog_error)?;
            match action {
                QueueAction::Next => client.next_queue(&session_id).await,
                QueueAction::Previous => client.previous_queue(&session_id).await,
                QueueAction::LoadMore => client.load_more_queue(&session_id).await,
                QueueAction::ClearUpcoming => client.clear_upcoming_queue(&session_id).await,
            }
        })
    }
}

#[derive(Clone, Copy)]
enum QueueAction {
    Next,
    Previous,
    LoadMore,
    ClearUpcoming,
}
