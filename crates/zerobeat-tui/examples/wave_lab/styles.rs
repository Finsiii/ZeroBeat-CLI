#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaveStyle {
    ThinBraille,
    DotMatrix,
    PeakCaps,
    SoftBlocks,
    TwinRail,
    Oscilloscope,
}

impl WaveStyle {
    pub const ALL: [Self; 6] = [
        Self::ThinBraille,
        Self::DotMatrix,
        Self::PeakCaps,
        Self::SoftBlocks,
        Self::TwinRail,
        Self::Oscilloscope,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::ThinBraille => "Thin Braille",
            Self::DotMatrix => "Dot Matrix",
            Self::PeakCaps => "Peak Caps",
            Self::SoftBlocks => "Soft Blocks",
            Self::TwinRail => "Twin Rail",
            Self::Oscilloscope => "Oscilloscope",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::ThinBraille => "Fine segmented columns with sixteen vertical levels",
            Self::DotMatrix => "Airy discrete points close to the visual reference",
            Self::PeakCaps => "Needle stems with a distinct transient peak",
            Self::SoftBlocks => "Dense classic spectrum with smooth partial blocks",
            Self::TwinRail => "Balanced energy expanding from a quiet center rail",
            Self::Oscilloscope => "Continuous signal ribbon instead of frequency bars",
        }
    }
}

pub fn synthetic_signal(frame: u64, intensity: u16) -> [f32; 32] {
    let time = frame as f32 * 0.085;
    let gain = f32::from(intensity).clamp(30.0, 150.0) / 100.0;
    std::array::from_fn(|index| {
        let x = index as f32 / 31.0;
        let low = (time * 0.9 + x * 7.5).sin() * 0.24;
        let detail = (time * 1.37 - x * 15.0).sin() * 0.13;
        let pulse = ((time * 0.52).sin().max(0.0) * (1.0 - x) * 0.18).max(0.0);
        ((0.43 + low + detail + pulse) * (1.0 - x * 0.18) * gain).clamp(0.02, 1.0)
    })
}

pub fn render(
    style: WaveStyle,
    signal: &[f32],
    width: usize,
    height: usize,
    intensity: u16,
) -> Vec<String> {
    let canvas = match style {
        WaveStyle::ThinBraille => thin_braille(signal, width, height, intensity),
        WaveStyle::DotMatrix => dot_matrix(signal, width, height, intensity),
        WaveStyle::PeakCaps => peak_caps(signal, width, height, intensity),
        WaveStyle::SoftBlocks => soft_blocks(signal, width, height, intensity),
        WaveStyle::TwinRail => twin_rail(signal, width, height, intensity),
        WaveStyle::Oscilloscope => oscilloscope(signal, width, height, intensity),
    };
    canvas
        .into_iter()
        .map(|row| row.into_iter().collect())
        .collect()
}

fn thin_braille(signal: &[f32], width: usize, height: usize, intensity: u16) -> Canvas {
    const LEFT_DOTS: [u32; 5] = [0x00, 0x40, 0x44, 0x46, 0x47];
    let mut canvas = blank(width, height);
    for column in (0..width).step_by(2) {
        let level =
            (sample(signal, column, width, intensity) * (height * 4) as f32).ceil() as usize;
        for (row, cells) in canvas.iter_mut().enumerate() {
            let dots = level
                .saturating_sub((height.saturating_sub(1 + row)) * 4)
                .min(4);
            if dots > 0 {
                cells[column] = char::from_u32(0x2800 + LEFT_DOTS[dots]).unwrap_or(' ');
            }
        }
    }
    canvas
}

fn dot_matrix(signal: &[f32], width: usize, height: usize, intensity: u16) -> Canvas {
    let mut canvas = blank(width, height);
    for column in (0..width).step_by(2) {
        let level = (sample(signal, column, width, intensity) * height as f32).ceil() as usize;
        for offset in 0..level.min(height) {
            canvas[height - 1 - offset][column] = '•';
        }
    }
    canvas
}

