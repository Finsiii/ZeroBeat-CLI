#![cfg(any(target_os = "linux", target_os = "windows"))]

use std::{
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use tempfile::tempdir;
use zerobeat_audio::{AudioBackend, NativeEngine, NativeState, SPECTRUM_BAND_COUNT, StreamSource};

#[test]
fn native_engine_prebuffers_local_audio_without_opening_an_output_device() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("silence.wav");
    std::fs::write(&path, silent_wav(1_000)).unwrap();
    let mut engine = NativeEngine::new().unwrap();

    engine
        .load(&StreamSource::new(path.to_string_lossy()))
        .unwrap();

    assert_eq!(engine.state(), NativeState::Ready);
    assert!((990..=1_010).contains(&engine.duration_ms()));
    assert!(engine.buffered_ms() > 0);
    assert_eq!(engine.position_ms(), 0);
    engine.stop().unwrap();
    assert_eq!(engine.state(), NativeState::Idle);
}

#[test]
fn native_engine_sends_stream_specific_http_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let read = connection.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..read]).to_ascii_lowercase();
        assert!(request.starts_with("get /audio.wav http/1.1\r\n"));
        assert!(request.contains("user-agent: zerobeat-native-test"));
        assert!(request.contains("referer: https://music.youtube.com/"));
        let body = silent_wav(1_000);
        let bounded_range = request
            .lines()
            .find_map(|line| line.strip_prefix("range: bytes=0-"))
            .is_some_and(|end| !end.trim().is_empty());
        if !bounded_range {
            write!(connection, "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            return false;
        }
        let requested_end = request
            .lines()
            .find_map(|line| line.strip_prefix("range: bytes=0-"))
            .and_then(|end| end.trim().parse::<usize>().ok())
            .unwrap();
        assert!(
            requested_end + 1 >= 512 * 1024,
            "HTTP range is too small: {} bytes",
            requested_end + 1
        );
        let end = requested_end.min(body.len() - 1);
        let chunk = &body[..=end];
        write!(
            connection,
            "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nConnection: close\r\n\r\n",
            chunk.len(),
            end,
            body.len(),
        )
        .unwrap();
        connection.write_all(chunk).unwrap();
        true
    });
    let mut engine = NativeEngine::new().unwrap();
    let source = StreamSource::new(format!("http://{address}/audio.wav"))
        .with_header("User-Agent", "ZeroBeat-Native-Test")
        .with_header("Referer", "https://music.youtube.com/");

    engine.load(&source).unwrap();

    assert_eq!(engine.state(), NativeState::Ready);
    assert!(
        server.join().unwrap(),
        "engine must use a bounded byte range"
    );
}

#[test]
fn native_engine_allows_slow_initial_http_probe_when_the_request_is_progressing() {
    let body = silent_wav(1_000);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let _ = connection.read(&mut request).unwrap();
        let end = body.len() - 1;
        write!(
            connection,
            "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nConnection: close\r\n\r\n",
            body.len(),
            end,
            body.len()
        )
        .unwrap();
        for chunk in body.chunks(65_536) {
            if connection.write_all(chunk).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(800));
        }
    });

    let mut engine = NativeEngine::new().unwrap();
    engine
        .load(&StreamSource::new(format!("http://{address}/audio")))
        .unwrap();
    engine.stop().unwrap();
    server.join().unwrap();
}

#[test]
fn native_engine_tolerates_a_multi_second_upstream_pause_before_audio_resumes() {
    let body = silent_wav(2_000);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let _ = connection.read(&mut request).unwrap();
        let end = body.len() - 1;
        write!(
            connection,
            "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nConnection: close\r\n\r\n",
            body.len(),
            end,
            body.len()
        )
        .unwrap();
        connection.write_all(&body[..44]).unwrap();
        std::thread::sleep(Duration::from_millis(3_200));
        let _ = connection.write_all(&body[44..]);
    });

    let mut engine = NativeEngine::new().unwrap();
    engine
        .load(&StreamSource::new(format!("http://{address}/audio")))
        .unwrap();
    engine.stop().unwrap();
    server.join().unwrap();
}

