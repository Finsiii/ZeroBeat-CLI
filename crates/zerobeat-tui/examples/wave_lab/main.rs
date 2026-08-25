mod app;
mod styles;

use std::{
    io::{self, Stdout},
    time::{Duration, Instant},
};

use app::Gallery;
use crossterm::{
    event::{self, DisableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use styles::{WaveStyle, render, synthetic_signal};

const BACKGROUND: Color = Color::Rgb(8, 9, 12);
const SURFACE: Color = Color::Rgb(17, 19, 24);
const TEXT: Color = Color::Rgb(235, 239, 246);
const MUTED: Color = Color::Rgb(132, 140, 154);
const BORDER: Color = Color::Rgb(45, 51, 62);
const ACCENT: Color = Color::Rgb(102, 225, 176);
const FRAME_TIME: Duration = Duration::from_millis(65);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut terminal = TerminalSession::enter()?;
    let mut gallery = Gallery::default();
    let mut last_frame = Instant::now();

    while !gallery.should_quit() {
        terminal.draw(&gallery)?;
        let timeout = FRAME_TIME.saturating_sub(last_frame.elapsed());
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            gallery.handle_key(key.code);
        }
        if last_frame.elapsed() >= FRAME_TIME {
            gallery.advance();
            last_frame = Instant::now();
        }
    }

    Ok(())
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                let mut stdout = io::stdout();
                let _ = execute!(stdout, LeaveAlternateScreen);
                let _ = disable_raw_mode();
                Err(error)
            }
        }
    }

    fn draw(&mut self, gallery: &Gallery) -> io::Result<()> {
        self.terminal.draw(|frame| draw(frame, gallery)).map(|_| ())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

fn draw(frame: &mut Frame, gallery: &Gallery) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(Style::default().bg(BACKGROUND)),
        area,
    );

    if area.width < 54 || area.height < 16 {
        frame.render_widget(
            Paragraph::new("Wave Lab needs at least 54 × 16 cells")
                .style(Style::default().fg(TEXT)),
            area,
        );
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(area);
    draw_header(frame, layout[0], gallery);
    if gallery.expanded() || area.width < 100 || area.height < 30 {
        draw_focus(frame, layout[1], gallery);
    } else {
        draw_grid(frame, layout[1], gallery);
    }
    draw_footer(frame, layout[2], gallery);
}

fn draw_header(frame: &mut Frame, area: Rect, gallery: &Gallery) {
    let state = if gallery.paused() {
        "Paused"
    } else {
        "Playing"
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(
                "ZeroBeat Wave Lab",
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ·  terminal renderer gallery", Style::default().fg(MUTED)),
        ]),
        Line::from(Span::styled(
            format!(
                "{}  ·  intensity {}%  ·  synthetic motion preview",
                state,
                gallery.intensity()
            ),
            Style::default().fg(ACCENT),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(BACKGROUND)),
        area,
    );
}

fn draw_grid(frame: &mut Frame, area: Rect, gallery: &Gallery) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .spacing(1)
        .split(area);
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 3); 3])
        .split(columns[0]);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Ratio(1, 3); 3])
        .split(columns[1]);

    for (index, style) in WaveStyle::ALL.into_iter().enumerate() {
        let target = if index < 3 {
            left[index]
        } else {
            right[index - 3]
        };
        draw_card(frame, target, gallery, index, style);
    }
}

fn draw_focus(frame: &mut Frame, area: Rect, gallery: &Gallery) {
    draw_card(
        frame,
        area,
        gallery,
        gallery.selected(),
        gallery.selected_style(),
    );
}

fn draw_card(frame: &mut Frame, area: Rect, gallery: &Gallery, index: usize, style: WaveStyle) {
    let selected = gallery.selected() == index;
    let border = if selected { ACCENT } else { BORDER };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .style(Style::default().bg(SURFACE))
        .title(Line::from(vec![
            Span::styled(
                format!(" {} ", index + 1),
                Style::default()
                    .fg(if selected { ACCENT } else { MUTED })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(style.name(), Style::default().fg(TEXT)),
            Span::raw(" "),
        ]));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(inner);
    frame.render_widget(
        Paragraph::new(style.description()).style(Style::default().fg(MUTED).bg(SURFACE)),
        rows[0],
    );

    let signal = synthetic_signal(gallery.tick(), 100);
    let wave = render(
        style,
        &signal,
        usize::from(rows[1].width),
        usize::from(rows[1].height),
        gallery.intensity(),
    );
    let wave_lines = wave
        .into_iter()
        .map(|line| {
            Line::styled(
                line,
                Style::default().fg(if selected { ACCENT } else { MUTED }),
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(wave_lines).style(Style::default().bg(SURFACE)),
        rows[1],
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, gallery: &Gallery) {
    let mode = if gallery.expanded() {
        "Esc back"
    } else {
        "Enter expand"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("1–6", Style::default().fg(ACCENT)),
            Span::styled(" choose  ·  ", Style::default().fg(MUTED)),
            Span::styled("←/→", Style::default().fg(ACCENT)),
            Span::styled(" browse  ·  ", Style::default().fg(MUTED)),
            Span::styled(mode, Style::default().fg(TEXT)),
            Span::styled(
                "  ·  Space freeze  ·  +/− intensity  ·  q quit",
                Style::default().fg(MUTED),
            ),
        ]))
        .style(Style::default().bg(BACKGROUND)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn wide_layout_shows_all_six_renderers() {
        let screen = render_screen(140, 40, &Gallery::default());

        for style in WaveStyle::ALL {
            assert!(screen.contains(style.name()));
        }
    }

    #[test]
    fn narrow_layout_shows_only_the_selected_renderer() {
        let mut gallery = Gallery::default();
        gallery.handle_key(crossterm::event::KeyCode::Char('5'));
        let screen = render_screen(80, 24, &gallery);

        assert!(screen.contains("Twin Rail"));
        assert!(!screen.contains("Thin Braille"));
        assert!(!screen.contains("Oscilloscope"));
    }

    #[test]
    fn expanded_layout_keeps_the_selected_renderer_and_back_hint() {
        let mut gallery = Gallery::default();
        gallery.handle_key(crossterm::event::KeyCode::Char('6'));
        gallery.handle_key(crossterm::event::KeyCode::Enter);
        let screen = render_screen(140, 40, &gallery);

        assert!(screen.contains("Oscilloscope"));
        assert!(screen.contains("Esc back"));
        assert!(!screen.contains("Thin Braille"));
    }

    fn render_screen(width: u16, height: u16, gallery: &Gallery) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| draw(frame, gallery)).expect("draw");

        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .chunks(usize::from(width))
            .map(|row| row.concat())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
