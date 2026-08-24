use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use zerobeat_api::{ApiCatalog, ApiConfig};
use zerobeat_catalog::{
    AudioQuality, MusicCatalog, MusicQueue, QueueRepeatMode, QueueStart, SearchRequest,
};
use zerobeat_core::Track;

#[tokio::test]
async fn provisions_without_static_secret_then_sends_signed_search() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (base_url, server) = mock_server(Arc::clone(&requests)).await;
    let directory = tempdir().unwrap();
    let config = ApiConfig::new(
        base_url,
        "/music",
        directory.path().join("device.identity"),
        "cli/0.1.0+1",
    )
    .unwrap();

    let client = ApiCatalog::new(config);
    assert!(requests.lock().unwrap().is_empty());
    let tracks = client
        .search_songs(SearchRequest::new("tampar", 20).unwrap())
        .await
        .unwrap();
    let stream = client
        .resolve_stream("video-123", AudioQuality::Automatic)
        .await
        .unwrap();
    let lyrics = client.lyrics(&tracks[0]).await.unwrap().unwrap();
    server.await.unwrap();

    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id, "video-123");
    assert_eq!(tracks[0].title, "Tampar");
    assert_eq!(tracks[0].artist, "Juicy Luicy");

    let requests = requests.lock().unwrap();
    assert_eq!(stream.headers.len(), 2);
    assert_eq!(stream.expires_at_epoch_seconds, Some(1_735_689));
    assert!(stream.url.contains("range=0-999"));
    assert!(
        stream
            .headers
            .contains(&("User-Agent".into(), "ZeroBeat Test".into()))
    );
    assert!(lyrics.synced);
    assert_eq!(lyrics.lines[1].words, "Masih saja kau ada");
    assert_eq!(requests.len(), 5);
    assert!(requests[0].starts_with("POST /music/v1/device/challenge "));
    assert!(requests[1].starts_with("POST /music/v1/device/provision "));
    assert!(!requests[1].contains("desktopProvisionSecret"));
    assert!(requests[2].starts_with("GET /music/v1/app/search/songs?q=tampar&limit=20 "));
    assert!(requests[2].contains("x-zerobeat-signature-version: v5"));
    assert!(requests[2].contains("x-zerobeat-device-id: device-123"));
    assert!(requests[3].starts_with("GET /music/v1/app/stream/resolve?video_id=video-123 "));
    assert!(requests[4].starts_with(
        "GET /music/v1/lyrics/sources/lookup?title=Tampar&artist=Juicy+Luicy&durationSeconds=245 "
    ));
}

#[tokio::test]
async fn queue_catalog_exposes_authoritative_active_session() {
    let config = ApiConfig::new(
        "http://127.0.0.1:1/music",
        "/music",
        tempdir().unwrap().path().join("device.identity"),
        "cli/0.1.0+1",
    )
    .unwrap();
    let client = ApiCatalog::new(config);
    let _ = client.active_queue().await;
}

#[tokio::test]
async fn queue_requests_use_signed_exact_json_bodies_and_backend_paths() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (base_url, server) = queue_mock_server(Arc::clone(&requests)).await;
    let directory = tempdir().unwrap();
    let config = ApiConfig::new(
        base_url,
        "/music",
        directory.path().join("device.identity"),
        "cli/0.1.0+1",
    )
    .unwrap();
    let client = ApiCatalog::new(config);
    assert_eq!(client.active_queue().await.unwrap(), None);
    let track = Track::new("a", "A", "Artist", 120_000);
    let session = client
        .start_queue(QueueStart {
            tracks: vec![track.clone()],
            endless_queue: true,
            ..QueueStart::default()
        })
        .await
        .unwrap();
    assert_eq!(session.id, "session-1");
    for call in [
        client.get_queue("session-1").await,
        client.next_queue("session-1").await,
        client.previous_queue("session-1").await,
        client.load_more_queue("session-1").await,
        client.play_next_queue("session-1", track.clone()).await,
        client.add_queue("session-1", track.clone()).await,
        client.play_index_queue("session-1", 1).await,
        client.remove_queue("session-1", 1).await,
        client.clear_upcoming_queue("session-1").await,
        client.set_shuffle_queue("session-1", true).await,
        client
            .set_repeat_queue("session-1", QueueRepeatMode::All)
            .await,
    ] {
        call.unwrap();
    }
    client.delete_queue("session-1").await.unwrap();
    server.await.unwrap();

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 16);
    assert!(requests[2].starts_with("GET /music/v1/app/player/queue/session "));
    assert!(requests[2].contains("x-zerobeat-signature-version: v5"));
    assert!(requests[3].starts_with("POST /music/v1/app/player/queue/sessions "));
    assert!(requests[3].ends_with("{\"tracks\":[{\"videoId\":\"a\",\"title\":\"A\",\"artist\":\"Artist\",\"durationSec\":120}],\"currentIndex\":0,\"shuffle\":false,\"repeatMode\":\"none\",\"endlessQueue\":true}"));
    assert!(requests[3].contains("x-zerobeat-body-sha256: "));
    assert!(requests[4].starts_with("GET /music/v1/app/player/queue/sessions/session-1 "));
    assert!(requests[5].starts_with("POST /music/v1/app/player/queue/sessions/session-1/next "));
    assert!(
        requests[6].starts_with("POST /music/v1/app/player/queue/sessions/session-1/previous ")
    );
    assert!(
        requests[7].starts_with("POST /music/v1/app/player/queue/sessions/session-1/load-more ")
    );
    assert!(
        requests[8].starts_with("POST /music/v1/app/player/queue/sessions/session-1/play-next ")
    );
    assert!(requests[9].starts_with("POST /music/v1/app/player/queue/sessions/session-1/add "));
    assert!(
        requests[10].starts_with("POST /music/v1/app/player/queue/sessions/session-1/play-index ")
    );
    assert!(
        requests[11].starts_with("DELETE /music/v1/app/player/queue/sessions/session-1/tracks/1 ")
    );
    assert!(
        requests[12]
            .starts_with("POST /music/v1/app/player/queue/sessions/session-1/clear-upcoming ")
    );
    assert!(requests[13].starts_with("PUT /music/v1/app/player/queue/sessions/session-1/shuffle "));
    assert!(requests[13].ends_with("{\"enabled\":true}"));
    assert!(requests[14].starts_with("PUT /music/v1/app/player/queue/sessions/session-1/repeat "));
    assert!(requests[14].ends_with("{\"mode\":\"all\"}"));
    assert!(requests[15].starts_with("DELETE /music/v1/app/player/queue/sessions/session-1 "));
}