#[test]
fn native_engine_rejects_a_server_that_ignores_the_http_range_cap() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let read = connection.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..read]).to_ascii_lowercase();
        assert!(request.contains("range: bytes=0-524287"));
        let body = vec![0_u8; 768 * 1024];
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = connection.write_all(header.as_bytes());
        let _ = connection.write_all(&body);
    });

    let mut engine = NativeEngine::new().unwrap();
    let started = Instant::now();
    let result = engine.load(&StreamSource::new(format!("http://{address}/audio")));

    assert!(result.is_err(), "range-ignoring server must be rejected");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "range cap was not enforced promptly: {:?}",
        started.elapsed()
    );
    server.join().unwrap();
}

#[test]
fn native_engine_starts_from_valid_prebuffer_when_a_later_range_fails() {
    let body = silent_wav(5_000);
    let body_len = body.len();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let failure_requested = Arc::new(AtomicBool::new(false));
    let release_failure = Arc::new(AtomicBool::new(false));
    let failure_sent = Arc::new(AtomicBool::new(false));
    let server_failure_requested = Arc::clone(&failure_requested);
    let server_release_failure = Arc::clone(&release_failure);
    let server_failure_sent = Arc::clone(&failure_sent);
    let server = std::thread::spawn(move || {
        let mut failure_released = false;
        for _ in 0..8 {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let read = connection.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            let range = request
                .split("range=")
                .nth(1)
                .and_then(|value| value.split(['&', ' ', '\r', '\n']).next())
                .and_then(|value| value.split('-').next())
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if range == 0 {
                let end = (512 * 1024).min(body.len()) - 1;
                write!(
                    connection,
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nConnection: close\r\n\r\n",
                    end + 1,
                    end,
                    body_len,
                )
                .unwrap();
                connection.write_all(&body[..=end]).unwrap();
                continue;
            }

            server_failure_requested.store(true, Ordering::Release);
            if !failure_released {
                while !server_release_failure.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                failure_released = true;
            }
            write!(
                connection,
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            server_failure_sent.store(true, Ordering::Release);
        }
    });

    let mut engine = NativeEngine::new().unwrap();
    let source = StreamSource::new(format!("http://{address}/audio?clen={}", body_len));
    engine.load(&source).unwrap();
    engine.set_volume(0.0).unwrap();
    engine.play().unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    while !failure_requested.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "decoder never requested a later range"
        );
        std::thread::yield_now();
    }
    release_failure.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !failure_sent.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "fixture never sent the transient failure"
        );
        std::thread::yield_now();
    }
    let deadline = Instant::now() + Duration::from_secs(8);
    while engine.last_error().is_none() {
        assert!(
            Instant::now() < deadline,
            "decoder did not report the range failure"
        );
        std::thread::yield_now();
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(engine.state(), NativeState::Playing);
    assert!(!engine.failed(), "buffered PCM must remain playable");
    let error = engine.last_error().unwrap();
    assert!(error.contains("HTTP status 503"));

    let deadline = Instant::now() + Duration::from_secs(6);
    while !engine.failed() {
        assert!(
            Instant::now() < deadline,
            "failed playback did not become terminal after the buffer drained"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(engine.state(), NativeState::Error);
    engine.stop().unwrap();
    server.join().unwrap();
}

#[test]
fn native_engine_retries_a_transient_later_range_before_playback() {
    let body = silent_wav(5_000);
    let body_len = body.len();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let failure_requested = Arc::new(AtomicBool::new(false));
    let release_failure = Arc::new(AtomicBool::new(false));
    let failure_sent = Arc::new(AtomicBool::new(false));
    let retry_requested = Arc::new(AtomicBool::new(false));
    let release_retry = Arc::new(AtomicBool::new(false));
    let retry_response_done = Arc::new(AtomicBool::new(false));
    let retry_response_ok = Arc::new(AtomicBool::new(false));
    let server_failure_requested = Arc::clone(&failure_requested);
    let server_release_failure = Arc::clone(&release_failure);
    let server_failure_sent = Arc::clone(&failure_sent);
    let server_retry_requested = Arc::clone(&retry_requested);
    let server_release_retry = Arc::clone(&release_retry);
    let server_retry_response_done = Arc::clone(&retry_response_done);
    let server_retry_response_ok = Arc::clone(&retry_response_ok);
    let server = std::thread::spawn(move || {
        let mut transient_sent = false;
        for _ in 0..8 {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let read = connection.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            let range = request
                .split("range=")
                .nth(1)
                .and_then(|value| value.split(['&', ' ', '\r', '\n']).next())
                .and_then(|value| value.split('-').next())
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if range == 0 {
                let end = (512 * 1024).min(body_len) - 1;
                write!(
                    connection,
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nConnection: close\r\n\r\n",
                    end + 1,
                    end,
                    body_len,
                )
                .unwrap();
                connection.write_all(&body[..=end]).unwrap();
                continue;
            }
            if !transient_sent {
                transient_sent = true;
                server_failure_requested.store(true, Ordering::Release);
                while !server_release_failure.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                write!(
                    connection,
                    "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                server_failure_sent.store(true, Ordering::Release);
                continue;
            }

            server_retry_requested.store(true, Ordering::Release);
            while !server_release_retry.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            let end = (range + 512 * 1024).min(body_len) - 1;
            let response_ok = write!(
                connection,
                "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                end + 1 - range,
                range,
                end,
                body_len,
            )
            .and_then(|()| connection.write_all(&body[range..=end]))
            .is_ok();
            server_retry_response_ok.store(response_ok, Ordering::Release);
            server_retry_response_done.store(true, Ordering::Release);
            return;
        }
    });

    let mut engine = NativeEngine::new().unwrap();
    let source = StreamSource::new(format!("http://{address}/audio?clen={body_len}"));
    engine.load(&source).unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    while !failure_requested.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "decoder never requested the failed range"
        );
        std::thread::yield_now();
    }
    release_failure.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !failure_sent.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "fixture never sent the transient failure"
        );
        std::thread::yield_now();
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while !retry_requested.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "decoder did not retry the failed range"
        );
        std::thread::yield_now();
    }

    engine.set_volume(0.0).unwrap();
    engine.play().unwrap();
    release_retry.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !retry_response_done.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "fixture did not complete the retry response"
        );
        std::thread::yield_now();
    }
    assert!(
        retry_response_ok.load(Ordering::Acquire),
        "fixture could not send the retry response"
    );
    engine.stop().unwrap();
    server.join().unwrap();
}

