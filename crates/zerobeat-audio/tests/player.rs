use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use zerobeat_audio::{
    AudioBackend, BackendError, BackendTelemetry, CrossfadeCurve, DualDeck, Player, PlayerState,
    QueueItem, StreamSource,
};
use zerobeat_core::Track;

#[test]
fn equal_power_crossfade_has_stable_endpoints_and_center() {
    let (outgoing, incoming) = CrossfadeCurve::gains(0.0);
    assert_close(outgoing, 1.0);
    assert_close(incoming, 0.0);

    let (outgoing, incoming) = CrossfadeCurve::gains(0.5);
    assert_close(outgoing, std::f32::consts::FRAC_1_SQRT_2);
    assert_close(incoming, std::f32::consts::FRAC_1_SQRT_2);

    let (outgoing, incoming) = CrossfadeCurve::gains(1.0);
    assert_close(outgoing, 0.0);
    assert_close(incoming, 1.0);
}

#[test]
fn queue_moves_through_buffering_playing_and_next_track() {
    let mut player = Player::new(RecordingBackend::default());
    player.enqueue(item("first"));
    player.enqueue(item("second"));

    player.play().unwrap();
    assert_eq!(player.state(), PlayerState::Buffering);
    assert_eq!(player.current().unwrap().track.id, "first");

    player.mark_ready().unwrap();
    assert_eq!(player.state(), PlayerState::Playing);

    player.skip_to_next().unwrap();
    assert_eq!(player.state(), PlayerState::Buffering);
    assert_eq!(player.current().unwrap().track.id, "second");
    assert_eq!(
        player.backend().events,
        ["load:first", "play", "stop", "load:second"]
    );
}

#[test]
fn pause_and_resume_do_not_reload_the_stream() {
    let mut player = Player::new(RecordingBackend::default());
    player.enqueue(item("first"));
    player.play().unwrap();
    player.mark_ready().unwrap();

    player.pause().unwrap();
    assert_eq!(player.state(), PlayerState::Paused);
    player.play().unwrap();
    assert_eq!(player.state(), PlayerState::Playing);
    assert_eq!(
        player.backend().events,
        ["load:first", "play", "pause", "play"]
    );
}

#[test]
fn dual_deck_crossfade_prebuffers_incoming_before_releasing_outgoing() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let first = SharedRecordingBackend::new("a", Arc::clone(&events));
    let second = SharedRecordingBackend::new("b", Arc::clone(&events));
    let mut mixer = DualDeck::new(first, second);
    mixer.load(&StreamSource::new("first")).unwrap();
    mixer.play().unwrap();

    mixer
        .transition_to(&StreamSource::new("second"), Duration::from_millis(20))
        .unwrap();
    std::thread::sleep(Duration::from_millis(60));

    let events = events.lock().unwrap();
    let incoming_load = event_index(&events, "b:load:second");
    let incoming_play = event_index(&events, "b:play");
    let outgoing_stop = events.iter().rposition(|event| event == "a:stop").unwrap();
    assert!(incoming_load < incoming_play);
    assert!(incoming_play < outgoing_stop);
    assert_eq!(mixer.telemetry().position_ms, 2_000);
}

#[derive(Default)]
struct RecordingBackend {
    events: Vec<String>,
}

struct SharedRecordingBackend {
    name: &'static str,
    events: Arc<Mutex<Vec<String>>>,
}

impl SharedRecordingBackend {
    fn new(name: &'static str, events: Arc<Mutex<Vec<String>>>) -> Self {
        Self { name, events }
    }

    fn record(&self, event: impl Into<String>) {
        self.events
            .lock()
            .unwrap()
            .push(format!("{}:{}", self.name, event.into()));
    }
}

impl AudioBackend for SharedRecordingBackend {
    fn load(&mut self, source: &StreamSource) -> Result<(), BackendError> {
        self.record(format!("load:{}", source.url));
        Ok(())
    }

    fn play(&mut self) -> Result<(), BackendError> {
        self.record("play");
        Ok(())
    }

    fn pause(&mut self) -> Result<(), BackendError> {
        self.record("pause");
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.record("stop");
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), BackendError> {
        self.record(format!("volume:{volume:.3}"));
        Ok(())
    }

    fn telemetry(&self) -> BackendTelemetry {
        BackendTelemetry {
            position_ms: if self.name == "a" { 1_000 } else { 2_000 },
            ..BackendTelemetry::default()
        }
    }
}

impl AudioBackend for RecordingBackend {
    fn load(&mut self, source: &StreamSource) -> Result<(), BackendError> {
        self.events.push(format!("load:{}", source.url));
        Ok(())
    }

    fn play(&mut self) -> Result<(), BackendError> {
        self.events.push("play".into());
        Ok(())
    }

    fn pause(&mut self) -> Result<(), BackendError> {
        self.events.push("pause".into());
        Ok(())
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.events.push("stop".into());
        Ok(())
    }
}

fn item(id: &str) -> QueueItem {
    QueueItem::new(Track::new(id, id, "artist", 180_000), StreamSource::new(id))
}

fn assert_close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
}

fn event_index(events: &[String], expected: &str) -> usize {
    events
        .iter()
        .position(|event| event == expected)
        .unwrap_or_else(|| panic!("missing {expected} in {events:?}"))
}
