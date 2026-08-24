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
    let controls = [
        (
            MouseTarget::Shuffle,
            if wide { "(s) Shuffle" } else { "(s)" }.to_owned(),
            app.playback().shuffle,
        ),
        (
            MouseTarget::Previous,
            if wide { "(p) Prev" } else { "(p)" }.to_owned(),
            false,
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
        ),
        (
            MouseTarget::Next,
            if wide { "(n) Next" } else { "(n)" }.to_owned(),
            false,
        ),
        (
            MouseTarget::Repeat,
            if wide {
                format!("(r) {repeat}")
            } else {
                "(r)".to_owned()
            },
            app.playback().repeat_mode != RepeatMode::Off,
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
        ),
        (
            MouseTarget::Lyrics,
            if wide { "(y) Lyrics" } else { "(y)" }.to_owned(),
            app.lyrics().visible,
        ),
        (
            MouseTarget::Queue,
            if wide { "(u) Queue" } else { "(u)" }.to_owned(),
            app.queue_focused(),
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
        ),
    ];
    let gap = if wide { 2_u16 } else { 1_u16 };
    let content_width = controls.iter().fold(0_u16, |width, (_, label, _)| {
        width.saturating_add(label.chars().count() as u16)
    });
    let total_width = content_width.saturating_add(gap * (controls.len() as u16 - 1));
    let mut x = area
        .x
        .saturating_add(area.width.saturating_sub(total_width) / 2);
    for (index, (target, label, active)) in controls.iter().enumerate() {
        let width = label.chars().count() as u16;
        let region = Rect::new(x, area.y, width, 1);
        frame.render_widget(
            Paragraph::new(label.as_str()).style(Style::default().fg(if *active {
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
    const ROWS: usize = 5;
    const PARTIAL: [char; 4] = [' ', '▂', '▄', '▆'];
    let width = usize::from(available_width.saturating_sub(8).clamp(16, 88));
    if spectrum.iter().all(|value| *value == 0) {
        let mut rows = vec![String::new(); ROWS];
        rows[ROWS - 1] = "─".repeat(width);
        return rows;
    }

    let values = (0..width)
        .map(|column| {
            let scaled = column * (spectrum.len() - 1);
            let left = scaled / width.saturating_sub(1).max(1);
            let right = (left + 1).min(spectrum.len() - 1);
            let remainder = scaled % width.saturating_sub(1).max(1);
            let span = width.saturating_sub(1).max(1);
            (usize::from(spectrum[left]) * (span - remainder)
                + usize::from(spectrum[right]) * remainder)
                / span
        })
        .collect::<Vec<_>>();

    (0..ROWS)
        .map(|row| {
            values
                .iter()
                .map(|value| {
                    let units = value.saturating_mul(ROWS * 4) / 100;
                    let threshold = (ROWS - row - 1) * 4;
                    if units >= threshold + 4 {
                        '█'
                    } else {
                        PARTIAL[units.saturating_sub(threshold).min(PARTIAL.len() - 1)]
                    }
                })
                .collect()
        })
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
    fn resampling_preserves_distinct_native_peak_locations() {
        let mut low = [0; 24];
        low[2] = 100;
        let mut high = [0; 24];
        high[21] = 100;

        assert_ne!(spectrum_rows(&low, 80), spectrum_rows(&high, 80));
    }
}