#[test]
fn native_engine_retries_a_transient_response_even_when_its_body_exceeds_the_range_cap() {
    let body = silent_wav(1_000);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_attempts = Arc::clone(&attempts);
    let server = std::thread::spawn(move || {
        for attempt in 0..4 {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = connection.read(&mut request).unwrap();
            server_attempts.fetch_add(1, Ordering::Release);
            if attempt < 3 {
                let oversized = vec![b'x'; 600 * 1024];
                write!(
                    connection,
                    "HTTP/1.1 503 Service Unavailable\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    oversized.len()
                )
                .unwrap();
                let _ = connection.write_all(&oversized);
                continue;
            }
            let end = body.len() - 1;
            write!(
                connection,
                "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nContent-Range: bytes 0-{}/{}\r\nConnection: close\r\n\r\n",
                body.len(),
                end,
                body.len()
            )
            .unwrap();
            connection.write_all(&body).unwrap();
        }
    });

    let mut engine = NativeEngine::new().unwrap();
    engine
        .load(&StreamSource::new(format!("http://{address}/audio")))
        .unwrap();

    assert_eq!(attempts.load(Ordering::Acquire), 4);
    engine.stop().unwrap();
    server.join().unwrap();
}

#[test]
fn native_engine_rejects_a_mismatched_content_range_offset() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let _ = connection.read(&mut request).unwrap();
        let body = silent_wav(1_000);
        write!(
            connection,
            "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nContent-Range: bytes 7-{}/{}\r\nConnection: close\r\n\r\n",
            body.len(),
            body.len() + 6,
            body.len() + 7
        )
        .unwrap();
        connection.write_all(&body).unwrap();
    });

    let mut engine = NativeEngine::new().unwrap();
    let result = engine.load(&StreamSource::new(format!("http://{address}/audio")));

    assert!(result.is_err(), "incorrect Content-Range offset must fail");
    server.join().unwrap();
}

