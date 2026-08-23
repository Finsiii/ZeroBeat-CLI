use crate::{BackendError, StreamSource};

pub trait AudioBackend {
    fn load(&mut self, source: &StreamSource) -> Result<(), BackendError>;
    fn play(&mut self) -> Result<(), BackendError>;
    fn pause(&mut self) -> Result<(), BackendError>;
    fn stop(&mut self) -> Result<(), BackendError>;
}
