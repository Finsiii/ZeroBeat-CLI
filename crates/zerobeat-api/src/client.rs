use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::{RequestBuilder, StatusCode};
use tokio::sync::Mutex;
use url::form_urlencoded;
use zerobeat_catalog::{
    AudioQuality, CatalogError, CatalogFuture, MusicCatalog, ResolvedStream, SearchRequest,
};
use zerobeat_core::Track;
use zerobeat_security::{DeviceIdentity, IdentityStore, RequestToSign};

use crate::{
    ApiConfig, ApiError,
    models::{
        ChallengeRequest, ChallengeResponse, ProvisionRequest, ProvisionResponse, ResolveResponse,
        SearchResponse, SearchTrack,
    },
};

const PLATFORM: &str = "desktop";

#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    config: ApiConfig,
    identity: Arc<Mutex<DeviceIdentity>>,
}

impl ApiClient {
    pub async fn connect(config: ApiConfig) -> Result<Self, ApiError> {
        let identity = IdentityStore::load_or_create(&config.identity_path, &config.app_version)?;
        let client = Self {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(15))
                .build()?,
            config,
            identity: Arc::new(Mutex::new(identity)),
        };
        if client.identity.lock().await.device_id().is_none() {
            client.provision().await?;
        }
        Ok(client)
    }

    async fn provision(&self) -> Result<(), ApiError> {
        let mut identity = self.identity.lock().await;
        let challenge = self
            .send_json(
                self.http.post(self.endpoint("/v1/device/challenge")),
                &ChallengeRequest {
                    install_id: identity.install_id(),
                    platform: PLATFORM,
                    app_version: identity.app_version(),
                },
            )
            .await?;
        let challenge: ChallengeResponse = parse_json(challenge)?;
        let request = ProvisionRequest {
            install_id: identity.install_id(),
            platform: PLATFORM,
            app_version: identity.app_version(),
            public_key: identity.public_key_spki_base64()?,
            key_version: 1,
            challenge: &challenge.challenge,
            challenge_signature: identity.sign_challenge(&challenge.challenge)?,
        };
        let response = self
            .send_json(
                self.http.post(self.endpoint("/v1/device/provision")),
                &request,
            )
            .await?;
        let response: ProvisionResponse = parse_json(response)?;
        identity.bind_credential(response.device_id, response.key_version);
        IdentityStore::save(&self.config.identity_path, &identity)?;
        Ok(())
    }

    async fn search(&self, request: SearchRequest) -> Result<Vec<Track>, ApiError> {
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("q", &request.query)
            .append_pair("limit", &request.limit.to_string())
            .finish();
        let response = self.send_signed_get("/v1/app/search/songs", &query).await?;
        let response: SearchResponse = parse_json(response)?;
        Ok(response.items.into_iter().map(track_from_api).collect())
    }

    async fn resolve(
        &self,
        track_id: &str,
        quality: AudioQuality,
    ) -> Result<ResolvedStream, ApiError> {
        let query = {
            let mut serializer = form_urlencoded::Serializer::new(String::new());
            serializer.append_pair("video_id", track_id);
            if quality == AudioQuality::DataSaver {
                serializer.append_pair("quality", "low");
            }
            serializer.finish()
        };
        let response = self
            .send_signed_get("/v1/app/stream/resolve", &query)
            .await?;
        let response: ResolveResponse = parse_json(response)?;
        let url = response
            .format
            .audio_url
            .filter(|url| !url.trim().is_empty())
            .ok_or(ApiError::InvalidResponse("missing audio URL"))?;
        Ok(ResolvedStream::new(url))
    }

    async fn send_signed_get(&self, path: &str, raw_query: &str) -> Result<Vec<u8>, ApiError> {
        let host = self
            .config
            .base_url
            .host_str()
            .ok_or(ApiError::InvalidBaseUrl)?;
        let canonical_path = format!("{}{}", self.config.canonical_prefix, path);
        let signed = {
            let mut identity = self.identity.lock().await;
            let signed = identity.sign_request(
                RequestToSign::get(host, canonical_path, raw_query),
                unix_time_millis()?,
            )?;
            IdentityStore::save(&self.config.identity_path, &identity)?;
            signed
        };
        let mut request = self.http.get(self.endpoint_with_query(path, raw_query));
        for (name, value) in signed.headers {
            request = request.header(name, value);
        }
        self.send(request).await
    }

    async fn send_json<T: serde::Serialize>(
        &self,
        request: RequestBuilder,
        body: &T,
    ) -> Result<Vec<u8>, ApiError> {
        self.send(request.json(body)).await
    }

    async fn send(&self, request: RequestBuilder) -> Result<Vec<u8>, ApiError> {
        let response = request.send().await?;
        let status = response.status();
        let body = response.bytes().await?;
        if !status.is_success() {
            return Err(ApiError::Rejected {
                status: status.as_u16(),
                message: rejection_message(&body, status),
            });
        }
        Ok(body.to_vec())
    }

    fn endpoint(&self, path: &str) -> String {
        format!(
            "{}{}",
            self.config.base_url.as_str().trim_end_matches('/'),
            path
        )
    }

    fn endpoint_with_query(&self, path: &str, raw_query: &str) -> String {
        format!("{}?{}", self.endpoint(path), raw_query)
    }
}