#[test]
fn native_engine_rejects_an_invalid_http_content_range_end() {
    let body = silent_wav(1_000);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let _ = connection.read(&mut request).unwrap();
        write!(
            connection,
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 0-9223372036854775807/9223372036854775807\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        connection.write_all(&body).unwrap();
    });

    let mut engine = NativeEngine::new().unwrap();
    let result = engine.load(&StreamSource::new(format!("http://{address}/audio")));

    assert!(result.is_err(), "invalid Content-Range must be rejected");
    server.join().unwrap();
}

#[test]
fn native_engine_rejects_a_nonzero_200_range_without_content_range() {
    let body = silent_wav(10_000);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let saw_nonzero = Arc::new(AtomicBool::new(false));
    let server_saw_nonzero = Arc::clone(&saw_nonzero);
    let reject_nonzero = Arc::new(AtomicBool::new(false));
    let server_reject_nonzero = Arc::clone(&reject_nonzero);
    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let accept_deadline = Instant::now() + Duration::from_secs(3);
        'connections: for _ in 0..16 {
            let (connection, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        if Instant::now() >= accept_deadline {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("failed to accept range request: {error}"),
                }
            };
            connection
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut connection = BufReader::new(connection);
            let mut request = String::new();
            loop {
                let mut line = String::new();
                let read = match connection.read_line(&mut line) {
                    Ok(read) if read > 0 => read,
                    Ok(_) => continue 'connections,
                    Err(error)
                        if error.kind() == ErrorKind::TimedOut
                            || error.kind() == ErrorKind::WouldBlock
                            || error.kind() == ErrorKind::ConnectionReset =>
                    {
                        continue 'connections;
                    }
                    Err(error) => panic!("failed to read range request: {error}"),
                };
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
                debug_assert!(read > 0);
            }
            let request = request.to_ascii_lowercase();
            let range = request
                .lines()
                .find_map(|line| line.strip_prefix("range: bytes="))
                .and_then(|value| value.split('-').next())
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if range > 0 {
                server_saw_nonzero.store(true, Ordering::Release);
            }
            let reject = range > 0 && server_reject_nonzero.load(Ordering::Acquire);
            let start = if reject { 0 } else { range };
            let end = (start + 512 * 1024).min(body.len());
            if end <= start {
                continue;
            }
            let chunk = &body[start..end];
            let response = if reject {
                write!(
                    connection.get_mut(),
                    "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    chunk.len()
                )
            } else {
                write!(
                    connection.get_mut(),
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                    chunk.len(),
                    start,
                    end - 1,
                    body.len()
                )
            };
            if response.is_err() {
                continue;
            }
            if connection.get_mut().write_all(chunk).is_err() {
                continue;
            }
            if reject {
                break;
            }
        }
    });

    let mut engine = NativeEngine::new().unwrap();
    let source = StreamSource::new(format!("http://{address}/audio"));
    engine.load(&source).unwrap();
    reject_nonzero.store(true, Ordering::Release);

    let seek_result = engine.seek(9_000);
    let mut play_result = None;
    if seek_result.is_ok() {
        engine.set_volume(0.0).unwrap();
        play_result = Some(engine.play());
        for _ in 0..200 {
            if engine.state() == NativeState::Error || engine.last_error().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert!(
        seek_result.is_err()
            || play_result.is_some_and(|result| result.is_err())
            || engine.state() == NativeState::Error
            || engine.last_error().is_some()
    );
    assert!(saw_nonzero.load(Ordering::Acquire));
    server.join().unwrap();
}

#[test]
fn native_engine_cancels_a_stalled_http_prebuffer_before_replacing_it() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicBool::new(false));
    let server_accepted = Arc::clone(&accepted);
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        server_accepted.store(true, Ordering::Release);
        let mut request = [0_u8; 4096];
        let _ = connection.read(&mut request).unwrap();
        let mut drain = [0_u8; 1];
        while connection.read(&mut drain).unwrap_or(0) > 0 {}
    });

    let directory = tempdir().unwrap();
    let path = directory.path().join("silence.wav");
    std::fs::write(&path, silent_wav(1_000)).unwrap();
    let mut engine = NativeEngine::new().unwrap();
    let source = StreamSource::new(format!("http://{address}/audio.wav"));
    let started = Instant::now();
    assert!(engine.load(&source).is_err());
    assert!(accepted.load(Ordering::Acquire));
    engine
        .load(&StreamSource::new(path.to_string_lossy()))
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "stalled HTTP worker was not cancelled promptly: {:?}",
        started.elapsed()
    );
    server.join().unwrap();
}

