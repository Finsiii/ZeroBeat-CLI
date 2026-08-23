use std::{
    io::{self, Stdout},
    time::Duration,
};

use crossterm::{
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use zerobeat_protocol::ClientCommand;
use zerobeat_runtime::socket_path;
use zerobeat_tui::{App, connect_or_spawn, render};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = connect_or_spawn(&socket_path()).await?;
    let mut app = App::new(client.snapshot().clone());
    let mut terminal = TerminalSession::enter()?;

    while !app.should_quit() {
        terminal.draw(&app)?;
        if !event::poll(Duration::from_millis(100))? {
            if app.needs_refresh() {
                let snapshot = client.execute(ClientCommand::RequestSnapshot).await?;
                app.replace_snapshot(snapshot);
            }
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if let Some(command) = app.handle_key(key) {
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
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }

    fn draw(&mut self, app: &App) -> io::Result<()> {
        self.terminal.draw(|frame| render(frame, app)).map(|_| ())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}
