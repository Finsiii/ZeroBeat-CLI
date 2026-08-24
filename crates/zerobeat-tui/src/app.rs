use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use zerobeat_core::Route;
use zerobeat_protocol::{
    AppSnapshot, ClientCommand, PlaybackSnapshot, PlaybackStatus, SearchSnapshot, SearchStatus,
};

#[derive(Default)]
pub struct App {
    snapshot: AppSnapshot,
    search_focused: bool,
    should_quit: bool,
    home_selected: usize,
    library_selected: usize,
    downloads_selected: usize,
    queue_focused: bool,
    queue_selected: usize,
}

impl App {
    pub fn new(snapshot: AppSnapshot) -> Self {
        Self {
            snapshot,
            ..Self::default()
        }
    }

    pub fn replace_snapshot(&mut self, snapshot: AppSnapshot) {
        self.queue_selected = self
            .queue_selected
            .min(snapshot.playback.queue.len().saturating_sub(1));
        self.snapshot = snapshot;
    }

    pub fn route(&self) -> Route {
        self.snapshot.navigation.active_route()
    }

    pub fn search_query(&self) -> &str {
        self.snapshot.navigation.search_query()
    }

    pub fn search_focused(&self) -> bool {
        self.search_focused
    }

    pub fn search(&self) -> &SearchSnapshot {
        &self.snapshot.search
    }

    pub fn playback(&self) -> &PlaybackSnapshot {
        &self.snapshot.playback
    }

    pub fn library(&self) -> &zerobeat_protocol::LibrarySnapshot {
        &self.snapshot.library
    }

    pub fn lyrics(&self) -> &zerobeat_protocol::LyricsSnapshot {
        &self.snapshot.lyrics
    }

    pub fn settings(&self) -> &zerobeat_protocol::SettingsSnapshot {
        &self.snapshot.settings
    }

    pub fn queue_focused(&self) -> bool {
        self.queue_focused
    }

    pub fn queue_selected(&self) -> usize {
        self.queue_selected
    }

    pub fn selected_index(&self) -> usize {
        match self.route() {
            Route::Home => self.home_selected,
            Route::Search => self.search().selected_index,
            Route::Library => self.library_selected,
            Route::Downloads => self.downloads_selected,
            Route::Settings => 0,
        }
    }

