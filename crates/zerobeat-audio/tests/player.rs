use zerobeat_audio::{
    AudioBackend, BackendError, CrossfadeCurve, Player, PlayerState, QueueItem, StreamSource,
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

#[derive(Default)]
struct RecordingBackend {
    events: Vec<String>,
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
