use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use zerobeat_protocol::{PlaybackStatus, RepeatMode};

use crate::{App, HitMap, MouseTarget, theme};

pub fn render_player(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    hits.add(MouseTarget::Player, area);
    frame.render_widget(
        Block::new()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme::BORDER))
            .style(Style::default().bg(theme::SURFACE)),
        area,
    );
    let inner = Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(1),
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
    );
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(5),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let Some(track) = app.playback().current.as_ref() else {
        render_empty(frame, rows.as_ref());
        return;
    };

    let status = match app.playback().status {
        PlaybackStatus::Playing => "PLAYING",
        PlaybackStatus::Paused => "PAUSED",
        PlaybackStatus::Resolving => "RESOLVING",
        PlaybackStatus::Buffering => "BUFFERING",
        PlaybackStatus::Ended => "ENDED",
        PlaybackStatus::Failed => "FAILED",
        PlaybackStatus::Idle => "IDLE",
    };
    let context = app.playback().error.as_deref().unwrap_or(status);
    centered(
        frame,
        rows[0],
        Line::from(vec![
            Span::styled(context, Style::default().fg(theme::ACCENT)),
            Span::styled(
                format!(
                    "  ·  NATIVE OUTPUT 48 kHz  ·  {} queued",
                    app.playback().queue.len()
                ),
                Style::default().fg(theme::TEXT_MUTED),
            ),
        ]),
    );
    centered(
        frame,
        rows[1],
        Line::from(vec![
            Span::styled(
                &track.title,
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  —  {}", track.artist),
                Style::default().fg(theme::TEXT_MUTED),
            ),
        ]),
    );
    for (index, line) in spectrum_rows(app.spectrum(), rows[2].width)
        .into_iter()
        .enumerate()
    {
        centered(
            frame,
            Rect::new(rows[2].x, rows[2].y + index as u16, rows[2].width, 1),
            Line::styled(line, Style::default().fg(theme::ACCENT)),
        );
    }
    render_progress(frame, rows[3], app, hits);
    render_controls(frame, rows[4], app, hits);
    centered(
        frame,
        rows[5],
        Line::styled(
            "/ search  ·  ↑/↓ browse  ·  Enter select  ·  ←/→ seek 10s  ·  q quit",
            Style::default().fg(theme::TEXT_MUTED),
        ),
    );
}

fn render_progress(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    let layout = Layout::horizontal([
        Constraint::Length(8),
        Constraint::Min(20),
        Constraint::Length(8),
    ])
    .horizontal_margin((area.width / 12).min(8))
    .split(area);
    let progress = if app.playback().duration_ms == 0 {
        0.0
    } else {
        app.playback().position_ms as f64 / app.playback().duration_ms as f64
    };
    let width = usize::from(layout[1].width);
    let filled = ((width as f64 * progress.clamp(0.0, 1.0)).round() as usize).min(width);
    let bar = format!(
        "{}{}",
        "━".repeat(filled),
        "─".repeat(width.saturating_sub(filled))
    );
    frame.render_widget(
        Paragraph::new(clock(app.playback().position_ms))
            .alignment(Alignment::Right)
            .style(Style::default().fg(theme::TEXT_MUTED)),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(bar).style(Style::default().fg(theme::ACCENT)),
        layout[1],
    );
    frame.render_widget(
        Paragraph::new(format!(
            "-{}",
            clock(
                app.playback()
                    .duration_ms
                    .saturating_sub(app.playback().position_ms)
            )
        ))
        .style(Style::default().fg(theme::TEXT_MUTED)),
        layout[2],
    );
    hits.add(MouseTarget::Progress, layout[1]);
}