impl MusicCatalog for ApiClient {
    fn search_songs(&self, request: SearchRequest) -> CatalogFuture<'_, Vec<Track>> {
        Box::pin(async move { self.search(request).await.map_err(catalog_error) })
    }

    fn resolve_stream(
        &self,
        track_id: &str,
        quality: AudioQuality,
    ) -> CatalogFuture<'_, ResolvedStream> {
        let track_id = track_id.to_owned();
        Box::pin(async move {
            self.resolve(&track_id, quality)
                .await
                .map_err(catalog_error)
        })
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(body: Vec<u8>) -> Result<T, ApiError> {
    serde_json::from_slice(&body).map_err(|error| ApiError::InvalidJson(error.to_string()))
}

fn track_from_api(track: SearchTrack) -> Track {
    let title = track
        .title
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "Untitled".to_owned());
    let artist = track
        .artists
        .into_iter()
        .map(|artist| artist.name)
        .filter(|name| !name.trim().is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let artist = if artist.is_empty() {
        "Unknown artist".to_owned()
    } else {
        artist
    };
    let thumbnail = track
        .thumbnails
        .into_iter()
        .max_by_key(|thumbnail| u64::from(thumbnail.width) * u64::from(thumbnail.height))
        .map(|thumbnail| thumbnail.url);
    let mut result = Track::new(
        track.video_id,
        title,
        artist,
        track
            .duration_seconds
            .unwrap_or_default()
            .saturating_mul(1_000),
    );
    if let Some(thumbnail) = thumbnail {
        result = result.with_thumbnail(thumbnail);
    }
    result
}

fn rejection_message(body: &[u8], status: StatusCode) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .or_else(|| value.get("message"))
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .or_else(|| String::from_utf8(body.to_vec()).ok())
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| {
            status
                .canonical_reason()
                .unwrap_or("request rejected")
                .to_owned()
        })
}

pub(crate) fn catalog_error(error: ApiError) -> CatalogError {
    match error {
        ApiError::Rejected {
            status: 401 | 403, ..
        } => CatalogError::Unauthorized,
        ApiError::InvalidResponse(message) => CatalogError::InvalidResponse(message.to_owned()),
        error => CatalogError::Unavailable(error.to_string()),
    }
}

fn unix_time_millis() -> Result<i64, ApiError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ApiError::InvalidClock)?;
    i64::try_from(elapsed.as_millis()).map_err(|_| ApiError::InvalidClock)
}
