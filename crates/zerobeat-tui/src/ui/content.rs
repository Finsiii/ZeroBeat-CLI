use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};
use zerobeat_core::Route;
use zerobeat_protocol::{DownloadStatus, LyricsStatus, SearchStatus};

use crate::{App, HitMap, MouseTarget, theme};

pub fn render_content(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    if app.queue_focused() {
        render_queue(frame, area, app, true, hits);
        return;
    }
    if app.lyrics().visible {
        render_lyrics(frame, area, app);
        return;
    }
    match app.route() {
        Route::Home => render_home(frame, area, app, hits),
        Route::Search => render_search(frame, area, app, hits),
        Route::Library => render_library(frame, area, app, hits),
        Route::RecentlyPlayed => render_recently_played(frame, area, app, hits),
        Route::Downloads => render_downloads(frame, area, app, hits),
        Route::Settings => render_settings(frame, area, app),
    }
}

pub(crate) fn render_queue(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    focused: bool,
    hits: &mut HitMap,
) {
    let mut lines = Vec::new();
    if app.playback().queue.is_empty() {
        lines.push(empty_line("Queue is empty"));
        lines.push(Line::raw(""));
        lines.push(empty_line("Press a to add tracks"));
    } else {
        for (index, track) in app.playback().queue.iter().enumerate() {
            let selected = focused && index == app.queue_selected();
            let top = area.y.saturating_add(2);
            hits.add(
                MouseTarget::QueueTrack(index),
                Rect::new(
                    area.x.saturating_add(1),
                    top.saturating_add(index as u16 * 2),
                    area.width.saturating_sub(2),
                    2,
                ),
            );
            lines.push(Line::styled(
                format!(
                    "{} {:02}  {}",
                    if selected { "▶" } else { " " },
                    index + 1,
                    track.title
                ),
                Style::default()
                    .fg(if selected || index == 0 {
                        theme::TEXT
                    } else {
                        theme::TEXT_MUTED
                    })
                    .add_modifier(if selected || index == 0 {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ));
            lines.push(Line::styled(
                format!("      {}", track.artist),
                Style::default().fg(theme::TEXT_MUTED),
            ));
        }
    }
    if focused {
        lines.push(Line::raw(""));
        lines.push(empty_line(
            "↑/↓ select · Enter play · Delete remove · x clear · u close",
        ));
    }
    frame.render_widget(
        Paragraph::new(lines).block(card().title(if focused {
            format!(" Queue · {} ", app.playback().queue.len())
        } else {
            format!(" Up next · {} ", app.playback().queue.len())
        })),
        area.inner(ratatui::layout::Margin::new(if focused { 2 } else { 1 }, 1)),
    );
}

fn render_lyrics(frame: &mut Frame, area: Rect, app: &App) {
    let title = if app.lyrics().synced {
        " Lyrics · Synced "
    } else {
        " Lyrics · Unsynced "
    };
    let mut lines = Vec::new();
    match &app.lyrics().status {
        LyricsStatus::Idle | LyricsStatus::Loading => {
            lines.push(Line::styled(
                "Finding the best lyrics…",
                Style::default().fg(theme::ACCENT),
            ));
        }
        LyricsStatus::Unavailable => {
            lines.push(empty_line("Lyrics are not available for this song"))
        }
        LyricsStatus::Failed(message) => lines.push(Line::styled(
            format!("Lyrics failed: {message}"),
            Style::default().fg(ratatui::style::Color::LightRed),
        )),
        LyricsStatus::Ready => {
            let active = app.lyrics().synced.then(|| {
                app.lyrics()
                    .lines
                    .iter()
                    .rposition(|line| {
                        line.start_ms
                            .is_some_and(|start| start <= app.playback().position_ms)
                    })
                    .unwrap_or(0)
            });
            let capacity = usize::from(area.height.saturating_sub(7).max(1));
            let start = active
                .map(|index| index.saturating_sub(capacity / 2))
                .unwrap_or(0)
                .min(app.lyrics().lines.len().saturating_sub(capacity));
            for (index, lyric) in app
                .lyrics()
                .lines
                .iter()
                .enumerate()
                .skip(start)
                .take(capacity)
            {
                let is_active = active == Some(index);
                lines.push(Line::styled(
                    format!("{}  {}", if is_active { "▶" } else { " " }, lyric.words),
                    if is_active {
                        Style::default()
                            .fg(theme::ACCENT)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme::TEXT_MUTED)
                    },
                ));
            }
        }
    }
    lines.push(Line::raw(""));
    lines.push(empty_line("y close lyrics · ←/→ seek · Space pause/resume"));
    frame.render_widget(
        Paragraph::new(lines).block(card().title(title)),
        area.inner(ratatui::layout::Margin::new(2, 1)),
    );
}

fn render_home(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
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
    let mut recent = vec![Line::styled(
        "Continue listening",
        Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD),
    )];
    if app.library().recent.is_empty() {
        recent.push(Line::raw(""));
        recent.push(Line::styled(
            "Search and play a song to shape your Home page",
            Style::default().fg(theme::TEXT_MUTED),
        ));
    } else {
        for (index, track) in app.library().recent.iter().take(3).enumerate() {
            hits.add(
                MouseTarget::ContentTrack(index),
                Rect::new(
                    rows[1].x.saturating_add(1),
                    rows[1].y.saturating_add(2 + index as u16),
                    rows[1].width.saturating_sub(2),
                    1,
                ),
            );
            recent.push(track_line(track, index == app.selected_index()));
        }
    }
    frame.render_widget(Paragraph::new(recent).block(card()), rows[1]);
    frame.render_widget(
        Paragraph::new(Line::styled(
            "Search for something you love and ZeroBeat will shape this space around you.",
            Style::default().fg(theme::TEXT_MUTED),
        ))
        .block(card().title(" Made for you ")),
        rows[2],
    );
}

