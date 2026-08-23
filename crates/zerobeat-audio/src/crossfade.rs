use std::time::Duration;

pub struct CrossfadeCurve;

impl CrossfadeCurve {
    pub fn gains(progress: f32) -> (f32, f32) {
        let angle = progress.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2;
        (angle.cos(), angle.sin())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrossfadeConfig {
    pub enabled: bool,
    pub duration: Duration,
    pub trim_silence: bool,
}

impl Default for CrossfadeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            duration: Duration::from_secs(6),
            trim_silence: true,
        }
    }
}