    pub fn needs_refresh(&self) -> bool {
        self.search().status == SearchStatus::Loading
            || matches!(
                self.playback().status,
                PlaybackStatus::Resolving | PlaybackStatus::Buffering | PlaybackStatus::Playing
            )
            || self.snapshot.library.downloads.iter().any(|download| {
                matches!(
                    download.status,
                    zerobeat_protocol::DownloadStatus::Queued
                        | zerobeat_protocol::DownloadStatus::Downloading
                )
            })
            || self.lyrics().status == zerobeat_protocol::LyricsStatus::Loading
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn open(&mut self, route: Route) {
        self.snapshot.navigation.open(route);
    }

    pub fn handle_key(&mut self, event: KeyEvent) -> Option<ClientCommand> {
        if self.search_focused {
            return self.handle_search_key(event.code);
        }

        match event.code {
            KeyCode::Char('/') => {
                self.open(Route::Search);
                self.search_focused = true;
                Some(ClientCommand::Navigate(Route::Search))
            }
            KeyCode::Char('1') => self.navigate(Route::Home),
            KeyCode::Char('2') => {
                self.search_focused = true;
                self.navigate(Route::Search)
            }
            KeyCode::Char('3') => self.navigate(Route::Library),
            KeyCode::Char('4') => self.navigate(Route::Downloads),
            KeyCode::Char('5') => self.navigate(Route::Settings),
            KeyCode::Char('q') => {
                self.should_quit = true;
                None
            }
            KeyCode::Esc => {
                self.snapshot.navigation.back();
                Some(ClientCommand::Back)
            }
            KeyCode::Enter if self.queue_focused && !self.playback().queue.is_empty() => {
                Some(ClientCommand::PlayQueueIndex(self.queue_selected))
            }
            KeyCode::Enter
                if self.route() == Route::Search && !self.search().results.is_empty() =>
            {
                Some(ClientCommand::PlaySelected)
            }
            KeyCode::Enter => self.selected_track().map(ClientCommand::PlayTrack),
            KeyCode::Char('a')
                if self.route() == Route::Search && !self.search().results.is_empty() =>
            {
                Some(ClientCommand::QueueSelected)
            }
            KeyCode::Char('a') => self.selected_track().map(ClientCommand::QueueTrack),
            KeyCode::Char('l') => self.selected_track().map(ClientCommand::ToggleLike),
            KeyCode::Char('d') => self.selected_track().map(ClientCommand::DownloadTrack),
            KeyCode::Char('y') => Some(ClientCommand::ToggleLyrics),
            KeyCode::Char('u') => {
                self.queue_focused = !self.queue_focused;
                None
            }
            KeyCode::Char('[') if self.route() == Route::Settings => {
                Some(ClientCommand::SetCrossfadeSeconds(
                    self.settings().crossfade_seconds.saturating_sub(1),
                ))
            }
            KeyCode::Char(']') if self.route() == Route::Settings => {
                Some(ClientCommand::SetCrossfadeSeconds(
                    self.settings().crossfade_seconds.saturating_add(1).min(12),
                ))
            }
            KeyCode::Char(' ') => Some(ClientCommand::TogglePlayback),
            KeyCode::Char('p') => Some(ClientCommand::PreviousTrack),
            KeyCode::Char('n') => Some(ClientCommand::NextTrack),
            KeyCode::Char('s') => Some(ClientCommand::ToggleShuffle),
            KeyCode::Char('r') => Some(ClientCommand::CycleRepeat),
            KeyCode::Char('m') => Some(ClientCommand::ToggleMute),
            KeyCode::Char('x') => Some(ClientCommand::ClearQueue),
            KeyCode::Delete if self.queue_focused && !self.playback().queue.is_empty() => {
                Some(ClientCommand::RemoveQueueIndex(self.queue_selected))
            }
            KeyCode::Left => Some(ClientCommand::SeekRelative(-10_000)),
            KeyCode::Right => Some(ClientCommand::SeekRelative(10_000)),
            KeyCode::Char('-') => Some(ClientCommand::SetVolume(
                self.playback().volume_percent.saturating_sub(5),
            )),
            KeyCode::Char('+') | KeyCode::Char('=') => Some(ClientCommand::SetVolume(
                self.playback().volume_percent.saturating_add(5).min(100),
            )),
            KeyCode::Down | KeyCode::Char('j') if self.queue_focused => {
                self.move_queue_selection(true);
                None
            }
            KeyCode::Up | KeyCode::Char('k') if self.queue_focused => {
                self.move_queue_selection(false);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(true),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(false),
            _ => None,
        }
    }

    pub fn handle_mouse(
        &mut self,
        event: MouseEvent,
        hits: &crate::HitMap,
    ) -> Option<ClientCommand> {
        if matches!(
            event.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) && hits.contains(crate::MouseTarget::Player, event.column, event.row)
        {
            let volume = if event.kind == MouseEventKind::ScrollUp {
                self.playback().volume_percent.saturating_add(5).min(100)
            } else {
                self.playback().volume_percent.saturating_sub(5)
            };
            return Some(ClientCommand::SetVolume(volume));
        }
        if event.kind != MouseEventKind::Down(MouseButton::Left) {
            return None;
        }
        let target = hits.target_at(event.column, event.row)?;
        match target {
            crate::MouseTarget::Navigation(Route::Search) => {
                self.search_focused = true;
                self.navigate(Route::Search)
            }
            crate::MouseTarget::Navigation(route) => self.navigate(route),
            crate::MouseTarget::SearchInput => {
                self.open(Route::Search);
                self.search_focused = true;
                Some(ClientCommand::Navigate(Route::Search))
            }
            crate::MouseTarget::ContentTrack(index) => {
                self.select_index(index);
                self.track_at(index).map(ClientCommand::PlayTrack)
            }
            crate::MouseTarget::QueueTrack(index) => {
                self.queue_selected = index;
                Some(ClientCommand::PlayQueueIndex(index))
            }
            crate::MouseTarget::Progress => {
                let area = hits.region(crate::MouseTarget::Progress)?;
                let denominator = area.width.saturating_sub(1).max(1);
                let offset = event.column.saturating_sub(area.x).min(denominator);
                let position = self
                    .playback()
                    .duration_ms
                    .saturating_mul(u64::from(offset))
                    / u64::from(denominator);
                Some(ClientCommand::SeekTo(position))
            }
            crate::MouseTarget::Shuffle => Some(ClientCommand::ToggleShuffle),
            crate::MouseTarget::Previous => Some(ClientCommand::PreviousTrack),
            crate::MouseTarget::PlayPause => Some(ClientCommand::TogglePlayback),
            crate::MouseTarget::Next => Some(ClientCommand::NextTrack),
            crate::MouseTarget::Repeat => Some(ClientCommand::CycleRepeat),
            crate::MouseTarget::Like => self
                .playback()
                .current
                .clone()
                .map(ClientCommand::ToggleLike),
            crate::MouseTarget::Lyrics => Some(ClientCommand::ToggleLyrics),
            crate::MouseTarget::Mute => Some(ClientCommand::ToggleMute),
            crate::MouseTarget::Queue => {
                self.queue_focused = !self.queue_focused;
                None
            }
            crate::MouseTarget::Player => None,
        }
    }

    fn selected_track(&self) -> Option<zerobeat_core::Track> {
        let selected = match self.route() {
            Route::Home => self.library().recent.get(self.home_selected).cloned(),
            Route::Search => self
                .search()
                .results
                .get(self.search().selected_index)
                .cloned(),
            Route::Library => self.library_tracks().get(self.library_selected).cloned(),
            Route::Downloads => self
                .library()
                .downloads
                .get(self.downloads_selected)
                .map(|download| download.track.clone()),
            Route::Settings => None,
        };
        selected.or_else(|| self.playback().current.clone())
    }

    fn track_at(&self, index: usize) -> Option<zerobeat_core::Track> {
        match self.route() {
            Route::Home => self.library().recent.get(index).cloned(),
            Route::Search => self.search().results.get(index).cloned(),
            Route::Library => self.library_tracks().get(index).cloned(),
            Route::Downloads => self
                .library()
                .downloads
                .get(index)
                .map(|download| download.track.clone()),
            Route::Settings => None,
        }
    }

    fn select_index(&mut self, index: usize) {
        match self.route() {
            Route::Home => self.home_selected = index,
            Route::Search => self.snapshot.search.selected_index = index,
            Route::Library => self.library_selected = index,
            Route::Downloads => self.downloads_selected = index,
            Route::Settings => {}
        }
    }

    fn library_tracks(&self) -> Vec<zerobeat_core::Track> {
        let mut tracks = self.library().liked.clone();
        for track in &self.library().recent {
            if !tracks.iter().any(|existing| existing.id == track.id) {
                tracks.push(track.clone());
            }
        }
        tracks
    }

    fn move_selection(&mut self, next: bool) -> Option<ClientCommand> {
        if self.route() == Route::Search {
            return Some(if next {
                ClientCommand::SelectNext
            } else {
                ClientCommand::SelectPrevious
            });
        }
        let length = match self.route() {
            Route::Home => self.library().recent.len(),
            Route::Library => self.library_tracks().len(),
            Route::Downloads => self.library().downloads.len(),
            Route::Search | Route::Settings => 0,
        };
        if length == 0 {
            return None;
        }
        let selected = match self.route() {
            Route::Home => &mut self.home_selected,
            Route::Library => &mut self.library_selected,
            Route::Downloads => &mut self.downloads_selected,
            Route::Search | Route::Settings => return None,
        };
        *selected = if next {
            (*selected + 1) % length
        } else {
            selected.checked_sub(1).unwrap_or(length - 1)
        };
        None
    }

    fn move_queue_selection(&mut self, next: bool) {
        let length = self.playback().queue.len();
        if length == 0 {
            self.queue_selected = 0;
        } else if next {
            self.queue_selected = (self.queue_selected + 1) % length;
        } else {
            self.queue_selected = self.queue_selected.checked_sub(1).unwrap_or(length - 1);
        }
    }

    fn navigate(&mut self, route: Route) -> Option<ClientCommand> {
        self.open(route);
        Some(ClientCommand::Navigate(route))
    }

    fn handle_search_key(&mut self, code: KeyCode) -> Option<ClientCommand> {
        match code {
            KeyCode::Esc => {
                self.search_focused = false;
                None
            }
            KeyCode::Enter => {
                self.search_focused = false;
                (!self.search_query().trim().is_empty()).then_some(ClientCommand::SubmitSearch)
            }
            KeyCode::Backspace => {
                let mut query = self.search_query().to_owned();
                query.pop();
                self.snapshot.navigation.update_search(query.clone());
                Some(ClientCommand::UpdateSearch(query))
            }
            KeyCode::Char(character) => {
                let mut query = self.search_query().to_owned();
                query.push(character);
                self.snapshot.navigation.update_search(query.clone());
                Some(ClientCommand::UpdateSearch(query))
            }
            _ => None,
        }
    }
}
