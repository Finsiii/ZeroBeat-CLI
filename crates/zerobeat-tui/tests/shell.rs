use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use zerobeat_core::{Route, Track};
use zerobeat_protocol::{
    AppSnapshot, ClientCommand, DownloadSnapshot, DownloadStatus, LyricsLineSnapshot, LyricsStatus,
    PlaybackSnapshot, PlaybackStatus, SearchSnapshot, SearchStatus,
};
use zerobeat_tui::{App, MouseTarget, render};

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
        app.handle_key(key(KeyCode::Char('5'))),
        Some(ClientCommand::Navigate(Route::Search))
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Char('a'))),
        Some(ClientCommand::UpdateSearch("a".to_owned()))
    );
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.handle_key(key(KeyCode::Esc)), Some(ClientCommand::Back));
}

#[test]
fn submitted_search_releases_space_for_global_playback() {
    let mut app = App::default();
    app.handle_key(key(KeyCode::Char('/')));
    for character in "juicy luicy".chars() {
        app.handle_key(key(KeyCode::Char(character)));
    }

    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        Some(ClientCommand::SubmitSearch)
    );
    assert!(!app.search_focused());
    assert_eq!(
        app.handle_key(key(KeyCode::Char(' '))),
        Some(ClientCommand::TogglePlayback)
    );
}

#[test]
fn navigation_closes_local_overlays_and_stale_search_focus() {
    let mut app = App::default();
    app.handle_key(key(KeyCode::Char('u')));
    assert!(app.queue_focused());

    assert_eq!(
        app.handle_key(key(KeyCode::Char('4'))),
        Some(ClientCommand::Navigate(Route::Home))
    );
    assert!(!app.queue_focused());

    app.handle_key(key(KeyCode::Char('/')));
    assert!(app.search_focused());
    let (_, hits) = render_screen_with_hits(140, 38, &app);
    let library = hits
        .region(MouseTarget::Navigation(Route::Library))
        .unwrap();
    assert_eq!(
        app.handle_mouse(click(library.x, library.y), &hits),
        Some(ClientCommand::Navigate(Route::Library))
    );
    assert!(!app.search_focused());
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
            ..PlaybackSnapshot::default()
        },
        ..AppSnapshot::default()
    };

    let screen = render_screen(140, 28, &App::new(snapshot));

    assert!(screen.contains("Tampar"));
    assert!(screen.contains("Juicy Luicy"));
    assert!(screen.contains("01:01"));
    assert!(screen.contains("-02:22"));
    assert!(screen.contains("(s) Shuffle"));
    assert!(screen.contains("(r) Repeat Off"));
    assert!(screen.contains("(Space) Pause"));
    assert!(screen.contains("75%"));
    assert!(!screen.contains("click controls"));
}

#[test]
fn studio_deck_renders_real_spectrum_and_library_first_sidebar_without_capsules() {
    let mut snapshot = AppSnapshot::default();
    snapshot.playback.status = PlaybackStatus::Playing;
    snapshot.playback.current = Some(Track::new("one", "Tampar", "Juicy Luicy", 203_000));
    snapshot.playback.duration_ms = 203_000;
    snapshot.playback.spectrum = [
        4, 8, 12, 24, 48, 72, 90, 64, 42, 30, 18, 12, 9, 7, 5, 4, 3, 2, 2, 1, 1, 0, 0, 0,
    ];
    snapshot.library.liked = vec![Track::new("liked", "Favorite", "Artist", 100_000)];
    snapshot.library.recent = vec![Track::new("recent", "Recent", "Artist", 100_000)];

    let (screen, hits) = render_screen_with_hits(140, 38, &App::new(snapshot));

    for expected in [
        "DISCOVER",
        "YOUR MUSIC",
        "Liked Songs",
        "Recently Played",
        "PLAYBACK",
        "Queue",
        "Lyrics",
        "Native audio 48 kHz",
    ] {
        assert!(screen.contains(expected), "missing {expected:?}");
    }
    assert!(screen.contains('█') || screen.contains('▆'));
    assert!(!screen.contains("▁ ▁"));
    assert!(!screen.contains("ONLINE"));
    assert!(!screen.contains("[Online]"));
    for target in [
        MouseTarget::Progress,
        MouseTarget::Shuffle,
        MouseTarget::Previous,
        MouseTarget::PlayPause,
        MouseTarget::Next,
        MouseTarget::Repeat,
        MouseTarget::Like,
        MouseTarget::Lyrics,
        MouseTarget::Queue,
        MouseTarget::Mute,
    ] {
        assert!(hits.region(target).is_some(), "missing {target:?}");
    }
}

#[test]
fn sidebar_numbers_match_keyboard_navigation_and_sections_are_dividers() {
    let screen = render_screen(140, 38, &App::default());

    for expected in [
        "1  Liked Songs",
        "2  Recently Played",
        "3  Downloads",
        "4  Home",
        "5  Search",
        "6  Queue",
        "7  Lyrics",
        "8  Settings",
        "─ DISCOVER",
    ] {
        assert!(screen.contains(expected), "missing {expected:?}");
    }

    let mut app = App::default();
    assert_eq!(
        app.handle_key(key(KeyCode::Char('1'))),
        Some(ClientCommand::Navigate(Route::Library))
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Char('3'))),
        Some(ClientCommand::Navigate(Route::Downloads))
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Char('8'))),
        Some(ClientCommand::Navigate(Route::Settings))
    );
}