fn render_controls(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    let repeat = match app.playback().repeat_mode {
        RepeatMode::Off => "Repeat Off",
        RepeatMode::All => "Repeat All",
        RepeatMode::One => "Repeat One",
    };
    let playing = app.playback().status == PlaybackStatus::Playing;
    let liked = app
        .playback()
        .current
        .as_ref()
        .is_some_and(|track| app.library().liked.iter().any(|liked| liked.id == track.id));
    let volume = if app.playback().muted {
        "Muted".to_owned()
    } else {
        format!("Vol {}%", app.playback().volume_percent)
    };
    let wide = area.width >= 112;
    let transport_disabled = app.transport_cooling_down();
    let controls = [
        (
            MouseTarget::Shuffle,
            if wide { "(s) Shuffle" } else { "(s)" }.to_owned(),
            app.playback().shuffle,
            false,
        ),
        (
            MouseTarget::Previous,
            if wide { "(p) Prev" } else { "(p)" }.to_owned(),
            false,
            transport_disabled,
        ),
        (
            MouseTarget::PlayPause,
            if wide {
                if playing {
                    "(Space) Pause"
                } else {
                    "(Space) Play"
                }
            } else {
                "(␠)"
            }
            .to_owned(),
            true,
            false,
        ),
        (
            MouseTarget::Next,
            if wide { "(n) Next" } else { "(n)" }.to_owned(),
            false,
            transport_disabled,
        ),
        (
            MouseTarget::Repeat,
            if wide {
                format!("(r) {repeat}")
            } else {
                "(r)".to_owned()
            },
            app.playback().repeat_mode != RepeatMode::Off,
            false,
        ),
        (
            MouseTarget::Like,
            if wide {
                if liked { "(l) ♥" } else { "(l) ♡" }
            } else {
                "(l)"
            }
            .to_owned(),
            liked,
            false,
        ),
        (
            MouseTarget::Lyrics,
            if wide { "(y) Lyrics" } else { "(y)" }.to_owned(),
            app.lyrics().visible,
            false,
        ),
        (
            MouseTarget::Queue,
            if wide { "(u) Queue" } else { "(u)" }.to_owned(),
            app.queue_focused(),
            false,
        ),
        (
            MouseTarget::Mute,
            if wide {
                format!("(m) {volume}")
            } else if app.playback().muted {
                "(m)".to_owned()
            } else {
                format!("(m){}", app.playback().volume_percent)
            },
            app.playback().muted,
            false,
        ),
    ];
    let gap = if wide { 2_u16 } else { 1_u16 };
    let content_width = controls.iter().fold(0_u16, |width, (_, label, _, _)| {
        width.saturating_add(label.chars().count() as u16)
    });
    let total_width = content_width.saturating_add(gap * (controls.len() as u16 - 1));
    let mut x = area
        .x
        .saturating_add(area.width.saturating_sub(total_width) / 2);
    for (index, (target, label, active, disabled)) in controls.iter().enumerate() {
        let width = label.chars().count() as u16;
        let region = Rect::new(x, area.y, width, 1);
        frame.render_widget(
            Paragraph::new(label.as_str()).style(Style::default().fg(if *disabled {
                theme::BORDER
            } else if *active {
                theme::ACCENT
            } else {
                theme::TEXT_MUTED
            })),
            region,
        );
        hits.add(*target, region);
        x = x.saturating_add(width);
        if index + 1 < controls.len() {
            x = x.saturating_add(gap);
        }
    }
}

