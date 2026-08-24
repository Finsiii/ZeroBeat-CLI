use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use crate::{AudioBackend, BackendError, BackendTelemetry, CrossfadeCurve, StreamSource};

pub struct DualDeck<B> {
    inner: Arc<Mutex<DeckState<B>>>,
    generation: Arc<AtomicU64>,
}

struct DeckState<B> {
    decks: [B; 2],
    active: usize,
    incoming: Option<usize>,
    progress: f32,
    volume: f32,
}

impl<B> DualDeck<B> {
    pub fn new(first: B, second: B) -> Self {
        Self {
            inner: Arc::new(Mutex::new(DeckState {
                decks: [first, second],
                active: 0,
                incoming: None,
                progress: 0.0,
                volume: 1.0,
            })),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl<B: AudioBackend + 'static> AudioBackend for DualDeck<B> {
    fn load(&mut self, source: &StreamSource) -> Result<(), BackendError> {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let mut state = lock_decks(&self.inner)?;
        stop_both(&mut state.decks)?;
        state.incoming = None;
        state.progress = 0.0;
        let active = state.active;
        let volume = state.volume;
        state.decks[active].load(source)?;
        state.decks[active].set_volume(volume)
    }

    fn play(&mut self) -> Result<(), BackendError> {
        let mut state = lock_decks(&self.inner)?;
        let active = state.incoming.unwrap_or(state.active);
        state.decks[active].play()
    }

    fn pause(&mut self) -> Result<(), BackendError> {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let mut state = lock_decks(&self.inner)?;
        if let Some(incoming) = state.incoming.take() {
            let outgoing = state.active;
            let volume = state.volume;
            state.decks[outgoing].stop()?;
            state.decks[incoming].set_volume(volume)?;
            state.active = incoming;
        }
        let active = state.active;
        state.decks[active].pause()
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let mut state = lock_decks(&self.inner)?;
        state.incoming = None;
        state.progress = 0.0;
        stop_both(&mut state.decks)
    }

    fn transition_to(
        &mut self,
        source: &StreamSource,
        duration: Duration,
    ) -> Result<(), BackendError> {
        self.transition_to_guarded(source, duration, &|| true)
    }

    fn transition_to_guarded(
        &mut self,
        source: &StreamSource,
        duration: Duration,
        should_continue: &dyn Fn() -> bool,
    ) -> Result<(), BackendError> {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        {
            let mut state = lock_decks(&self.inner)?;
            let incoming = 1 - state.active;
            state.decks[incoming].stop()?;
            if !should_continue() {
                return Ok(());
            }
            state.decks[incoming].load(source)?;
            if !should_continue() {
                return state.decks[incoming].stop();
            }
            state.decks[incoming].set_volume(0.0)?;
            state.decks[incoming].play()?;
            if !should_continue() {
                return state.decks[incoming].stop();
            }
            state.incoming = Some(incoming);
            state.progress = 0.0;
        }

        let inner = Arc::clone(&self.inner);
        let active_generation = Arc::clone(&self.generation);
        thread::spawn(move || {
            let millis = duration.as_millis().max(1);
            let steps = u32::try_from((millis / 25).max(1)).unwrap_or(u32::MAX);
            let interval = duration / steps;
            for step in 1..=steps {
                thread::sleep(interval);
                if active_generation.load(Ordering::Acquire) != generation {
                    return;
                }
                let mut state = match inner.lock() {
                    Ok(state) => state,
                    Err(_) => return,
                };
                let Some(incoming) = state.incoming else {
                    return;
                };
                let outgoing = state.active;
                let progress = step as f32 / steps as f32;
                let (outgoing_gain, incoming_gain) = CrossfadeCurve::gains(progress);
                let volume = state.volume;
                let (outgoing_deck, incoming_deck) =
                    deck_pair(&mut state.decks, outgoing, incoming);
                if outgoing_deck.set_volume(volume * outgoing_gain).is_err()
                    || incoming_deck.set_volume(volume * incoming_gain).is_err()
                {
                    let _ = incoming_deck.stop();
                    let _ = outgoing_deck.set_volume(volume);
                    state.incoming = None;
                    state.progress = 0.0;
                    return;
                }
                state.progress = progress;
            }

            if let Ok(mut state) = inner.lock()
                && let Some(incoming) = state.incoming.take()
            {
                let outgoing = state.active;
                let volume = state.volume;
                let _ = state.decks[outgoing].stop();
                let _ = state.decks[incoming].set_volume(volume);
                state.active = incoming;
                state.progress = 0.0;
            }
        });
        Ok(())
    }

    fn seek(&mut self, position_ms: u64) -> Result<(), BackendError> {
        let mut state = lock_decks(&self.inner)?;
        let target = state.incoming.unwrap_or(state.active);
        state.decks[target].seek(position_ms)
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), BackendError> {
        let mut state = lock_decks(&self.inner)?;
        state.volume = volume.clamp(0.0, 1.0);
        let active = state.active;
        if let Some(incoming) = state.incoming {
            let (outgoing_gain, incoming_gain) = CrossfadeCurve::gains(state.progress);
            let volume = state.volume;
            let (outgoing_deck, incoming_deck) = deck_pair(&mut state.decks, active, incoming);
            outgoing_deck.set_volume(volume * outgoing_gain)?;
            incoming_deck.set_volume(volume * incoming_gain)
        } else {
            let volume = state.volume;
            state.decks[active].set_volume(volume)
        }
    }

    fn telemetry(&self) -> BackendTelemetry {
        let Ok(state) = self.inner.lock() else {
            return BackendTelemetry::default();
        };
        let target = state.incoming.unwrap_or(state.active);
        state.decks[target].telemetry()
    }
}

fn lock_decks<B>(
    inner: &Mutex<DeckState<B>>,
) -> Result<std::sync::MutexGuard<'_, DeckState<B>>, BackendError> {
    inner
        .lock()
        .map_err(|_| BackendError::Failed("dual-deck state lock was poisoned".into()))
}

fn stop_both<B: AudioBackend>(decks: &mut [B; 2]) -> Result<(), BackendError> {
    let first = decks[0].stop();
    let second = decks[1].stop();
    first.and(second)
}

fn deck_pair<B>(decks: &mut [B; 2], outgoing: usize, incoming: usize) -> (&mut B, &mut B) {
    debug_assert_ne!(outgoing, incoming);
    if outgoing == 0 {
        let (left, right) = decks.split_at_mut(1);
        (&mut left[0], &mut right[0])
    } else {
        let (left, right) = decks.split_at_mut(1);
        (&mut right[0], &mut left[0])
    }
}
