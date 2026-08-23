use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChallengeRequest<'a> {
    pub install_id: &'a str,
    pub platform: &'static str,
    pub app_version: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChallengeResponse {
    pub challenge: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProvisionRequest<'a> {
    pub install_id: &'a str,
    pub platform: &'static str,
    pub app_version: &'a str,
    pub public_key: String,
    pub key_version: u32,
    pub challenge: &'a str,
    pub challenge_signature: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProvisionResponse {
    pub device_id: String,
    pub key_version: u32,
}

#[derive(Deserialize)]
pub(crate) struct SearchResponse {
    pub items: Vec<SearchTrack>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchTrack {
    pub video_id: String,
    pub title: Option<String>,
    #[serde(default)]
    pub artists: Vec<Artist>,
    pub duration_seconds: Option<u64>,
    #[serde(default)]
    pub thumbnails: Vec<Thumbnail>,
}

#[derive(Deserialize)]
pub(crate) struct Artist {
    pub name: String,
}

#[derive(Deserialize)]
pub(crate) struct Thumbnail {
    pub url: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

#[derive(Deserialize)]
pub(crate) struct ResolveResponse {
    pub format: ResolveFormat,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolveFormat {
    pub audio_url: Option<String>,
    #[serde(default)]
    pub http_headers: BTreeMap<String, String>,
}