#[test]
fn native_engine_internal_stop_aborts_a_trickling_http_prebuffer() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4096];
        let _ = connection.read(&mut request).unwrap();
        connection
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: 10000000\r\nConnection: keep-alive\r\n\r\n",
            )
            .unwrap();
        let header = silent_wav(60_000);
        connection.write_all(&header[..44]).unwrap();
        for _ in 0..200 {
            if connection.write_all(&[0]).is_err() {
                break;
            }
            connection.flush().unwrap();
            std::thread::sleep(Duration::from_millis(100));
        }
    });

    let mut engine = NativeEngine::new().unwrap();
    let started = Instant::now();
    let result = engine.load(&StreamSource::new(format!("http://{address}/audio.wav")));
    assert!(result.is_err());
    assert!(
        started.elapsed() < Duration::from_secs(14),
        "internal decode stop did not abort trickling HTTP input promptly: {:?}",
        started.elapsed()
    );
    server.join().unwrap();
}

#[test]
fn native_engine_accepts_query_range_200_responses_across_multiple_chunks() {
    let body = silent_wav(10_000);
    let body_len = body.len();
    assert!(body_len > 2 * 512 * 1024);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let later_range_requested = Arc::new(AtomicBool::new(false));
    let server_later_range_requested = Arc::clone(&later_range_requested);
    let server = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut served = 0;
        let mut saw_later_range = false;
        while served < 8 && Instant::now() < deadline {
            let Ok((mut connection, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            served += 1;
            let mut request = [0_u8; 8192];
            let Ok(read) = connection.read(&mut request) else {
                continue;
            };
            let request = String::from_utf8_lossy(&request[..read]);
            let start = request
                .split("range=")
                .nth(1)
                .and_then(|value| value.split(['&', ' ', '\r', '\n']).next())
                .and_then(|value| value.split('-').next())
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if start > 0 {
                server_later_range_requested.store(true, Ordering::Release);
                saw_later_range = true;
            }
            let end = (start + 512 * 1024).min(body_len);
            if end <= start {
                continue;
            }
            write!(
                connection,
                "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                end - start
            )
            .unwrap();
            let _ = connection.write_all(&body[start..end]);
            if saw_later_range {
                break;
            }
        }
    });

    let load_succeeded = Arc::new(AtomicBool::new(false));
    let load_done = Arc::new(AtomicBool::new(false));
    let loader_succeeded = Arc::clone(&load_succeeded);
    let loader_done = Arc::clone(&load_done);
    let mut engine = NativeEngine::new().unwrap();
    let cancellation = engine.cancellation_handle().unwrap();
    let loader = std::thread::spawn(move || {
        let source = StreamSource::new(format!("http://{address}/audio?clen={body_len}"));
        if engine.load(&source).is_ok() && engine.set_volume(0.0).is_ok() && engine.play().is_ok() {
            loader_succeeded.store(true, Ordering::Release);
            std::thread::sleep(Duration::from_secs(3));
        }
        loader_done.store(true, Ordering::Release);
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    while !load_done.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::yield_now();
    }
    cancellation.cancel();
    loader.join().unwrap();
    assert!(
        load_done.load(Ordering::Acquire),
        "query-range load did not settle"
    );
    assert!(
        load_succeeded.load(Ordering::Acquire),
        "query-range load failed"
    );
    assert!(later_range_requested.load(Ordering::Acquire));
    server.join().unwrap();
}

#[test]
fn native_engine_cancellation_handle_aborts_a_load_from_another_thread() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicBool::new(false));
    let server_accepted = Arc::clone(&accepted);
    let server = std::thread::spawn(move || {
        let (mut connection, _) = listener.accept().unwrap();
        server_accepted.store(true, Ordering::Release);
        let mut request = [0_u8; 4096];
        let _ = connection.read(&mut request).unwrap();
        let mut drain = [0_u8; 1];
        while connection.read(&mut drain).unwrap_or(0) > 0 {}
    });

    let mut engine = NativeEngine::new().unwrap();
    let cancellation = engine.cancellation_handle().unwrap();
    let trigger = cancellation.clone();
    let source = StreamSource::new(format!("http://{address}/audio"));
    let started = Instant::now();
    let loader = std::thread::spawn(move || engine.load(&source));
    while !accepted.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    trigger.cancel();
    drop(cancellation);

    assert!(loader.join().unwrap().is_err());
    trigger.cancel();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancellation did not release load promptly: {:?}",
        started.elapsed()
    );
    server.join().unwrap();
}

