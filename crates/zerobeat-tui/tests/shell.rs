use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use zerobeat_core::{Route, Track};
use zerobeat_protocol::{
    AppSnapshot, ClientCommand, PlaybackSnapshot, PlaybackStatus, SearchSnapshot, SearchStatus,
};
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

#[test]
fn enter_submits_search_and_results_can_be_selected() {
    let mut app = App::default();
    app.handle_key(key(KeyCode::Char('/')));
    for character in "tampar".chars() {
        app.handle_key(key(KeyCode::Char(character)));
    }

    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        Some(ClientCommand::SubmitSearch)
    );

    let mut snapshot = AppSnapshot::default();
    snapshot.navigation.open(Route::Search);
    snapshot.navigation.update_search("tampar");
    snapshot.search = SearchSnapshot {
        status: SearchStatus::Ready,
        results: vec![
            Track::new("one", "Tampar", "Juicy Luicy", 245_000),
            Track::new("two", "Lantas", "Juicy Luicy", 234_000),
        ],
        selected_index: 0,
        request_id: 1,
    };
    app.replace_snapshot(snapshot);

    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        Some(ClientCommand::PlaySelected)
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Down)),
        Some(ClientCommand::SelectNext)
    );
    let screen = render_screen(100, 30, &app);
    assert!(screen.contains("Tampar"));
    assert!(screen.contains("Juicy Luicy"));
}

#[test]
fn player_bar_shows_current_track_and_transport_state() {
    let snapshot = AppSnapshot {
        playback: PlaybackSnapshot {
            status: PlaybackStatus::Playing,
            current: Some(Track::new("one", "Tampar", "Juicy Luicy", 203_000)),
            position_ms: 61_000,
            duration_ms: 203_000,
            buffered_ms: 90_000,
            volume_percent: 75,
            error: None,
            request_id: 1,
        },
        ..AppSnapshot::default()
    };

    let screen = render_screen(100, 28, &App::new(snapshot));

    assert!(screen.contains("Tampar"));
    assert!(screen.contains("Juicy Luicy"));
    assert!(screen.contains("01:01 / 03:23"));
    assert!(screen.contains("75%"));
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
