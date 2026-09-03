use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::CancellationController;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::NativeCancellationHandle;
use crate::{AudioBackend, BackendError, BackendTelemetry, CrossfadeCurve, StreamSource};

#[cfg(test)]
use std::sync::{Condvar, OnceLock};

pub struct DualDeck<B> {
    inner: Arc<Mutex<DeckState<B>>>,
    generation: Arc<AtomicU64>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    active_slot: Arc<AtomicUsize>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    incoming_slot: Arc<AtomicUsize>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    cancellation_handles: [Option<NativeCancellationHandle>; 2],
}

struct DeckState<B> {
    decks: [B; 2],
    active: usize,
    incoming: Option<usize>,
    progress: f32,
    volume: f32,
}

#[cfg(test)]
struct TransitionTestHook {
    state: Mutex<(bool, bool)>,
    wake: Condvar,
}

#[cfg(test)]
static TRANSITION_TEST_HOOK: OnceLock<Mutex<Option<Arc<TransitionTestHook>>>> = OnceLock::new();

#[cfg(test)]
fn pause_after_generation_check() {
    let hook = TRANSITION_TEST_HOOK
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .take();
    let Some(hook) = hook else {
        return;
    };
    let mut state = hook.state.lock().unwrap();
    state.0 = true;
    hook.wake.notify_all();
    while !state.1 {
        state = hook.wake.wait(state).unwrap();
    }
}

