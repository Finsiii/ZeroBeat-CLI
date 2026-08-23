use zerobeat_core::{NavigationState, Route, SessionMode};
use zerobeat_protocol::{
    AppSnapshot, ClientCommand, DaemonEvent, PROTOCOL_VERSION, decode, encode,
};

#[test]
fn command_round_trips_without_losing_payload() {
    let commands = [
        ClientCommand::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
        ClientCommand::Navigate(Route::Library),
        ClientCommand::UpdateSearch("tampar".into()),
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
    let event = DaemonEvent::Snapshot(AppSnapshot {
        session: SessionMode::Guest,
        navigation,
    });

    let bytes = encode(&event).expect("encode event");
    let decoded: DaemonEvent = decode(&bytes).expect("decode event");

    assert_eq!(decoded, event);
}

#[test]
fn malformed_payload_is_rejected() {
    let result = decode::<ClientCommand>(&[0xc1]);

    assert!(result.is_err());
}
