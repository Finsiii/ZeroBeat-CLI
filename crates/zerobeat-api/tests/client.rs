use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use zerobeat_api::{ApiCatalog, ApiConfig};
use zerobeat_catalog::{AudioQuality, MusicCatalog, RadioRequest, SearchRequest};

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
    let radio = client
        .radio_tracks(RadioRequest::from_seed("video-123", 12))
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
    assert_eq!(radio.tracks[0].id, "video-456");
    assert_eq!(radio.continuation, None);

    let requests = requests.lock().unwrap();
    assert_eq!(stream.headers.len(), 2);
    assert!(stream.url.contains("range=0-999"));
    assert!(
        stream
            .headers
            .contains(&("User-Agent".into(), "ZeroBeat Test".into()))
    );
    assert!(lyrics.synced);
    assert_eq!(lyrics.lines[1].words, "Masih saja kau ada");
    assert_eq!(requests.len(), 6);
    assert!(requests[0].starts_with("POST /music/v1/device/challenge "));
    assert!(requests[1].starts_with("POST /music/v1/device/provision "));
    assert!(!requests[1].contains("desktopProvisionSecret"));
    assert!(requests[2].starts_with("GET /music/v1/app/search/songs?q=tampar&limit=20 "));
    assert!(requests[2].contains("x-zerobeat-signature-version: v5"));
    assert!(requests[2].contains("x-zerobeat-device-id: device-123"));
    assert!(requests[3].starts_with("GET /music/v1/app/next?video_id=video-123&limit=12 "));
    assert!(requests[4].starts_with("GET /music/v1/app/stream/resolve?video_id=video-123 "));
    assert!(requests[5].starts_with(
        "GET /music/v1/lyrics/sources/lookup?title=Tampar&artist=Juicy+Luicy&durationSeconds=245 "
    ));
}

async fn mock_server(requests: Arc<Mutex<Vec<String>>>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let responses = [
            r#"{"challenge":"c2luZ2xlLXVzZS1jaGFsbGVuZ2U","integrityNonce":"nonce","expiresAt":"2026-08-24T02:00:00Z"}"#,
            r#"{"deviceId":"device-123","keyVersion":1,"signatureVersion":"v5","assuranceTier":"LOW"}"#,
            r#"{"items":[{"videoId":"video-123","title":"Tampar","artists":[{"name":"Juicy Luicy"}],"durationSeconds":245,"thumbnails":[{"url":"https://img.example/cover.jpg","width":544,"height":544}]}],"continuation":null,"source":"backend-app-compatible"}"#,
            r#"{"title":"Up next","items":[{"videoId":"video-456","title":"Sialan","artists":[{"name":"Juicy Luicy"}],"thumbnails":[]}],"source":"youtubei-native"}"#,
            r#"{"format":{"audioUrl":"https://stream.example/audio.webm?expire=1&range=0-999&sig=ok","httpHeaders":{"User-Agent":"ZeroBeat Test","Referer":"https://music.youtube.com/"}}}"#,
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