fn peak_caps(signal: &[f32], width: usize, height: usize, intensity: u16) -> Canvas {
    let mut canvas = blank(width, height);
    for column in (0..width).step_by(2) {
        let level = (sample(signal, column, width, intensity) * height as f32)
            .ceil()
            .max(1.0) as usize;
        let top = height.saturating_sub(level.min(height));
        canvas[top][column] = '╻';
        for cells in canvas.iter_mut().take(height).skip(top + 1) {
            cells[column] = '│';
        }
    }
    canvas
}

fn soft_blocks(signal: &[f32], width: usize, height: usize, intensity: u16) -> Canvas {
    const PARTIAL: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let mut canvas = blank(width, height);
    for column in (0..width).step_by(2) {
        let scaled = sample(signal, column, width, intensity) * height as f32;
        let full = scaled.floor() as usize;
        for offset in 0..full.min(height) {
            canvas[height - 1 - offset][column] = '█';
        }
        if full < height {
            let partial = ((scaled.fract() * 8.0).ceil() as usize).clamp(1, 8);
            canvas[height - 1 - full][column] = PARTIAL[partial - 1];
        }
    }
    canvas
}

fn twin_rail(signal: &[f32], width: usize, height: usize, intensity: u16) -> Canvas {
    let mut canvas = blank(width, height);
    if height == 0 {
        return canvas;
    }
    let center = height / 2;
    let reach = center.min(height - 1 - center);
    for column in (0..width).step_by(2) {
        canvas[center][column] = '·';
        let level = (sample(signal, column, width, intensity) * reach as f32).ceil() as usize;
        for distance in 1..=level.min(reach) {
            canvas[center - distance][column] = if distance == level { '╹' } else { '│' };
            canvas[center + distance][column] = if distance == level { '╻' } else { '│' };
        }
    }
    canvas
}

fn oscilloscope(signal: &[f32], width: usize, height: usize, intensity: u16) -> Canvas {
    let mut canvas = blank(width, height);
    if width == 0 || height == 0 {
        return canvas;
    }
    let mut previous: Option<usize> = None;
    for column in 0..width {
        let value = sample(signal, column, width, intensity);
        let row = ((1.0 - value) * height.saturating_sub(1) as f32).round() as usize;
        if let Some(previous_row) = previous {
            let start = previous_row.min(row);
            let end = previous_row.max(row);
            for cells in canvas.iter_mut().take(end + 1).skip(start) {
                cells[column] = '│';
            }
            if previous_row == row {
                canvas[row][column] = '─';
            }
        }
        canvas[row][column] = '•';
        previous = Some(row);
    }
    canvas
}

type Canvas = Vec<Vec<char>>;

fn blank(width: usize, height: usize) -> Canvas {
    vec![vec![' '; width]; height]
}

fn sample(signal: &[f32], column: usize, width: usize, intensity: u16) -> f32 {
    if signal.is_empty() || width == 0 {
        return 0.0;
    }
    let index = column
        .saturating_mul(signal.len().saturating_sub(1))
        .checked_div(width.saturating_sub(1).max(1))
        .unwrap_or_default()
        .min(signal.len() - 1);
    (signal[index] * f32::from(intensity) / 100.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gallery_exposes_six_distinct_styles() {
        assert_eq!(WaveStyle::ALL.len(), 6);
        let mut names = WaveStyle::ALL.map(WaveStyle::name).to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 6);
    }

    #[test]
    fn every_style_fills_the_requested_canvas() {
        let signal = synthetic_signal(42, 100);

        for style in WaveStyle::ALL {
            let rows = render(style, &signal, 47, 7, 100);
            assert_eq!(rows.len(), 7, "{} height", style.name());
            assert!(
                rows.iter().all(|row| row.chars().count() == 47),
                "{} width",
                style.name()
            );
            assert!(
                rows.iter().any(|row| row.chars().any(|cell| cell != ' ')),
                "{} is blank",
                style.name()
            );
        }
    }

    #[test]
    fn synthetic_motion_is_deterministic_but_changes_over_time() {
        assert_eq!(synthetic_signal(12, 80), synthetic_signal(12, 80));
        assert_ne!(synthetic_signal(12, 80), synthetic_signal(13, 80));
    }
}
