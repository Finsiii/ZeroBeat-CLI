use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::App;
use crate::theme;

pub fn render_player(frame: &mut Frame, area: Rect, app: &App) {
    let Some(track) = app.playback().current.as_ref() else {
        render_empty_player(frame, area);
        return;
    };
    let icon = match app.playback().status {
        zerobeat_protocol::PlaybackStatus::Playing => "▶",
        zerobeat_protocol::PlaybackStatus::Paused => "Ⅱ",
        zerobeat_protocol::PlaybackStatus::Resolving
        | zerobeat_protocol::PlaybackStatus::Buffering => "◌",
        zerobeat_protocol::PlaybackStatus::Failed => "!",
        _ => "■",
    };
    let status = app
        .playback()
        .error
        .as_deref()
        .unwrap_or(match app.playback().status {
            zerobeat_protocol::PlaybackStatus::Resolving => "Resolving stream…",
            zerobeat_protocol::PlaybackStatus::Buffering => "Buffering…",
            zerobeat_protocol::PlaybackStatus::Playing => "Space pause · ←/→ seek · n next",
            zerobeat_protocol::PlaybackStatus::Paused => "Space resume · ←/→ seek · n next",
            zerobeat_protocol::PlaybackStatus::Ended => "Playback ended",
            _ => "",
        });
    let player = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                format!("  {icon}  {} — {}", track.title, track.artist),
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "   {} / {}   🔊 {}%",
                    clock(app.playback().position_ms),
                    clock(app.playback().duration_ms),
                    app.playback().volume_percent
                ),
                Style::default().fg(theme::TEXT_MUTED),
            ),
        ]),
        Line::styled(
            format!("      {status}"),
            Style::default().fg(theme::TEXT_MUTED),
        ),
    ])
    .block(
        Block::new()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme::BORDER)),
    )
    .style(Style::default().bg(theme::SURFACE));
    frame.render_widget(player, area);
}

fn render_empty_player(frame: &mut Frame, area: Rect) {
    let player = Paragraph::new(vec![
        Line::styled(
            "  ◯  Nothing playing",
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            "      Search and press Enter to start listening",
            Style::default().fg(theme::TEXT_MUTED),
        ),
    ])
    .block(
        Block::new()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme::BORDER)),
    )
    .style(Style::default().bg(theme::SURFACE));
    frame.render_widget(player, area);
}

fn clock(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}