fn render_library(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    let mut lines = Vec::new();
    let mut index = 0;
    lines.push(section_title("Liked songs"));
    if app.library().liked.is_empty() {
        lines.push(empty_line("Press l on any track to keep it here"));
    }
    for track in &app.library().liked {
        hits.add(
            MouseTarget::ContentTrack(index),
            Rect::new(
                area.x.saturating_add(4),
                area.y.saturating_add(4 + index as u16),
                area.width.saturating_sub(8),
                1,
            ),
        );
        lines.push(track_line(track, index == app.selected_index()));
        index += 1;
    }
    lines.push(Line::raw(""));
    lines.push(section_title("Recently played"));
    let mut recent_count = 0;
    for track in &app.library().recent {
        if app.library().liked.iter().any(|liked| liked.id == track.id) {
            continue;
        }
        lines.push(track_line(track, index == app.selected_index()));
        let row = lines.len().saturating_sub(1) as u16;
        hits.add(
            MouseTarget::ContentTrack(index),
            Rect::new(
                area.x.saturating_add(4),
                area.y.saturating_add(3 + row),
                area.width.saturating_sub(8),
                1,
            ),
        );
        index += 1;
        recent_count += 1;
    }
    if recent_count == 0 {
        lines.push(empty_line("No additional listening history yet"));
    }
    lines.push(Line::raw(""));
    lines.push(empty_line(
        "↑/↓ select · Enter play · l like/unlike · d download · a queue",
    ));
    frame.render_widget(
        Paragraph::new(lines).block(card().title(" Your local library ")),
        area.inner(ratatui::layout::Margin::new(2, 2)),
    );
}

fn render_recently_played(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    let mut lines = vec![section_title("Recently played")];
    if app.library().recent.is_empty() {
        lines.push(empty_line("Your listening history will appear here"));
    } else {
        for (index, track) in app.library().recent.iter().enumerate() {
            let row = lines.len() as u16;
            hits.add(
                MouseTarget::ContentTrack(index),
                Rect::new(
                    area.x.saturating_add(4),
                    area.y.saturating_add(3 + row),
                    area.width.saturating_sub(8),
                    1,
                ),
            );
            lines.push(track_line(track, index == app.selected_index()));
        }
    }
    lines.push(Line::raw(""));
    lines.push(empty_line("↑/↓ select · Enter play · a add to queue"));
    frame.render_widget(
        Paragraph::new(lines).block(card().title(format!(
            " Recently played · {} ",
            app.library().recent.len()
        ))),
        area.inner(ratatui::layout::Margin::new(2, 2)),
    );
}

fn render_downloads(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    let mut lines = Vec::new();
    if app.library().downloads.is_empty() {
        lines.push(section_title("Listen without a connection"));
        lines.push(Line::raw(""));
        lines.push(empty_line("Press d on a search result or library track"));
    } else {
        for (index, download) in app.library().downloads.iter().enumerate() {
            hits.add(
                MouseTarget::ContentTrack(index),
                Rect::new(
                    area.x.saturating_add(4),
                    area.y.saturating_add(3 + index as u16 * 2),
                    area.width.saturating_sub(8),
                    2,
                ),
            );
            lines.push(track_line(&download.track, index == app.selected_index()));
            let label = match download.status {
                DownloadStatus::Queued => "Queued",
                DownloadStatus::Downloading => "Downloading…",
                DownloadStatus::Available => "Available offline",
                DownloadStatus::Failed => download.error.as_deref().unwrap_or("Download failed"),
            };
            lines.push(Line::styled(
                format!("     {label}"),
                Style::default().fg(if download.status == DownloadStatus::Available {
                    theme::ACCENT
                } else {
                    theme::TEXT_MUTED
                }),
            ));
        }
        lines.push(Line::raw(""));
        lines.push(empty_line("↑/↓ select · Enter play · a queue"));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(card().title(format!(" Downloads · {} ", app.library().downloads.len()))),
        area.inner(ratatui::layout::Margin::new(2, 2)),
    );
}

