use std::time::Duration;

use crate::{BackendError, StreamSource};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackendTelemetry {
    pub position_ms: u64,
    pub duration_ms: u64,
    pub buffered_ms: u64,
    pub underrun_count: u64,
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
        _duration: Duration,
    ) -> Result<(), BackendError> {
        self.stop()?;
        self.load(source)?;
        self.play()
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
}