#[test]
fn real_spectrum_places_a_440_hz_tone_in_the_expected_frequency_band() {
    let samples = stereo_tone(440.0);

    let spectrum = NativeEngine::analyze_spectrum(&samples, 2).unwrap();
    let peak = spectrum
        .iter()
        .enumerate()
        .max_by_key(|(_, level)| *level)
        .map(|(index, _)| index)
        .unwrap();

    assert_eq!(spectrum.len(), SPECTRUM_BAND_COUNT);
    assert!(
        (7..=9).contains(&peak),
        "unexpected peak {peak}: {spectrum:?}"
    );
    assert!(spectrum[peak] >= 70, "weak peak: {spectrum:?}");
}

#[test]
fn real_spectrum_tracks_bass_mid_and_treble_without_false_silence_energy() {
    for (frequency, expected_band) in [(100.0, 2_usize), (3_000.0, 16), (10_000.0, 21)] {
        let spectrum = NativeEngine::analyze_spectrum(&stereo_tone(frequency), 2).unwrap();
        let peak = spectrum
            .iter()
            .enumerate()
            .max_by_key(|(_, level)| *level)
            .map(|(index, _)| index)
            .unwrap();
        assert!(
            peak.abs_diff(expected_band) <= 1,
            "{frequency} Hz peaked in band {peak}: {spectrum:?}"
        );
        assert!(spectrum[peak] >= 70);
    }

    let silence = NativeEngine::analyze_spectrum(&vec![0.0; 8_192], 2).unwrap();
    assert_eq!(silence, [0; SPECTRUM_BAND_COUNT]);
}

fn stereo_tone(frequency: f32) -> Vec<f32> {
    const SAMPLE_RATE: usize = 48_000;
    (0..4_096)
        .flat_map(|frame| {
            let phase = std::f32::consts::TAU * frequency * frame as f32 / SAMPLE_RATE as f32;
            let sample = phase.sin() * 0.8;
            [sample, sample]
        })
        .collect()
}

fn silent_wav(duration_ms: u32) -> Vec<u8> {
    const SAMPLE_RATE: u32 = 48_000;
    const CHANNELS: u16 = 2;
    const BITS_PER_SAMPLE: u16 = 16;
    let frames = SAMPLE_RATE * duration_ms / 1_000;
    let data_size = frames * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE / 8);
    let mut bytes = Vec::with_capacity(44 + data_size as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&CHANNELS.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    let byte_rate = SAMPLE_RATE * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE / 8);
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&(CHANNELS * (BITS_PER_SAMPLE / 8)).to_le_bytes());
    bytes.extend_from_slice(&BITS_PER_SAMPLE.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    bytes.resize(44 + data_size as usize, 0);
    bytes
}
