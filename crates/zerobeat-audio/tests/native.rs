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
fn native_engine_rejects_a_nonzero_200_range_without_content_range() {
    let body = silent_wav(10_000);
    let body_length = body.len();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let saw_nonzero = Arc::new(AtomicBool::new(false));
    let server_saw_nonzero = Arc::clone(&saw_nonzero);
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
            let range = request
                .split("range=")
                .nth(1)
                .and_then(|value| value.split(['&', ' ']).next())
                .and_then(|value| value.split('-').next())
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if range > 0 {
                server_saw_nonzero.store(true, Ordering::Release);
            }
            let chunk = &body[..512 * 1024];
            if write!(
                connection.get_mut(),
                "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                chunk.len()
            )
            .is_err()
            {
                continue;
            }
            if connection.get_mut().write_all(chunk).is_err() {
                continue;
            }
            if range > 0 {
                break;
            }
        }
    });

    let mut engine = NativeEngine::new().unwrap();
    let source = StreamSource::new(format!("http://{address}/audio?clen={}", body_length));
    engine.load(&source).unwrap();

    let seek_result = engine.seek(9_000);
    if seek_result.is_ok() {
        for _ in 0..100 {
            if engine.state() == NativeState::Error {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    assert!(seek_result.is_err() || engine.state() == NativeState::Error);
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