fn render_empty(frame: &mut Frame, rows: &[Rect]) {
    centered(
        frame,
        rows[1],
        Line::styled(
            "Nothing playing",
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
    );
    centered(
        frame,
        rows[3],
        Line::styled(
            "Search a track to wake the studio deck",
            Style::default().fg(theme::TEXT_MUTED),
        ),
    );
}

fn centered(frame: &mut Frame, area: Rect, line: Line<'_>) {
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
}

fn spectrum_rows(spectrum: &[u8; 24], available_width: u16) -> Vec<String> {
    const WAVE_ROWS: usize = 4;
    const DOT_ROWS: usize = 4;
    const BAR_COUNT: usize = 24;
    const SLOT_WIDTH: usize = 2;
    const WIDTH: usize = BAR_COUNT * SLOT_WIDTH - 1;
    const MAX_LEVEL: usize = WAVE_ROWS * DOT_ROWS;
    const LEFT_DOTS: [u32; DOT_ROWS + 1] = [0x00, 0x40, 0x44, 0x46, 0x47];

    let mut rows = vec![vec![' '; WIDTH]; WAVE_ROWS + 1];
    for (band, value) in spectrum.iter().enumerate() {
        let height = (usize::from((*value).min(100)) * MAX_LEVEL).div_ceil(100);
        let column = band * SLOT_WIDTH;
        for (row, output) in rows.iter_mut().take(WAVE_ROWS).enumerate() {
            let levels = height
                .saturating_sub((WAVE_ROWS - 1 - row) * DOT_ROWS)
                .min(DOT_ROWS);
            if levels == 0 {
                continue;
            }
            output[column] =
                char::from_u32(0x2800 + LEFT_DOTS[levels]).expect("valid Braille glyph");
        }
    }
    rows[WAVE_ROWS].fill('─');
    let output_width = usize::from(available_width).min(WIDTH);
    let crop_start = WIDTH.saturating_sub(output_width) / 2;
    let crop_end = crop_start + output_width;
    rows.into_iter()
        .map(|row| row[crop_start..crop_end].iter().collect())
        .collect()
}

fn clock(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::spectrum_rows;

    #[test]
    fn spectrum_bars_keep_fixed_columns_when_amplitude_changes() {
        let mut low = [0; 24];
        low[2] = 12;
        let mut high = [0; 24];
        high[2] = 100;

        let low_columns = active_columns(&spectrum_rows(&low, 80));
        let high_columns = active_columns(&spectrum_rows(&high, 80));

        assert_eq!(low_columns, high_columns);
        assert_eq!(low_columns.len(), 1);
        assert_eq!(low_columns[0], 4);

        let low_rows = spectrum_rows(&low, 80);
        let high_rows = spectrum_rows(&high, 80);
        assert_eq!(glyph_at(&low_rows[3], 4), '\u{2844}');
        assert!(
            high_rows[..4]
                .iter()
                .all(|row| glyph_at(row, 4) == '\u{2847}')
        );
    }

    #[test]
    fn isolated_band_does_not_spread_horizontally() {
        let mut spectrum = [0; 24];
        spectrum[12] = 100;

        let columns = active_columns(&spectrum_rows(&spectrum, 80));

        assert_eq!(columns, vec![24]);
    }

    #[test]
    fn all_bands_use_fixed_two_cell_slots() {
        let columns = active_columns(&spectrum_rows(&[100; 24], 80));

        assert_eq!(columns, (0..24).map(|band| band * 2).collect::<Vec<_>>());
    }

    #[test]
    fn narrow_spectrum_crops_symmetrically_without_resampling_bars() {
        let rows = spectrum_rows(&[100; 24], 36);

        assert!(rows.iter().all(|row| row.chars().count() == 36));
        assert_eq!(
            active_columns(&rows),
            (0..18).map(|column| column * 2 + 1).collect::<Vec<_>>()
        );
    }

    #[test]
    fn narrow_isolated_band_keeps_a_fixed_x_position_across_amplitude() {
        let mut low = [0; 24];
        low[12] = 25;
        let mut high = [0; 24];
        high[12] = 100;

        let low_columns = active_columns(&spectrum_rows(&low, 36));
        let high_columns = active_columns(&spectrum_rows(&high, 36));

        assert_eq!(low_columns, high_columns);
        assert_eq!(low_columns, vec![19]);
    }

    #[test]
    fn zero_spectrum_is_a_stable_horizontal_baseline() {
        let rows = spectrum_rows(&[0; 24], 80);

        assert_eq!(rows.len(), 5);
        assert!(rows[..4].iter().all(|row| row.chars().all(|c| c == ' ')));
        assert!(rows[4].chars().all(|c| c == '─'));
    }

    fn active_columns(rows: &[String]) -> Vec<usize> {
        rows.iter()
            .flat_map(|row| {
                row.chars().enumerate().filter_map(|(column, character)| {
                    (!character.is_whitespace() && character != '─').then_some(column)
                })
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn glyph_at(row: &str, column: usize) -> char {
        row.chars().nth(column).expect("glyph column")
    }
}
