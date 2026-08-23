use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use zerobeat_core::Route;
use zerobeat_protocol::ClientCommand;
use zerobeat_tui::{App, render};

#[test]
fn wide_layout_has_sidebar_home_content_and_persistent_player() {
    let screen = render_screen(120, 34, &App::default());

    assert!(screen.contains("ZeroBeat"));
    assert!(screen.contains("Guest · Local"));
    assert!(screen.contains("Home"));
    assert!(screen.contains("Continue listening"));
    assert!(screen.contains("Nothing playing"));
}

#[test]
fn compact_layout_uses_top_navigation_without_losing_player() {
    let screen = render_screen(72, 24, &App::default());

    assert!(screen.contains("Home  Search  Library"));
    assert!(screen.contains("Guest · Local"));
    assert!(screen.contains("Nothing playing"));
}

#[test]
fn slash_focuses_search_and_query_survives_navigation() {
    let mut app = App::default();

    app.handle_key(key(KeyCode::Char('/')));
    for character in "tampar".chars() {
        app.handle_key(key(KeyCode::Char(character)));
    }
    app.handle_key(key(KeyCode::Esc));
    app.open(Route::Home);
    app.open(Route::Search);

    assert_eq!(app.route(), Route::Search);
    assert_eq!(app.search_query(), "tampar");
    assert!(!app.search_focused());
}

#[test]
fn navigation_and_search_keys_produce_daemon_commands() {
    let mut app = App::default();

    assert_eq!(
        app.handle_key(key(KeyCode::Char('2'))),
        Some(ClientCommand::Navigate(Route::Search))
    );
    app.handle_key(key(KeyCode::Char('/')));
    assert_eq!(
        app.handle_key(key(KeyCode::Char('a'))),
        Some(ClientCommand::UpdateSearch("a".to_owned()))
    );
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.handle_key(key(KeyCode::Esc)), Some(ClientCommand::Back));
}

fn render_screen(width: u16, height: u16, app: &App) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal.draw(|frame| render(frame, app)).expect("draw");

    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<Vec<_>>()
        .chunks(width as usize)
        .map(|row| row.concat())
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
