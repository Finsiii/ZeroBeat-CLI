use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::theme;

pub fn render_player(frame: &mut Frame, area: Rect) {
    let player = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                "  ◯  Nothing playing",
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("                                      ", Style::default()),
            Span::styled(
                "♡   ━━━━━━━━━━━━━   🔊",
                Style::default().fg(theme::TEXT_MUTED),
            ),
        ]),
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