#[test]
fn paused_visualizer_is_a_continuous_line_instead_of_dotted_blocks() {
    let mut snapshot = AppSnapshot::default();
    snapshot.playback.status = PlaybackStatus::Paused;
    snapshot.playback.current = Some(Track::new("one", "Tampar", "Juicy Luicy", 203_000));

    let screen = render_screen(140, 28, &App::new(snapshot));

    assert!(screen.contains("────────"));
    assert!(!screen.contains("▁ ▁"));
}

#[test]
fn spectrum_smoothing_attacks_releases_and_resets() {
    let track = Track::new("one", "Tampar", "Juicy Luicy", 203_000);
    let mut snapshot = AppSnapshot::default();
    snapshot.playback.status = PlaybackStatus::Playing;
    snapshot.playback.current = Some(track.clone());
    let mut app = App::new(snapshot.clone());

    snapshot.playback.spectrum = [100; 24];
    app.replace_snapshot(snapshot.clone());
    assert_eq!(app.spectrum()[0], 75);

    snapshot.playback.spectrum = [0; 24];
    app.replace_snapshot(snapshot.clone());
    assert_eq!(app.spectrum()[0], 60);

    snapshot.playback.status = PlaybackStatus::Paused;
    app.replace_snapshot(snapshot.clone());
    assert_eq!(app.spectrum(), &[0; 24]);

    snapshot.playback.status = PlaybackStatus::Idle;
    app.replace_snapshot(snapshot.clone());
    assert_eq!(app.spectrum(), &[0; 24]);

    snapshot.playback.status = PlaybackStatus::Playing;
    snapshot.playback.current = None;
    snapshot.playback.spectrum = [100; 24];
    app.replace_snapshot(snapshot);
    assert_eq!(app.spectrum(), &[0; 24]);
}

#[test]
fn transport_shortcuts_cover_shuffle_repeat_previous_mute_and_queue_cleanup() {
    let mut app = App::default();

    assert_eq!(
        app.handle_key(key(KeyCode::Char('s'))),
        Some(ClientCommand::ToggleShuffle)
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Char('r'))),
        Some(ClientCommand::CycleRepeat)
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Char('p'))),
        Some(ClientCommand::PreviousTrack)
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Char('m'))),
        Some(ClientCommand::ToggleMute)
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Char('x'))),
        Some(ClientCommand::ClearQueue)
    );
}

#[test]
fn mouse_targets_navigate_seek_play_and_control_transport() {
    let mut snapshot = AppSnapshot::default();
    snapshot.playback.status = PlaybackStatus::Playing;
    snapshot.playback.current = Some(Track::new("one", "Tampar", "Juicy Luicy", 200_000));
    snapshot.playback.duration_ms = 200_000;
    snapshot.playback.volume_percent = 75;
    snapshot.navigation.open(Route::Search);
    snapshot.search.status = SearchStatus::Ready;
    snapshot.search.results = vec![Track::new("two", "Sialan", "Juicy Luicy", 242_000)];
    let mut app = App::new(snapshot);
    let (_, hits) = render_screen_with_hits(140, 38, &app);

    let home = hits.region(MouseTarget::Navigation(Route::Home)).unwrap();
    assert_eq!(
        app.handle_mouse(click(home.x, home.y), &hits),
        Some(ClientCommand::Navigate(Route::Home))
    );

    app.open(Route::Search);
    let (_, hits) = render_screen_with_hits(140, 38, &app);
    let track = hits.region(MouseTarget::ContentTrack(0)).unwrap();
    assert_eq!(
        app.handle_mouse(click(track.x, track.y), &hits),
        Some(ClientCommand::PlayTrack(Track::new(
            "two",
            "Sialan",
            "Juicy Luicy",
            242_000
        )))
    );

    let progress = hits.region(MouseTarget::Progress).unwrap();
    assert_eq!(
        app.handle_mouse(click(progress.right() - 1, progress.y), &hits),
        Some(ClientCommand::SeekTo(200_000))
    );

    let shuffle = hits.region(MouseTarget::Shuffle).unwrap();
    assert_eq!(
        app.handle_mouse(click(shuffle.x, shuffle.y), &hits),
        Some(ClientCommand::ToggleShuffle)
    );
    let player = hits.region(MouseTarget::Player).unwrap();
    assert_eq!(
        app.handle_mouse(scroll_up(player.x, player.y), &hits),
        Some(ClientCommand::SetVolume(80))
    );
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
    render_screen_with_hits(width, height, app).0
}

fn render_screen_with_hits(width: u16, height: u16, app: &App) -> (String, zerobeat_tui::HitMap) {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut hits = None;
    terminal
        .draw(|frame| hits = Some(render(frame, app)))
        .expect("draw");

    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<Vec<_>>()
        .chunks(width as usize)
        .map(|row| row.concat())
        .collect::<Vec<_>>()
        .join("\n");
    (screen, hits.unwrap())
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn scroll_up(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}
