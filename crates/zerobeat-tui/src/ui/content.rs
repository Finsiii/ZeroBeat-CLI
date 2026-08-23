use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};
use zerobeat_core::Route;

use crate::{App, theme};

pub fn render_content(frame: &mut Frame, area: Rect, app: &App) {
    match app.route() {
        Route::Home => render_home(frame, area),
        Route::Search => render_search(frame, area, app),
        Route::Library => render_empty(frame, area, "Library", "Your local collection starts here"),
        Route::Downloads => {
            render_empty(frame, area, "Downloads", "Offline music will appear here")
        }
        Route::Settings => render_settings(frame, area),
    }
}

fn render_home(frame: &mut Frame, area: Rect) {
    let rows = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(7),
        Constraint::Min(5),
    ])
    .margin(2)
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "Good evening",
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                "Pick up where you left off",
                Style::default().fg(theme::TEXT_MUTED),
            ),
        ]),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "Continue listening",
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
            Line::styled(
                "Your recent music will appear here",
                Style::default().fg(theme::TEXT_MUTED),
            ),
        ])
        .block(card()),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(Line::styled(
            "Search for something you love and ZeroBeat will shape this space around you.",
            Style::default().fg(theme::TEXT_MUTED),
        ))
        .block(card().title(" Made for you ")),
        rows[2],
    );
}

fn render_search(frame: &mut Frame, area: Rect, app: &App) {
    let query = if app.search_query().is_empty() {
        "Type to search songs, artists, albums, and playlists"
    } else {
        app.search_query()
    };
    let border = if app.search_focused() {
        theme::ACCENT
    } else {
        theme::BORDER
    };
    let search = Paragraph::new(Line::styled(
        format!("  / {query}"),
        Style::default().fg(theme::TEXT),
    ))
    .block(
        Block::bordered()
            .title(" Search ")
            .border_style(Style::default().fg(border)),
    )
    .style(Style::default().bg(theme::SURFACE));
    frame.render_widget(search, area.inner(ratatui::layout::Margin::new(2, 2)));
}

fn render_empty(frame: &mut Frame, area: Rect, title: &str, message: &str) {
    let paragraph = Paragraph::new(Line::styled(
        message,
        Style::default().fg(theme::TEXT_MUTED),
    ))
    .block(card().title(format!(" {title} ")));
    frame.render_widget(paragraph, area.inner(ratatui::layout::Margin::new(2, 2)));
}

fn render_settings(frame: &mut Frame, area: Rect) {
    let content = Paragraph::new(vec![
        Line::styled(
            "Lightweight Crossfade Engine",
            Style::default().fg(theme::TEXT),
        ),
        Line::styled(
            "Optimized for smooth playback under 100 MiB",
            Style::default().fg(theme::TEXT_MUTED),
        ),
        Line::raw(""),
        Line::styled("Native DJ Engine", Style::default().fg(theme::TEXT)),
        Line::styled(
            "Currently available on ZeroBeat for Android",
            Style::default().fg(theme::ACCENT),
        ),
    ])
    .block(card().title(" Audio "));
    frame.render_widget(content, area.inner(ratatui::layout::Margin::new(2, 2)));
}

fn card<'a>() -> Block<'a> {
    Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::SURFACE_HIGH))
        .padding(ratatui::widgets::Padding::horizontal(1))
}
