use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use zerobeat_core::{Route, Track};
use zerobeat_protocol::{
    AppSnapshot, ClientCommand, DownloadSnapshot, DownloadStatus, LyricsLineSnapshot, LyricsStatus,
    PlaybackSnapshot, PlaybackStatus, SearchSnapshot, SearchStatus,
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
    assert_eq!(
        app.handle_key(key(KeyCode::Char('a'))),
        Some(ClientCommand::QueueSelected)
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
            queue: Vec::new(),
        },
        ..AppSnapshot::default()
    };

    let screen = render_screen(100, 28, &App::new(snapshot));

    assert!(screen.contains("Tampar"));
    assert!(screen.contains("Juicy Luicy"));
    assert!(screen.contains("01:01 / 03:23"));
    assert!(screen.contains("75%"));
}

#[test]
fn guest_home_library_and_downloads_render_real_local_data() {
    let liked = Track::new("liked", "Favorite Song", "ZeroBeat", 180_000);
    let recent = Track::new("recent", "Recently Played", "ZeroBeat", 190_000);
    let offline = Track::new("offline", "Offline Song", "ZeroBeat", 200_000);
    let mut snapshot = AppSnapshot::default();
    snapshot.library.liked = vec![liked.clone()];
    snapshot.library.recent = vec![recent.clone()];
    snapshot.library.downloads = vec![DownloadSnapshot {
        track: offline.clone(),
        status: DownloadStatus::Available,
        error: None,
    }];
    let mut app = App::new(snapshot);

    assert!(render_screen(110, 32, &app).contains("Recently Played"));
    app.open(Route::Library);
    let library = render_screen(110, 32, &app);
    assert!(library.contains("Favorite Song"));
    assert!(library.contains("Recently Played"));
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        Some(ClientCommand::PlayTrack(liked.clone()))
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Char('l'))),
        Some(ClientCommand::ToggleLike(liked))
    );

    app.open(Route::Downloads);
    let downloads = render_screen(110, 32, &app);
    assert!(downloads.contains("Offline Song"));
    assert!(downloads.contains("Available offline"));
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        Some(ClientCommand::PlayTrack(offline))
    );
}

#[test]
fn search_result_can_be_liked_or_downloaded() {
    let song = Track::new("one", "Tampar", "Juicy Luicy", 245_000);
    let mut snapshot = AppSnapshot::default();
    snapshot.navigation.open(Route::Search);
    snapshot.search.status = SearchStatus::Ready;
    snapshot.search.results = vec![song.clone()];
    let mut app = App::new(snapshot);

    assert_eq!(
        app.handle_key(key(KeyCode::Char('l'))),
        Some(ClientCommand::ToggleLike(song.clone()))
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Char('d'))),
        Some(ClientCommand::DownloadTrack(song))
    );
}

#[test]
fn lyrics_view_highlights_the_current_synced_line() {
    let mut snapshot = AppSnapshot::default();
    snapshot.playback.current = Some(Track::new("one", "Tampar", "Juicy Luicy", 245_000));
    snapshot.playback.position_ms = 6_000;
    snapshot.lyrics.visible = true;
    snapshot.lyrics.status = LyricsStatus::Ready;
    snapshot.lyrics.synced = true;
    snapshot.lyrics.lines = vec![
        LyricsLineSnapshot {
            start_ms: Some(1_000),
            words: "Entah sudah selasa".into(),
        },
        LyricsLineSnapshot {
            start_ms: Some(5_000),
            words: "Masih saja kau ada".into(),
        },
    ];
    let mut app = App::new(snapshot);

    let screen = render_screen(100, 30, &app);
    assert!(screen.contains("Lyrics · Synced"));
    assert!(screen.contains("▶  Masih saja kau ada"));
    assert_eq!(
        app.handle_key(key(KeyCode::Char('y'))),
        Some(ClientCommand::ToggleLyrics)
    );
}

#[test]
fn settings_control_persistent_crossfade_duration() {
    let mut snapshot = AppSnapshot::default();
    snapshot.navigation.open(Route::Settings);
    snapshot.settings.crossfade_seconds = 6;
    let mut app = App::new(snapshot);

    let screen = render_screen(100, 30, &app);
    assert!(screen.contains("6 seconds"));
    assert!(screen.contains("Android app only"));
    assert_eq!(
        app.handle_key(key(KeyCode::Char(']'))),
        Some(ClientCommand::SetCrossfadeSeconds(7))
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Char('['))),
        Some(ClientCommand::SetCrossfadeSeconds(5))
    );
}

#[test]
fn queue_is_visible_on_wide_layout_and_toggleable_as_a_focus_view() {
    let mut snapshot = AppSnapshot::default();
    snapshot.playback.queue = vec![
        Track::new("two", "Sialan", "Juicy Luicy", 242_000),
        Track::new("three", "Lampu Kuning", "Juicy Luicy", 240_000),
    ];
    let mut app = App::new(snapshot);

    let wide = render_screen(130, 32, &app);
    assert!(wide.contains("Up next"));
    assert!(wide.contains("Sialan"));
    assert_eq!(app.handle_key(key(KeyCode::Char('u'))), None);
    let focused = render_screen(80, 26, &app);
    assert!(focused.contains("Queue · 2"));
    assert!(focused.contains("Lampu Kuning"));
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
