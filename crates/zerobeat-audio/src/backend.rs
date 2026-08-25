use std::{sync::Arc, time::Duration};

#[cfg(target_os = "linux")]
use crate::NativeCancellationHandle;
use crate::{BackendError, StreamSource};

pub const SPECTRUM_BAND_COUNT: usize = 24;

#[cfg(target_os = "linux")]
pub type CancellationController = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackendTelemetry {
    pub position_ms: u64,
    pub duration_ms: u64,
    pub buffered_ms: u64,
    pub underrun_count: u64,
    pub spectrum: [u8; SPECTRUM_BAND_COUNT],
    pub ended: bool,
}

pub trait AudioBackend: Send {
    fn load(&mut self, source: &StreamSource) -> Result<(), BackendError>;
    fn play(&mut self) -> Result<(), BackendError>;
    fn pause(&mut self) -> Result<(), BackendError>;
    fn stop(&mut self) -> Result<(), BackendError>;

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
        _duration: Duration,
        should_continue: &dyn Fn() -> bool,
    ) -> Result<(), BackendError> {
        if !should_continue() {
            return Ok(());
        }
        self.stop()?;
        self.load(source)?;
        if !should_continue() {
            return self.stop();
        }
        self.play()?;
        if !should_continue() {
            return self.stop();
        }
        Ok(())
    }

    fn seek(&mut self, _position_ms: u64) -> Result<(), BackendError> {
        Err(BackendError::Unavailable("seeking is not supported".into()))
    }

    fn set_volume(&mut self, _volume: f32) -> Result<(), BackendError> {
        Err(BackendError::Unavailable(
            "volume control is not supported".into(),
        ))
    }

    fn telemetry(&self) -> BackendTelemetry {
        BackendTelemetry::default()
    }

    fn failed(&self) -> bool {
        false
    }

    fn last_error(&self) -> Option<String> {
        None
    }

    #[cfg(target_os = "linux")]
    fn cancellation_handle(&self) -> Option<NativeCancellationHandle> {
        None
    }

    #[cfg(target_os = "linux")]
    fn cancel_current_load(&self) {
        if let Some(handle) = self.cancellation_handle() {
            handle.cancel();
        }
    }

    #[cfg(target_os = "linux")]
    fn cancellation_controller(&self) -> Option<CancellationController> {
        self.cancellation_handle()
            .map(|handle| Arc::new(move || handle.cancel()) as CancellationController)
    }
}