async fn mock_server(requests: Arc<Mutex<Vec<String>>>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let responses = [
            r#"{"challenge":"c2luZ2xlLXVzZS1jaGFsbGVuZ2U","integrityNonce":"nonce","expiresAt":"2026-08-24T02:00:00Z"}"#,
            r#"{"deviceId":"device-123","keyVersion":1,"signatureVersion":"v5","assuranceTier":"LOW"}"#,
            r#"{"items":[{"videoId":"video-123","title":"Tampar","artists":[{"name":"Juicy Luicy"}],"durationSeconds":245,"thumbnails":[{"url":"https://img.example/cover.jpg","width":544,"height":544}]}],"continuation":null,"source":"backend-app-compatible"}"#,
            r#"{"format":{"audioUrl":"https://stream.example/audio.webm?expire=1&range=0-999&sig=ok","expiresAtUnixMs":1735689000,"httpHeaders":{"User-Agent":"ZeroBeat Test","Referer":"https://music.youtube.com/"}}}"#,
            r#"{"found":true,"source":{"videoId":"video-123","lyricsHash":"hash","syncType":"line","lines":[{"startTimeMs":"1000","words":"Entah sudah selasa yang ke berapa"},{"startTimeMs":"5000","words":"Masih saja kau ada"}]}}"#,
        ];
        for response in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            requests.lock().unwrap().push(request);
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response.len(),
                response
            );
            stream.write_all(reply.as_bytes()).await.unwrap();
        }
    });
    (format!("http://{address}/music"), server)
}

async fn queue_mock_server(
    requests: Arc<Mutex<Vec<String>>>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for index in 0..16 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            requests.lock().unwrap().push(request);
            let (status, response) = if index == 2 {
                (404, r#"{"error":"queue session not found"}"#.to_owned())
            } else if index == 0 {
                (200, r#"{"challenge":"Y2hhbGxlbmdl"}"#.to_owned())
            } else if index == 1 {
                (
                    200,
                    r#"{"deviceId":"device-123","keyVersion":1}"#.to_owned(),
                )
            } else if index == 15 {
                (200, r#"{"message":"queue session deleted"}"#.to_owned())
            } else {
                (200, r#"{"id":"session-1","state":"initialized","tracks":[{"videoId":"a","title":"A","artist":"Artist","durationSec":120}],"currentIndex":0,"shuffle":false,"playOrder":[],"repeatMode":"none","endlessQueue":true,"revision":4}"#.to_owned())
            };
            let reason = if status == 200 { "OK" } else { "Not Found" };
            let reply = format!(
                "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
                response.len()
            );
            stream.write_all(reply.as_bytes()).await.unwrap();
        }
    });
    (format!("http://{address}/music"), server)
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end;
    loop {
        let read = stream.read(&mut buffer).await.unwrap();
        request.extend_from_slice(&buffer[..read]);
        if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = position + 4;
            break;
        }
    }
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while request.len() < header_end + content_length {
        let read = stream.read(&mut buffer).await.unwrap();
        request.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8(request).unwrap()
}
