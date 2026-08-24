use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use tokio::sync::Mutex;
use zerobeat_catalog::{AudioQuality, MusicCatalog, ResolvedStream};
use zerobeat_core::Track;
use zerobeat_protocol::AppSnapshot;
use zerobeat_storage::{Database, DownloadState};

use crate::server::library_snapshot;

type SharedStorage = Arc<StdMutex<Database>>;

pub(crate) fn spawn_download(
    track: Track,
    state: Arc<Mutex<AppSnapshot>>,
    catalog: Arc<dyn MusicCatalog>,
    storage: SharedStorage,
    download_directory: Arc<PathBuf>,
) {
    tokio::spawn(async move {
        update_download_state(
            &storage,
            &track,
            DownloadState::Downloading,
            None,
            None,
            &state,
        )
        .await;
        let final_path = download_directory.join(format!("{}.audio", safe_track_id(&track.id)));
        let result = async {
            let stream = catalog
                .resolve_stream(&track.id, AudioQuality::Automatic)
                .await
                .map_err(|error| error.to_string())?;
            download_stream(&stream, &final_path).await
        }
        .await;
        match result {
            Ok(()) => {
                let path = final_path.to_string_lossy().into_owned();
                update_download_state(
                    &storage,
                    &track,
                    DownloadState::Available,
                    Some(path.as_str()),
                    None,
                    &state,
                )
                .await;
            }
            Err(error) => {
                update_download_state(
                    &storage,
                    &track,
                    DownloadState::Failed,
                    None,
                    Some(error.as_str()),
                    &state,
                )
                .await;
            }
        }
    });
}

async fn update_download_state(
    storage: &SharedStorage,
    track: &Track,
    download_state: DownloadState,
    local_path: Option<&str>,
    error: Option<&str>,
    state: &Mutex<AppSnapshot>,
) {
    let library = storage.lock().ok().and_then(|database| {
        database
            .set_download_detail(track, download_state, local_path, error)
            .ok()?;
        library_snapshot(&database).ok()
    });
    if let Some(library) = library {
        state.lock().await.library = library;
    }
}

async fn download_stream(stream: &ResolvedStream, destination: &Path) -> Result<(), String> {
    const CHUNK_SIZE: u64 = 60 * 1024;
    let temporary = destination.with_extension("part");
    let result = async {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| error.to_string())?;
        let mut offset = 0_u64;
        let declared_total = declared_stream_length(&stream.url);
        let mut expected_total = declared_total;
        loop {
            let end = expected_total
                .map(|total| {
                    if declared_total.is_some() {
                        total - 1
                    } else {
                        offset.saturating_add(CHUNK_SIZE - 1).min(total - 1)
                    }
                })
                .unwrap_or_else(|| offset.saturating_add(CHUNK_SIZE - 1));
            let ranged_url = url_with_range(
                &stream.url,
                offset,
                end,
                declared_total.is_some(),
            );
            let mut request = client.get(ranged_url);
            if declared_total.is_none() {
                request = request.header(reqwest::header::RANGE, format!("bytes={offset}-{end}"));
            }
            for (name, value) in &stream.headers {
                if !name.eq_ignore_ascii_case("range") {
                    request = request.header(name, value);
                }
            }
            let mut response = request.send().await.map_err(|error| error.to_string())?;
            let status = response.status();
            let reported_total = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(content_range_total);
            if status == reqwest::StatusCode::PARTIAL_CONTENT {
                if declared_total.is_none() {
                    expected_total = reported_total.or(expected_total);
                }
            } else if expected_total.is_none() && offset == 0 {
                expected_total = reported_total;
            }
            if !(status == reqwest::StatusCode::PARTIAL_CONTENT
                || (status.is_success() && (offset == 0 || expected_total.is_some())))
            {
                return Err(format!(
                    "download server returned HTTP {status} for bytes={offset}-{end}"
                ));
            }
            let requested = end.saturating_sub(offset).saturating_add(1);
            let mut received = 0_u64;
            while let Some(bytes) = response.chunk().await.map_err(|error| error.to_string())? {
                received = received
                    .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
                    .ok_or_else(|| "download size overflow".to_owned())?;
                if received > requested {
                    return Err(format!(
                        "download server ignored bytes={offset}-{end} and returned more than {requested} bytes"
                    ));
                }
                file.write_all(&bytes).map_err(|error| error.to_string())?;
            }
            if received == 0 {
                return Err("download server returned an empty chunk".to_owned());
            }
            offset = offset.saturating_add(received);
            if let Some(total) = expected_total {
                if offset == total {
                    break;
                }
                if offset > total || received != requested {
                    return Err(format!("download ended early at {offset} of {total} bytes"));
                }
            } else if status != reqwest::StatusCode::PARTIAL_CONTENT || received < CHUNK_SIZE {
                break;
            }
        }
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        if let Some(total) = expected_total {
            let actual = std::fs::metadata(&temporary)
                .map_err(|error| error.to_string())?
                .len();
            if actual != total {
                return Err(format!(
                    "download size mismatch: expected {total} bytes, received {actual}"
                ));
            }
        }
        std::fs::rename(&temporary, destination).map_err(|error| error.to_string())?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.parse().ok()
}

fn declared_stream_length(value: &str) -> Option<u64> {
    url::Url::parse(value)
        .ok()?
        .query_pairs()
        .find_map(|(name, value)| (name == "clen").then(|| value.parse().ok()).flatten())
        .filter(|length| *length > 0)
}

fn url_with_range(value: &str, start: u64, end: u64, use_query_range: bool) -> String {
    if !use_query_range {
        return value.to_owned();
    }
    let range = format!("{start}-{end}");
    let fragment = value.find('#').unwrap_or(value.len());
    let Some(query) = value[..fragment].find('?') else {
        return format!("{}?range={range}{}", &value[..fragment], &value[fragment..]);
    };
    let mut offset = query + 1;
    while offset < fragment {
        let next = value[offset..fragment]
            .find('&')
            .map(|position| offset + position)
            .unwrap_or(fragment);
        let name_end = value[offset..next]
            .find('=')
            .map(|position| offset + position)
            .unwrap_or(next);
        if &value[offset..name_end] == "range" {
            return format!("{}range={range}{}", &value[..offset], &value[next..]);
        }
        offset = next.saturating_add(1);
    }
    format!("{}&range={range}{}", &value[..fragment], &value[fragment..])
}

fn safe_track_id(track_id: &str) -> String {
    let value: String = track_id
        .chars()
        .take(96)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if value.is_empty() {
        "track".to_owned()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::url_with_range;

    #[test]
    fn generic_urls_are_not_mutated() {
        assert_eq!(
            url_with_range("https://cdn.example/audio?signature=abc", 0, 99, false),
            "https://cdn.example/audio?signature=abc"
        );
    }

    #[test]
    fn existing_query_range_is_replaced_once() {
        assert_eq!(
            url_with_range(
                "https://cdn.example/audio?clen=100&range=0-9&signature=abc",
                10,
                99,
                true,
            ),
            "https://cdn.example/audio?clen=100&range=10-99&signature=abc"
        );
    }
}