fn render_search(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
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
    hits.add(MouseTarget::SearchInput, rows[0]);

    let mut lines = Vec::new();
    match &app.search().status {
        SearchStatus::Idle => {
            lines.push(Line::styled(
                "Search by song, artist, album, or playlist",
                Style::default().fg(theme::TEXT),
            ));
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                if app.search_focused() {
                    "Esc unfocus · Enter to search"
                } else {
                    "Press / to focus · Enter to search"
                },
                Style::default().fg(theme::TEXT_MUTED),
            ));
        }
        SearchStatus::Loading => {
            lines.push(Line::styled(
                "Searching ZeroBeat…",
                Style::default().fg(theme::ACCENT),
            ));
            if app.search_focused() {
                focused_search_hint(&mut lines);
            }
        }
        SearchStatus::Failed(message) => {
            lines.push(Line::styled(
                format!("Search failed: {message}"),
                Style::default().fg(ratatui::style::Color::LightRed),
            ));
            if app.search_focused() {
                focused_search_hint(&mut lines);
            }
        }
        SearchStatus::Ready if app.search().results.is_empty() => {
            lines.push(Line::styled(
                "No results found. Try a different title or artist.",
                Style::default().fg(theme::TEXT_MUTED),
            ));
            if app.search_focused() {
                focused_search_hint(&mut lines);
            }
        }
        SearchStatus::Ready => {
            for (index, track) in app.search().results.iter().enumerate() {
                hits.add(
                    MouseTarget::ContentTrack(index),
                    Rect::new(
                        rows[1].x.saturating_add(2),
                        rows[1].y.saturating_add(1 + index as u16),
                        rows[1].width.saturating_sub(4),
                        1,
                    ),
                );
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
                if app.search_focused() {
                    "Esc unfocus · ↑/↓ select · Enter play · a add to queue"
                } else {
                    "↑/↓ select · Enter play · a add to queue"
                },
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

fn focused_search_hint(lines: &mut Vec<Line<'static>>) {
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Esc unfocus",
        Style::default().fg(theme::TEXT_MUTED),
    ));
}

fn render_settings(frame: &mut Frame, area: Rect, app: &App) {
    let content = Paragraph::new(vec![
        Line::styled(
            "Lightweight Crossfade Engine",
            Style::default().fg(theme::TEXT),
        ),
        Line::styled(
            format!(
                "[ / ]  {} seconds{}",
                app.settings().crossfade_seconds,
                if app.settings().crossfade_seconds == 0 {
                    " · disabled"
                } else {
                    ""
                }
            ),
            Style::default().fg(theme::ACCENT),
        ),
        Line::styled(
            "Equal-power dual-deck transition · range 0–12 seconds",
            Style::default().fg(theme::TEXT_MUTED),
        ),
        Line::raw(""),
        Line::styled("Native DJ Engine", Style::default().fg(theme::TEXT)),
        Line::styled(
            "Android app only · not included in the desktop CLI",
            Style::default().fg(theme::ACCENT),
        ),
    ])
    .block(card().title(" Audio "));
    frame.render_widget(content, area.inner(ratatui::layout::Margin::new(2, 2)));
}

fn section_title(title: &str) -> Line<'_> {
    Line::styled(
        title,
        Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD),
    )
}

fn empty_line(message: &str) -> Line<'_> {
    Line::styled(message, Style::default().fg(theme::TEXT_MUTED))
}

fn track_line(track: &zerobeat_core::Track, selected: bool) -> Line<'static> {
    let marker = if selected { "▶" } else { " " };
    let style = if selected {
        Style::default()
            .fg(theme::TEXT)
            .bg(theme::SURFACE_HIGH)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::TEXT_MUTED)
    };
    Line::styled(
        format!("{marker}  {}  ·  {}", track.title, track.artist),
        style,
    )
}

fn card<'a>() -> Block<'a> {
    Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::SURFACE_HIGH))
        .padding(ratatui::widgets::Padding::horizontal(1))
}
