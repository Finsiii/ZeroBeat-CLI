use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};
use zerobeat_core::Route;
use zerobeat_protocol::SearchStatus;

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
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(5)])
        .spacing(1)
        .split(area.inner(ratatui::layout::Margin::new(2, 1)));
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
    frame.render_widget(search, rows[0]);

    let mut lines = Vec::new();
    match &app.search().status {
        SearchStatus::Idle => {
            lines.push(Line::styled(
                "Search by song, artist, album, or playlist",
                Style::default().fg(theme::TEXT),
            ));
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "Press / to focus · Enter to search",
                Style::default().fg(theme::TEXT_MUTED),
            ));
        }
        SearchStatus::Loading => lines.push(Line::styled(
            "Searching ZeroBeat…",
            Style::default().fg(theme::ACCENT),
        )),
        SearchStatus::Failed(message) => lines.push(Line::styled(
            format!("Search failed: {message}"),
            Style::default().fg(ratatui::style::Color::LightRed),
        )),
        SearchStatus::Ready if app.search().results.is_empty() => lines.push(Line::styled(
            "No results found. Try a different title or artist.",
            Style::default().fg(theme::TEXT_MUTED),
        )),
        SearchStatus::Ready => {
            for (index, track) in app.search().results.iter().enumerate() {
                let selected = index == app.search().selected_index;
                let marker = if selected { "▶" } else { " " };
                let minutes = track.duration_ms / 60_000;
                let seconds = track.duration_ms / 1_000 % 60;
                let style = if selected {
                    Style::default()
                        .fg(theme::TEXT)
                        .bg(theme::SURFACE_HIGH)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::TEXT_MUTED)
                };
                lines.push(Line::styled(
                    format!(
                        "{marker}  {}  ·  {}  {:02}:{:02}",
                        track.title, track.artist, minutes, seconds
                    ),
                    style,
                ));
            }
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "↑/↓ select · Enter play",
                Style::default().fg(theme::TEXT_MUTED),
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(card().title(format!(" {} results ", app.search().results.len()))),
        rows[1],
    );
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
