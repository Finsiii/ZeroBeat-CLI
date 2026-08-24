use std::{
    ffi::{CStr, CString, c_char, c_int},
    ptr::NonNull,
};

use crate::{AudioBackend, BackendError, BackendTelemetry, SPECTRUM_BAND_COUNT, StreamSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeState {
    Idle,
    Opening,
    Buffering,
    Ready,
    Playing,
    Paused,
    Ended,
    Error,
    Released,
}

pub struct NativeEngine {
    raw: NonNull<ZbEngine>,
}

impl NativeEngine {
    pub fn new() -> Result<Self, BackendError> {
        NonNull::new(unsafe { zb_engine_create() })
            .map(|raw| Self { raw })
            .ok_or_else(|| BackendError::Unavailable("native engine allocation failed".into()))
    }

    pub fn state(&self) -> NativeState {
        match unsafe { zb_engine_get_state(self.raw.as_ptr()) } {
            0 => NativeState::Idle,
            1 => NativeState::Opening,
            2 => NativeState::Buffering,
            3 => NativeState::Ready,
            4 => NativeState::Playing,
            5 => NativeState::Paused,
            6 => NativeState::Ended,
            8 => NativeState::Released,
            _ => NativeState::Error,
        }
    }

    pub fn position_ms(&self) -> u64 {
        non_negative(unsafe { zb_engine_get_position_ms(self.raw.as_ptr()) })
    }

    pub fn duration_ms(&self) -> u64 {
        non_negative(unsafe { zb_engine_get_duration_ms(self.raw.as_ptr()) })
    }

    pub fn buffered_ms(&self) -> u64 {
        non_negative(unsafe { zb_engine_get_buffered_ms(self.raw.as_ptr()) })
    }

    pub fn underrun_count(&self) -> u64 {
        non_negative(unsafe { zb_engine_get_underrun_count(self.raw.as_ptr()) })
    }

    pub fn spectrum(&self) -> [u8; SPECTRUM_BAND_COUNT] {
        let mut bands = [0; SPECTRUM_BAND_COUNT];
        let result = unsafe {
            zb_engine_get_spectrum(
                self.raw.as_ptr(),
                bands.as_mut_ptr(),
                i32::try_from(bands.len()).unwrap_or(i32::MAX),
            )
        };
        if result == 0 {
            bands
        } else {
            [0; SPECTRUM_BAND_COUNT]
        }
    }

    pub fn analyze_spectrum(
        samples: &[f32],
        channels: u8,
    ) -> Result<[u8; SPECTRUM_BAND_COUNT], BackendError> {
        if channels == 0 || samples.is_empty() || !samples.len().is_multiple_of(channels.into()) {
            return Err(BackendError::Failed("invalid spectrum PCM shape".into()));
        }
        let mut bands = [0; SPECTRUM_BAND_COUNT];
        let result = unsafe {
            zb_engine_analyze_spectrum(
                samples.as_ptr(),
                i64::try_from(samples.len()).unwrap_or(i64::MAX),
                i32::from(channels),
                bands.as_mut_ptr(),
                i32::try_from(bands.len()).unwrap_or(i32::MAX),
            )
        };
        if result == 0 {
            Ok(bands)
        } else {
            Err(BackendError::Failed(format!(
                "native spectrum analysis failed with code {result}"
            )))
        }
    }

    pub fn seek(&mut self, position_ms: u64) -> Result<(), BackendError> {
        self.check(unsafe {
            zb_engine_seek_ms(
                self.raw.as_ptr(),
                i64::try_from(position_ms).unwrap_or(i64::MAX),
            )
        })
    }

    pub fn set_volume(&mut self, volume: f32) -> Result<(), BackendError> {
        self.check(unsafe { zb_engine_set_volume(self.raw.as_ptr(), volume) })
    }

    fn check(&self, result: c_int) -> Result<(), BackendError> {
        if result == 0 {
            return Ok(());
        }
        let message = unsafe { zb_engine_get_last_error(self.raw.as_ptr()) };
        let message = if message.is_null() {
            format!("native error {result}")
        } else {
            unsafe { CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned()
        };
        Err(BackendError::Failed(message))
    }
}

impl AudioBackend for NativeEngine {
    fn load(&mut self, source: &StreamSource) -> Result<(), BackendError> {
        let mut serialized_headers = String::new();
        let mut user_agent = String::new();
        for (name, value) in &source.headers {
            if name.contains(['\r', '\n', ':']) || value.contains(['\r', '\n']) {
                return Err(BackendError::Failed("invalid audio HTTP header".into()));
            }
            if name.eq_ignore_ascii_case("user-agent") {
                user_agent.clone_from(value);
            } else {
                serialized_headers.push_str(name);
                serialized_headers.push_str(": ");
                serialized_headers.push_str(value);
                serialized_headers.push_str("\r\n");
            }
        }
        let serialized_headers = CString::new(serialized_headers)
            .map_err(|_| BackendError::Failed("audio HTTP header contains a null byte".into()))?;
        let user_agent = CString::new(user_agent)
            .map_err(|_| BackendError::Failed("audio user agent contains a null byte".into()))?;
        self.check(unsafe {
            zb_engine_set_http_headers(
                self.raw.as_ptr(),
                serialized_headers.as_ptr(),
                user_agent.as_ptr(),
            )
        })?;
        let source = CString::new(source.url.as_str())
            .map_err(|_| BackendError::Failed("audio source contains a null byte".into()))?;
        let result = if source.to_bytes().starts_with(b"http://")
            || source.to_bytes().starts_with(b"https://")
        {
            unsafe { zb_engine_prebuffer_url(self.raw.as_ptr(), source.as_ptr()) }
        } else {
            unsafe { zb_engine_prebuffer_file(self.raw.as_ptr(), source.as_ptr()) }
        };
        self.check(result)
    }

    fn play(&mut self) -> Result<(), BackendError> {
        self.check(unsafe { zb_engine_play(self.raw.as_ptr()) })
    }

    fn pause(&mut self) -> Result<(), BackendError> {
        self.check(unsafe { zb_engine_pause(self.raw.as_ptr()) })
    }

    fn stop(&mut self) -> Result<(), BackendError> {
        self.check(unsafe { zb_engine_stop(self.raw.as_ptr()) })
    }

    fn seek(&mut self, position_ms: u64) -> Result<(), BackendError> {
        NativeEngine::seek(self, position_ms)
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), BackendError> {
        NativeEngine::set_volume(self, volume)
    }

    fn telemetry(&self) -> BackendTelemetry {
        BackendTelemetry {
            position_ms: self.position_ms(),
            duration_ms: self.duration_ms(),
            buffered_ms: self.buffered_ms(),
            underrun_count: self.underrun_count(),
            spectrum: self.spectrum(),
            ended: self.state() == NativeState::Ended,
        }
    }
}

impl Drop for NativeEngine {
    fn drop(&mut self) {
        unsafe { zb_engine_destroy(self.raw.as_ptr()) };
    }
}

unsafe impl Send for NativeEngine {}

fn non_negative(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

#[repr(C)]
struct ZbEngine {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn zb_engine_create() -> *mut ZbEngine;
    fn zb_engine_destroy(engine: *mut ZbEngine);
    fn zb_engine_prebuffer_file(engine: *mut ZbEngine, path: *const c_char) -> c_int;
    fn zb_engine_prebuffer_url(engine: *mut ZbEngine, url: *const c_char) -> c_int;
    fn zb_engine_play(engine: *mut ZbEngine) -> c_int;
    fn zb_engine_pause(engine: *mut ZbEngine) -> c_int;
    fn zb_engine_stop(engine: *mut ZbEngine) -> c_int;
    fn zb_engine_seek_ms(engine: *mut ZbEngine, position_ms: i64) -> c_int;
    fn zb_engine_set_volume(engine: *mut ZbEngine, volume: f32) -> c_int;
    fn zb_engine_set_http_headers(
        engine: *mut ZbEngine,
        headers: *const c_char,
        user_agent: *const c_char,
    ) -> c_int;
    fn zb_engine_get_state(engine: *mut ZbEngine) -> c_int;
    fn zb_engine_get_position_ms(engine: *mut ZbEngine) -> i64;
    fn zb_engine_get_duration_ms(engine: *mut ZbEngine) -> i64;
    fn zb_engine_get_buffered_ms(engine: *mut ZbEngine) -> i64;
    fn zb_engine_get_underrun_count(engine: *mut ZbEngine) -> i64;
    fn zb_engine_get_spectrum(engine: *mut ZbEngine, bands: *mut u8, band_count: c_int) -> c_int;
    fn zb_engine_analyze_spectrum(
        samples: *const f32,
        sample_count: i64,
        channels: c_int,
        bands: *mut u8,
        band_count: c_int,
    ) -> c_int;
    fn zb_engine_get_last_error(engine: *mut ZbEngine) -> *const c_char;
}