impl<B: AudioBackend> DualDeck<B> {
    pub fn new(first: B, second: B) -> Self {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let cancellation_handles = [first.cancellation_handle(), second.cancellation_handle()];
        Self {
            inner: Arc::new(Mutex::new(DeckState {
                decks: [first, second],
                active: 0,
                incoming: None,
                progress: 0.0,
                volume: 1.0,
            })),
            generation: Arc::new(AtomicU64::new(0)),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            active_slot: Arc::new(AtomicUsize::new(0)),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            incoming_slot: Arc::new(AtomicUsize::new(usize::MAX)),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            cancellation_handles,
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub fn incoming_cancellation_handle(&self) -> Option<NativeCancellationHandle> {
        let slot = self.incoming_slot.load(Ordering::Acquire);
        (slot < self.cancellation_handles.len())
            .then(|| self.cancellation_handles[slot].clone())
            .flatten()
    }
}

impl<B: AudioBackend + 'static> AudioBackend for DualDeck<B> {
    fn load(&mut self, source: &StreamSource) -> Result<(), BackendError> {
        let mut state = lock_decks(&self.inner)?;
        self.generation.fetch_add(1, Ordering::AcqRel);
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        self.incoming_slot.store(usize::MAX, Ordering::Release);
        stop_both(&mut state.decks)?;
        state.incoming = None;
        state.progress = 0.0;
        let active = state.active;
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        self.active_slot.store(active, Ordering::Release);
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
        let mut state = lock_decks(&self.inner)?;
        self.generation.fetch_add(1, Ordering::AcqRel);
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        self.incoming_slot.store(usize::MAX, Ordering::Release);
        if let Some(incoming) = state.incoming.take() {
            let outgoing = state.active;
            let volume = state.volume;
            state.decks[outgoing].stop()?;
            state.decks[incoming].set_volume(volume)?;
            state.active = incoming;
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            self.active_slot.store(incoming, Ordering::Release);
        }
        let active = state.active;
        state.decks[active].pause()
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        let mut state = lock_decks(&self.inner)?;
        self.generation.fetch_add(1, Ordering::AcqRel);
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        self.incoming_slot.store(usize::MAX, Ordering::Release);
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
        if !should_continue() {
            return Ok(());
        }
        let generation;
        {
            let mut state = lock_decks(&self.inner)?;
            let incoming = 1 - state.active;
            if !should_continue() {
                return Ok(());
            }
            generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
            state.decks[incoming].stop()?;
            if !should_continue() {
                return Ok(());
            }
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            self.incoming_slot.store(incoming, Ordering::Release);
            if let Err(error) = state.decks[incoming].load(source) {
                #[cfg(any(target_os = "linux", target_os = "windows"))]
                self.incoming_slot.store(usize::MAX, Ordering::Release);
                return Err(error);
            }
            if !should_continue() {
                #[cfg(any(target_os = "linux", target_os = "windows"))]
                self.incoming_slot.store(usize::MAX, Ordering::Release);
                return state.decks[incoming].stop();
            }
            if let Err(error) = state.decks[incoming].set_volume(0.0) {
                #[cfg(any(target_os = "linux", target_os = "windows"))]
                self.incoming_slot.store(usize::MAX, Ordering::Release);
                return Err(error);
            }
            if let Err(error) = state.decks[incoming].play() {
                #[cfg(any(target_os = "linux", target_os = "windows"))]
                self.incoming_slot.store(usize::MAX, Ordering::Release);
                return Err(error);
            }
            if !should_continue() {
                #[cfg(any(target_os = "linux", target_os = "windows"))]
                self.incoming_slot.store(usize::MAX, Ordering::Release);
                return state.decks[incoming].stop();
            }
            state.incoming = Some(incoming);
            state.progress = 0.0;
        }

        let inner = Arc::clone(&self.inner);
        let active_generation = Arc::clone(&self.generation);
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let incoming_slot = Arc::clone(&self.incoming_slot);
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let active_slot = Arc::clone(&self.active_slot);
        thread::spawn(move || {
            let millis = duration.as_millis().max(1);
            let steps = u32::try_from((millis / 25).max(1)).unwrap_or(u32::MAX);
            let interval = duration / steps;
            for step in 1..=steps {
                thread::sleep(interval);
                if active_generation.load(Ordering::Acquire) != generation {
                    return;
                }
                #[cfg(test)]
                pause_after_generation_check();
                let mut state = match inner.lock() {
                    Ok(state) => state,
                    Err(_) => return,
                };
                if active_generation.load(Ordering::Acquire) != generation {
                    return;
                }
                let Some(incoming) = state.incoming else {
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    incoming_slot.store(usize::MAX, Ordering::Release);
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
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    incoming_slot.store(usize::MAX, Ordering::Release);
                    return;
                }
                state.progress = progress;
            }

            if let Ok(mut state) = inner.lock()
                && active_generation.load(Ordering::Acquire) == generation
                && let Some(incoming) = state.incoming.take()
            {
                let outgoing = state.active;
                let volume = state.volume;
                let _ = state.decks[outgoing].stop();
                let _ = state.decks[incoming].set_volume(volume);
                state.active = incoming;
                state.progress = 0.0;
                #[cfg(any(target_os = "linux", target_os = "windows"))]
                {
                    active_slot.store(incoming, Ordering::Release);
                    incoming_slot.store(usize::MAX, Ordering::Release);
                }
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

    fn failed(&self) -> bool {
        let Ok(state) = self.inner.lock() else {
            return false;
        };
        let target = state.incoming.unwrap_or(state.active);
        state.decks[target].failed()
    }

    fn last_error(&self) -> Option<String> {
        let Ok(state) = self.inner.lock() else {
            return None;
        };
        let target = state.incoming.unwrap_or(state.active);
        state.decks[target].last_error()
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn cancellation_handle(&self) -> Option<NativeCancellationHandle> {
        self.incoming_cancellation_handle().or_else(|| {
            let slot = self.active_slot.load(Ordering::Acquire);
            (slot < self.cancellation_handles.len())
                .then(|| self.cancellation_handles[slot].clone())
                .flatten()
        })
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn cancel_current_load(&self) {
        if let Some(handle) = self.incoming_cancellation_handle() {
            handle.cancel();
            return;
        }
        let slot = self.active_slot.load(Ordering::Acquire);
        if let Some(handle) = self.cancellation_handles.get(slot).and_then(Clone::clone) {
            handle.cancel();
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn cancellation_controller(&self) -> Option<CancellationController> {
        let incoming_slot = Arc::clone(&self.incoming_slot);
        let active_slot = Arc::clone(&self.active_slot);
        let handles = self.cancellation_handles.clone();
        Some(Arc::new(move || {
            let slot = incoming_slot.load(Ordering::Acquire);
            let slot = if slot < handles.len() {
                slot
            } else {
                active_slot.load(Ordering::Acquire)
            };
            if let Some(handle) = handles.get(slot).and_then(Clone::clone) {
                handle.cancel();
            }
        }))
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    use crate::NativeEngine;

    struct RecordingBackend {
        name: &'static str,
        current: String,
        events: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingBackend {
        fn new(name: &'static str, events: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                name,
                current: String::new(),
                events,
            }
        }

        fn record(&self, event: impl Into<String>) {
            self.events
                .lock()
                .unwrap()
                .push(format!("{}:{}", self.name, event.into()));
        }
    }

    impl AudioBackend for RecordingBackend {
        fn load(&mut self, source: &StreamSource) -> Result<(), BackendError> {
            self.current = source.url.clone();
            self.record(format!("{}:load", self.current));
            Ok(())
        }

        fn play(&mut self) -> Result<(), BackendError> {
            self.record(format!("{}:play", self.current));
            Ok(())
        }

        fn pause(&mut self) -> Result<(), BackendError> {
            self.record(format!("{}:pause", self.current));
            Ok(())
        }

        fn stop(&mut self) -> Result<(), BackendError> {
            self.record(format!("{}:stop", self.current));
            Ok(())
        }

        fn set_volume(&mut self, volume: f32) -> Result<(), BackendError> {
            self.record(format!("{}:volume:{volume:.3}", self.current));
            Ok(())
        }
    }

    #[test]
    fn stale_worker_exits_after_replacement_generation_changes() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let hook = Arc::new(TransitionTestHook {
            state: Mutex::new((false, false)),
            wake: Condvar::new(),
        });
        *TRANSITION_TEST_HOOK
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some(Arc::clone(&hook));

        let first = RecordingBackend::new("a", Arc::clone(&events));
        let second = RecordingBackend::new("b", Arc::clone(&events));
        let mixer = Arc::new(Mutex::new(DualDeck::new(first, second)));
        {
            let mut mixer = mixer.lock().unwrap();
            mixer.load(&StreamSource::new("first")).unwrap();
            mixer.play().unwrap();
            mixer
                .transition_to(&StreamSource::new("second"), Duration::from_secs(10))
                .unwrap();
        }

        {
            let mut state = hook.state.lock().unwrap();
            while !state.0 {
                state = hook.wake.wait(state).unwrap();
            }
        }

        let replacement = Arc::clone(&mixer);
        let replacement_thread = std::thread::spawn(move || {
            replacement
                .lock()
                .unwrap()
                .transition_to(&StreamSource::new("third"), Duration::from_millis(1))
                .unwrap();
        });
        for _ in 0..100 {
            if events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event == "b:third:play")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event == "b:third:play"),
            "replacement did not reach the playing state"
        );
        replacement_thread.join().unwrap();

        {
            let mut state = hook.state.lock().unwrap();
            state.1 = true;
            hook.wake.notify_all();
        }
        std::thread::sleep(Duration::from_millis(5));

        let events = events.lock().unwrap();
        assert!(
            !events.iter().any(|event| event == "b:third:volume:0.004"),
            "stale worker touched the replacement deck: {events:?}"
        );
    }

    #[test]
    fn guarded_transition_aborts_after_stopping_the_incoming_deck() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let first = RecordingBackend::new("a", Arc::clone(&events));
        let second = RecordingBackend::new("b", Arc::clone(&events));
        let mut mixer = DualDeck::new(first, second);
        let checks = std::sync::atomic::AtomicUsize::new(0);

        mixer
            .transition_to_guarded(
                &StreamSource::new("replacement"),
                Duration::from_millis(1),
                &|| checks.fetch_add(1, Ordering::AcqRel) < 2,
            )
            .unwrap();

        let events = events.lock().unwrap();
        assert!(events.iter().any(|event| event == "b::stop"));
        assert!(!events.iter().any(|event| event == "b:replacement:load"));
        assert!(!events.iter().any(|event| event == "b:replacement:play"));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn incoming_cancellation_handle_tracks_dynamic_deck_slot() {
        let first = NativeEngine::new().unwrap();
        let second = NativeEngine::new().unwrap();
        let first_handle = first.cancellation_handle().unwrap();
        let second_handle = second.cancellation_handle().unwrap();
        let mixer = DualDeck::new(first, second);

        assert!(mixer.incoming_cancellation_handle().is_none());
        assert!(
            mixer
                .cancellation_handle()
                .unwrap()
                .same_allocation(&first_handle)
        );

        mixer.incoming_slot.store(1, Ordering::Release);
        assert!(
            mixer
                .incoming_cancellation_handle()
                .unwrap()
                .same_allocation(&second_handle)
        );

        mixer.active_slot.store(1, Ordering::Release);
        mixer.incoming_slot.store(usize::MAX, Ordering::Release);
        assert!(
            mixer
                .cancellation_handle()
                .unwrap()
                .same_allocation(&second_handle)
        );

        mixer.incoming_slot.store(0, Ordering::Release);
        assert!(
            mixer
                .cancellation_handle()
                .unwrap()
                .same_allocation(&first_handle)
        );
    }
}
