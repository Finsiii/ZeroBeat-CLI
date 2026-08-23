use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use zerobeat_core::Route;

use crate::{App, theme};

use super::{content::render_content, player::render_player};

const WIDE_LAYOUT_MIN_COLUMNS: u16 = 90;

pub fn render(frame: &mut Frame, app: &App) {
    frame.render_widget(
        Block::new().style(Style::default().bg(theme::BACKGROUND)),
        frame.area(),
    );
    let rows = Layout::vertical([Constraint::Min(10), Constraint::Length(4)]).split(frame.area());

    if rows[0].width >= WIDE_LAYOUT_MIN_COLUMNS {
        render_wide(frame, rows[0], app);
    } else {
        render_compact(frame, rows[0], app);
    }
    render_player(frame, rows[1]);
}

fn render_wide(frame: &mut Frame, area: Rect, app: &App) {
    let columns = Layout::horizontal([Constraint::Length(24), Constraint::Min(30)]).split(area);
    render_sidebar(frame, columns[0], app.route());
    render_content(frame, columns[1], app);
}

fn render_compact(frame: &mut Frame, area: Rect, app: &App) {
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
    render_content(frame, rows[1], app);
}

fn render_sidebar(frame: &mut Frame, area: Rect, active: Route) {
    let entries = [
        (Route::Home, "1  Home"),
        (Route::Search, "2  Search"),
        (Route::Library, "3  Library"),
        (Route::Downloads, "4  Downloads"),
        (Route::Settings, "5  Settings"),
    ];
    let mut lines = vec![
        Line::styled(
            "◉  ZeroBeat",
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled("   Guest · Local", Style::default().fg(theme::ACCENT)),
        Line::raw(""),
    ];
    lines.extend(entries.map(|(route, label)| {
        let marker = if active == route { "●" } else { " " };
        let color = if active == route {
            theme::TEXT
        } else {
            theme::TEXT_MUTED
        };
        Line::styled(format!("{marker}  {label}"), Style::default().fg(color))
    }));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "/  Quick search",
        Style::default().fg(theme::TEXT_MUTED),
    ));
    lines.push(Line::styled(
        "q  Quit",
        Style::default().fg(theme::TEXT_MUTED),
    ));

    let sidebar = Paragraph::new(lines)
        .block(
            Block::new()
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(theme::BORDER)),
        )
        .style(Style::default().bg(theme::SURFACE));
    frame.render_widget(sidebar, area);
}
