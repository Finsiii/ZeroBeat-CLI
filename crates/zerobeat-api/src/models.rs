use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use zerobeat_catalog::{QueueRepeatMode, QueueSession, QueueStart};
use zerobeat_core::Track;

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
    #[serde(default)]
    pub expires_at_unix_ms: i64,
}

#[derive(Deserialize)]
pub(crate) struct LyricsResponse {
    pub found: bool,
    pub source: Option<LyricsSource>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LyricsSource {
    #[serde(default)]
    pub sync_type: String,
    #[serde(default)]
    pub lines: Vec<LyricsLine>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LyricsLine {
    #[serde(default)]
    pub start_time_ms: String,
    pub words: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueStartRequest {
    pub tracks: Vec<QueueTrack>,
    pub current_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track: Option<QueueTrack>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
    pub shuffle: bool,
    pub repeat_mode: String,
    pub endless_queue: bool,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueTrack {
    pub video_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub artist: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub duration_sec: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thumbnail: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub result_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueueSessionResponse {
    pub id: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub tracks: Vec<QueueTrack>,
    #[serde(default)]
    pub current_index: usize,
    #[serde(default)]
    pub playlist_id: Option<String>,
    #[serde(default)]
    pub playlist_name: Option<String>,
    #[serde(default)]
    pub playlist_type: Option<String>,
    #[serde(default)]
    pub continuation: Option<String>,
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default)]
    pub play_order: Vec<usize>,
    #[serde(default)]
    pub repeat_mode: String,
    #[serde(default)]
    pub endless_queue: bool,
    #[serde(default)]
    pub revision: i64,
}

#[derive(Serialize)]
pub(crate) struct QueueIndexRequest {
    pub index: usize,
}

#[derive(Serialize)]
pub(crate) struct QueueTrackRequest {
    pub track: QueueTrack,
}

#[derive(Serialize)]
pub(crate) struct QueueShuffleRequest {
    pub enabled: bool,
}

#[derive(Serialize)]
pub(crate) struct QueueRepeatRequest {
    pub mode: String,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

impl From<&Track> for QueueTrack {
    fn from(track: &Track) -> Self {
        Self {
            video_id: track.id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            duration_sec: track.duration_ms / 1_000,
            thumbnail: track.thumbnail_url.clone().unwrap_or_default(),
            result_type: String::new(),
            source: String::new(),
        }
    }
}

impl QueueTrack {
    pub(crate) fn into_track(self) -> Track {
        let mut track = Track::new(
            self.video_id,
            self.title,
            self.artist,
            self.duration_sec.saturating_mul(1_000),
        );
        if !self.thumbnail.is_empty() {
            track.thumbnail_url = Some(self.thumbnail);
        }
        track
    }
}

impl From<QueueStart> for QueueStartRequest {
    fn from(request: QueueStart) -> Self {
        Self {
            tracks: request.tracks.iter().map(QueueTrack::from).collect(),
            current_index: request.current_index,
            track: request.track.as_ref().map(QueueTrack::from),
            playlist_id: request.playlist_id,
            playlist_name: request.playlist_name,
            playlist_type: request.playlist_type,
            continuation: request.continuation,
            shuffle: request.shuffle,
            repeat_mode: repeat_mode_name(request.repeat_mode),
            endless_queue: request.endless_queue,
        }
    }
}

impl From<QueueSessionResponse> for QueueSession {
    fn from(response: QueueSessionResponse) -> Self {
        Self {
            id: response.id,
            state: response.state,
            tracks: response
                .tracks
                .into_iter()
                .map(QueueTrack::into_track)
                .collect(),
            current_index: response.current_index,
            playlist_id: response.playlist_id,
            playlist_name: response.playlist_name,
            playlist_type: response.playlist_type,
            continuation: response.continuation,
            shuffle: response.shuffle,
            play_order: response.play_order,
            repeat_mode: parse_repeat_mode(&response.repeat_mode),
            endless_queue: response.endless_queue,
            revision: response.revision,
        }
    }
}

fn repeat_mode_name(mode: QueueRepeatMode) -> String {
    match mode {
        QueueRepeatMode::None => "none",
        QueueRepeatMode::All => "all",
        QueueRepeatMode::One => "one",
    }
    .to_owned()
}

fn parse_repeat_mode(mode: &str) -> QueueRepeatMode {
    match mode {
        "all" => QueueRepeatMode::All,
        "one" => QueueRepeatMode::One,
        _ => QueueRepeatMode::None,
    }
}
