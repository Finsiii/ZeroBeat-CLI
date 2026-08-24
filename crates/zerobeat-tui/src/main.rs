use std::{
    io::{self, Stdout},
    time::Duration,
};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use zerobeat_protocol::{ClientCommand, PROTOCOL_VERSION};
use zerobeat_runtime::socket_path;
use zerobeat_tui::{App, HitMap, connect_or_spawn, render};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = connect_or_spawn(&socket_path(PROTOCOL_VERSION)).await?;
    let mut app = App::new(client.snapshot().clone());
    let mut terminal = TerminalSession::enter()?;

    while !app.should_quit() {
        let hits = terminal.draw(&app)?;
        if !event::poll(Duration::from_millis(100))? {
            if app.needs_refresh() {
                let snapshot = client.execute(ClientCommand::RequestSnapshot).await?;
                app.replace_snapshot(snapshot);
            }
            continue;
        }
        let command = match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
            Event::Mouse(mouse) => app.handle_mouse(mouse, &hits),
            _ => None,
        };
        if let Some(command) = command {
            let snapshot = client.execute(command).await?;
            app.replace_snapshot(snapshot);
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
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
            let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(error);
        }
        let terminal = match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => terminal,
            Err(error) => {
                let mut stdout = io::stdout();
                let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
                let _ = disable_raw_mode();
                return Err(error);
            }
        };
        Ok(Self { terminal })
    }

    fn draw(&mut self, app: &App) -> io::Result<HitMap> {
        let mut hits = None;
        self.terminal
            .draw(|frame| hits = Some(render(frame, app)))?;
        Ok(hits.unwrap_or_default())
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
