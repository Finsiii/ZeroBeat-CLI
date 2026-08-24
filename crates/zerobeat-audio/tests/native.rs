use std::{
    io::{Read, Write},
    net::TcpListener,
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
        let end = request
            .lines()
            .find_map(|line| line.strip_prefix("range: bytes=0-"))
            .and_then(|end| end.trim().parse::<usize>().ok())
            .unwrap()
            .min(body.len() - 1);
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
