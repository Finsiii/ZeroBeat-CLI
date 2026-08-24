use zerobeat_core::{NavigationState, Route, SessionMode, Track};
use zerobeat_protocol::{
    AppSnapshot, ClientCommand, DaemonEvent, PROTOCOL_VERSION, PlaybackSnapshot, PlaybackStatus,
    SearchSnapshot, SearchStatus, decode, encode,
};

#[test]
fn command_round_trips_without_losing_payload() {
    let commands = [
        ClientCommand::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
        ClientCommand::Navigate(Route::Library),
        ClientCommand::UpdateSearch("tampar".into()),
        ClientCommand::SubmitSearch,
        ClientCommand::SelectNext,
        ClientCommand::PlaySelected,
        ClientCommand::PlayTrack(Track::new("video-1", "Tampar", "Juicy Luicy", 245_000)),
        ClientCommand::QueueTrack(Track::new("video-1", "Tampar", "Juicy Luicy", 245_000)),
        ClientCommand::ToggleLike(Track::new("video-1", "Tampar", "Juicy Luicy", 245_000)),
        ClientCommand::DownloadTrack(Track::new("video-1", "Tampar", "Juicy Luicy", 245_000)),
        ClientCommand::ToggleLyrics,
        ClientCommand::SetCrossfadeSeconds(8),
        ClientCommand::TogglePlayback,
        ClientCommand::NextTrack,
        ClientCommand::RequestSnapshot,
        ClientCommand::Shutdown,
    ];

    for command in commands {
        let bytes = encode(&command).expect("encode command");
        let decoded: ClientCommand = decode(&bytes).expect("decode command");
        assert_eq!(decoded, command);
    }
}

#[test]
fn snapshot_event_round_trips_with_guest_navigation_state() {
    let mut navigation = NavigationState::default();
    navigation.open(Route::Search);
    navigation.update_search("juicy luicy");
    let event = DaemonEvent::Snapshot(Box::new(AppSnapshot {
        session: SessionMode::Guest,
        navigation,
        search: SearchSnapshot {
            status: SearchStatus::Ready,
            results: vec![Track::new("video-1", "Tampar", "Juicy Luicy", 245_000)],
            selected_index: 0,
            request_id: 1,
        },
        playback: PlaybackSnapshot {
            status: PlaybackStatus::Playing,
            current: Some(Track::new("video-1", "Tampar", "Juicy Luicy", 245_000)),
            position_ms: 12_000,
            duration_ms: 245_000,
            buffered_ms: 30_000,
            volume_percent: 80,
            error: None,
            request_id: 1,
            queue: vec![Track::new("video-2", "Sialan", "Juicy Luicy", 242_000)],
        },
        library: Default::default(),
        lyrics: Default::default(),
        settings: Default::default(),
    }));

    let bytes = encode(&event).expect("encode event");
    let decoded: DaemonEvent = decode(&bytes).expect("decode event");

    assert_eq!(decoded, event);
}

#[test]
fn malformed_payload_is_rejected() {
    let result = decode::<ClientCommand>(&[0xc1]);

    assert!(result.is_err());
}
