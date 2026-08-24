use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use zerobeat_core::Route;

use crate::{App, HitMap, MouseTarget, theme};

use super::{
    content::{render_content, render_queue},
    player::render_player,
};

const WIDE_LAYOUT_MIN_COLUMNS: u16 = 90;
const QUEUE_SIDEBAR_MIN_COLUMNS: u16 = 118;

pub fn render(frame: &mut Frame, app: &App) -> HitMap {
    let mut hits = HitMap::default();
    frame.render_widget(
        Block::new().style(Style::default().bg(theme::BACKGROUND)),
        frame.area(),
    );
    let rows = Layout::vertical([Constraint::Min(10), Constraint::Length(9)]).split(frame.area());

    if rows[0].width >= WIDE_LAYOUT_MIN_COLUMNS {
        render_wide(frame, rows[0], app, &mut hits);
    } else {
        render_compact(frame, rows[0], app, &mut hits);
    }
    render_player(frame, rows[1], app, &mut hits);
    hits
}

fn render_wide(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    if area.width >= QUEUE_SIDEBAR_MIN_COLUMNS && !app.queue_focused() {
        let columns = Layout::horizontal([
            Constraint::Length(28),
            Constraint::Min(44),
            Constraint::Length(30),
        ])
        .split(area);
        render_sidebar(frame, columns[0], app, hits);
        render_content(frame, columns[1], app, hits);
        render_queue(frame, columns[2], app, false, hits);
        return;
    }
    let columns = Layout::horizontal([Constraint::Length(28), Constraint::Min(30)]).split(area);
    render_sidebar(frame, columns[0], app, hits);
    render_content(frame, columns[1], app, hits);
}

fn render_compact(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    let rows = Layout::vertical([Constraint::Length(3), Constraint::Min(7)]).split(area);
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                " ZeroBeat ",
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Guest · Local", Style::default().fg(theme::ACCENT)),
        ]),
        Line::from(" Home  Search  Library  Downloads  Settings ")
            .style(Style::default().fg(theme::TEXT_MUTED)),
    ])
    .style(Style::default().bg(theme::SURFACE));
    frame.render_widget(header, rows[0]);
    let nav_width = rows[0].width / 5;
    for (index, route) in [
        Route::Home,
        Route::Search,
        Route::Library,
        Route::Downloads,
        Route::Settings,
    ]
    .into_iter()
    .enumerate()
    {
        hits.add(
            MouseTarget::Navigation(route),
            Rect::new(
                rows[0].x.saturating_add(nav_width * index as u16),
                rows[0].y.saturating_add(1),
                nav_width,
                2,
            ),
        );
    }
    render_content(frame, rows[1], app, hits);
}

fn render_sidebar(frame: &mut Frame, area: Rect, app: &App, hits: &mut HitMap) {
    let mut lines = vec![
        Line::styled(
            "   ZeroBeat",
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled("   Guest · Local", Style::default().fg(theme::ACCENT)),
        Line::raw(""),
        section("YOUR MUSIC"),
    ];
    let mut targets = Vec::new();
    push_nav(
        &mut lines,
        &mut targets,
        app,
        Route::Library,
        1,
        "Liked Songs",
        Some(app.library().liked.len()),
    );
    targets.push((lines.len(), MouseTarget::Navigation(Route::Home)));
    lines.push(menu_line(
        2,
        "Recently Played",
        false,
        Some(app.library().recent.len()),
    ));
    push_nav(
        &mut lines,
        &mut targets,
        app,
        Route::Downloads,
        3,
        "Downloads",
        Some(app.library().downloads.len()),
    );
    lines.push(Line::raw(""));
    lines.push(section("DISCOVER"));
    push_nav(&mut lines, &mut targets, app, Route::Home, 4, "Home", None);
    push_nav(
        &mut lines,
        &mut targets,
        app,
        Route::Search,
        5,
        "Search",
        None,
    );
    lines.push(Line::raw(""));
    lines.push(section("PLAYBACK"));
    targets.push((lines.len(), MouseTarget::Queue));
    lines.push(menu_line(
        6,
        "Queue",
        app.queue_focused(),
        Some(app.playback().queue.len()),
    ));
    targets.push((lines.len(), MouseTarget::Lyrics));
    lines.push(menu_line(7, "Lyrics", app.lyrics().visible, None));
    lines.push(Line::raw(""));
    lines.push(section("SYSTEM"));
    push_nav(
        &mut lines,
        &mut targets,
        app,
        Route::Settings,
        8,
        "Settings",
        None,
    );

    for (line, target) in targets {
        hits.add(
            target,
            Rect::new(
                area.x,
                area.y.saturating_add(line as u16),
                area.width.saturating_sub(1),
                1,
            ),
        );
    }

    let sidebar = Paragraph::new(lines)
        .block(
            Block::new()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(theme::BORDER)),
        )
        .style(Style::default().bg(theme::SURFACE));
    frame.render_widget(sidebar, area);
    if area.height >= 6 {
        let status = Rect::new(
            area.x.saturating_add(3),
            area.bottom().saturating_sub(4),
            area.width.saturating_sub(5),
            3,
        );
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("Guest session", Style::default().fg(theme::ACCENT)),
                Line::styled(
                    "Native audio 48 kHz",
                    Style::default().fg(theme::TEXT_MUTED),
                ),
                Line::styled(
                    if app.playback().underrun_count == 0 {
                        "No underruns".to_owned()
                    } else {
                        format!("{} underruns", app.playback().underrun_count)
                    },
                    Style::default().fg(theme::TEXT_MUTED),
                ),
            ]),
            status,
        );
    }
}

fn push_nav(
    lines: &mut Vec<Line<'static>>,
    targets: &mut Vec<(usize, MouseTarget)>,
    app: &App,
    route: Route,
    shortcut: u8,
    label: &str,
    count: Option<usize>,
) {
    targets.push((lines.len(), MouseTarget::Navigation(route)));
    lines.push(menu_line(shortcut, label, app.route() == route, count));
}

fn menu_line(shortcut: u8, label: &str, active: bool, count: Option<usize>) -> Line<'static> {
    let marker = if active { "│" } else { " " };
    let count = count.map(|value| format!("  {value}")).unwrap_or_default();
    Line::styled(
        format!("{marker}  {shortcut}  {label}{count}"),
        Style::default()
            .fg(if active {
                theme::TEXT
            } else {
                theme::TEXT_MUTED
            })
            .add_modifier(if active {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
    )
}

fn section(label: &str) -> Line<'static> {
    Line::styled(format!("   ─ {label}"), Style::default().fg(theme::BORDER))
}
