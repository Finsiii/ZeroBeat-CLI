use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use zerobeat_core::Route;
use zerobeat_protocol::{
    AppSnapshot, ClientCommand, PlaybackSnapshot, PlaybackStatus, SPECTRUM_BAND_COUNT,
    SearchSnapshot, SearchStatus,
};

const TRANSPORT_COOLDOWN: Duration = Duration::from_secs(2);

#[derive(Default)]
struct TransportCooldown {
    blocked_until: Option<Instant>,
}

impl TransportCooldown {
    fn try_acquire(&mut self, now: Instant) -> bool {
        if self.is_active_at(now) {
            return false;
        }
        self.restart_at(now);
        true
    }

    fn restart_at(&mut self, now: Instant) {
        self.blocked_until = Some(now + TRANSPORT_COOLDOWN);
    }

    fn is_active_at(&self, now: Instant) -> bool {
        self.blocked_until.is_some_and(|until| now < until)
    }
}

#[derive(Default)]
pub struct App {
    snapshot: AppSnapshot,
    search_focused: bool,
    should_quit: bool,
    home_selected: usize,
    library_selected: usize,
    recent_selected: usize,
    downloads_selected: usize,
    queue_focused: bool,
    queue_selected: usize,
    spectrum_smoothed: [u8; SPECTRUM_BAND_COUNT],
    spectrum_initialized: bool,
    spectrum_track_id: Option<String>,
    transport_cooldown: TransportCooldown,
}

impl App {
    pub fn new(snapshot: AppSnapshot) -> Self {
        let mut app = Self {
            snapshot,
            ..Self::default()
        };
        app.spectrum_track_id = app
            .snapshot
            .playback
            .current
            .as_ref()
            .map(|track| track.id.clone());
        if app.spectrum_is_active() {
            app.spectrum_smoothed = app.snapshot.playback.spectrum;
            app.spectrum_initialized = true;
        }
        app
    }

    pub fn replace_snapshot(&mut self, snapshot: AppSnapshot) {
        self.update_spectrum(&snapshot.playback);
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

    pub fn spectrum(&self) -> &[u8; SPECTRUM_BAND_COUNT] {
        &self.spectrum_smoothed
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
            Route::RecentlyPlayed => self.recent_selected,
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

    pub fn transport_cooling_down(&self) -> bool {
        self.transport_cooldown.is_active_at(Instant::now())
    }

    pub fn extend_transport_cooldown(&mut self) {
        self.transport_cooldown.restart_at(Instant::now());
    }

    pub fn open(&mut self, route: Route) {
        self.search_focused = false;
        self.queue_focused = false;
        self.snapshot.lyrics.visible = false;
        self.snapshot.navigation.open(route);
    }

    pub fn handle_key(&mut self, event: KeyEvent) -> Option<ClientCommand> {
        self.handle_key_at(event, Instant::now())
    }

    fn handle_key_at(&mut self, event: KeyEvent, now: Instant) -> Option<ClientCommand> {
        if self.search_focused {
            return self.handle_search_key(event.code);
        }

        match event.code {
            KeyCode::Char('/') => self.navigate_search(),
            KeyCode::Char('1') => self.navigate(Route::Library),
            KeyCode::Char('2') => self.navigate(Route::RecentlyPlayed),
            KeyCode::Char('3') => self.navigate(Route::Downloads),
            KeyCode::Char('4') => self.navigate(Route::Home),
            KeyCode::Char('5') => self.navigate_search(),
            KeyCode::Char('6') => self.toggle_queue(),
            KeyCode::Char('7') => self.toggle_lyrics(),
            KeyCode::Char('8') => self.navigate(Route::Settings),
            KeyCode::Char('q') => {
                self.should_quit = true;
                None
            }
            KeyCode::Esc => {
                if self.queue_focused {
                    self.queue_focused = false;
                    return None;
                }
                if self.lyrics().visible {
                    return Some(ClientCommand::ToggleLyrics);
                }
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
            KeyCode::Char('y') => self.toggle_lyrics(),
            KeyCode::Char('u') => self.toggle_queue(),
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
            KeyCode::Char('p') => self.transport_command(ClientCommand::PreviousTrack, now),
            KeyCode::Char('n') => self.transport_command(ClientCommand::NextTrack, now),
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
        self.handle_mouse_at(event, hits, Instant::now())
    }

    fn handle_mouse_at(
        &mut self,
        event: MouseEvent,
        hits: &crate::HitMap,
        now: Instant,
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
            crate::MouseTarget::Navigation(Route::Search) => self.navigate_search(),
            crate::MouseTarget::Navigation(route) => self.navigate(route),
            crate::MouseTarget::SearchInput => self.navigate_search(),
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
            crate::MouseTarget::Previous => {
                self.transport_command(ClientCommand::PreviousTrack, now)
            }
            crate::MouseTarget::PlayPause => Some(ClientCommand::TogglePlayback),
            crate::MouseTarget::Next => self.transport_command(ClientCommand::NextTrack, now),
            crate::MouseTarget::Repeat => Some(ClientCommand::CycleRepeat),
            crate::MouseTarget::Like => self
                .playback()
                .current
                .clone()
                .map(ClientCommand::ToggleLike),
            crate::MouseTarget::Lyrics => self.toggle_lyrics(),
            crate::MouseTarget::Mute => Some(ClientCommand::ToggleMute),
            crate::MouseTarget::Queue => self.toggle_queue(),
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
            Route::RecentlyPlayed => self.library().recent.get(self.recent_selected).cloned(),
            Route::Downloads => self
                .library()
                .downloads
                .get(self.downloads_selected)
                .map(|download| download.track.clone()),
            Route::Settings => None,
        };
        selected.or_else(|| self.playback().current.clone())
    }

    fn transport_command(&mut self, command: ClientCommand, now: Instant) -> Option<ClientCommand> {
        self.transport_cooldown.try_acquire(now).then_some(command)
    }

    fn track_at(&self, index: usize) -> Option<zerobeat_core::Track> {
        match self.route() {
            Route::Home => self.library().recent.get(index).cloned(),
            Route::Search => self.search().results.get(index).cloned(),
            Route::Library => self.library_tracks().get(index).cloned(),
            Route::RecentlyPlayed => self.library().recent.get(index).cloned(),
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
            Route::RecentlyPlayed => self.recent_selected = index,
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
            Route::RecentlyPlayed => self.library().recent.len(),
            Route::Downloads => self.library().downloads.len(),
            Route::Search | Route::Settings => 0,
        };
        if length == 0 {
            return None;
        }
        let selected = match self.route() {
            Route::Home => &mut self.home_selected,
            Route::Library => &mut self.library_selected,
            Route::RecentlyPlayed => &mut self.recent_selected,
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

    fn navigate_search(&mut self) -> Option<ClientCommand> {
        let command = self.navigate(Route::Search);
        self.search_focused = true;
        command
    }

    fn toggle_queue(&mut self) -> Option<ClientCommand> {
        if self.queue_focused {
            self.queue_focused = false;
            return None;
        }
        self.queue_focused = true;
        if self.lyrics().visible {
            self.snapshot.lyrics.visible = false;
            Some(ClientCommand::ToggleLyrics)
        } else {
            None
        }
    }

    fn toggle_lyrics(&mut self) -> Option<ClientCommand> {
        self.queue_focused = false;
        Some(ClientCommand::ToggleLyrics)
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

    fn spectrum_is_active(&self) -> bool {
        self.snapshot.playback.status == PlaybackStatus::Playing
            && self.snapshot.playback.current.is_some()
    }

    fn update_spectrum(&mut self, playback: &PlaybackSnapshot) {
        let track_id = playback.current.as_ref().map(|track| track.id.clone());
        let active = playback.status == PlaybackStatus::Playing && track_id.is_some();
        if !active {
            self.spectrum_smoothed = [0; SPECTRUM_BAND_COUNT];
            self.spectrum_initialized = false;
            self.spectrum_track_id = track_id;
            return;
        }

        let track_changed = self.spectrum_track_id != track_id;
        if track_changed || !self.spectrum_initialized {
            self.spectrum_smoothed = playback.spectrum;
            self.spectrum_initialized = true;
        } else {
            for (smoothed, target) in self
                .spectrum_smoothed
                .iter_mut()
                .zip(playback.spectrum.iter().copied())
            {
                *smoothed = smooth_band(*smoothed, target.min(100));
            }
        }
        self.spectrum_track_id = track_id;
    }
}

fn smooth_band(current: u8, target: u8) -> u8 {
    if target > current {
        let delta = u16::from(target - current);
        current.saturating_add((delta * 3).div_ceil(4) as u8)
    } else {
        let delta = u16::from(current - target);
        current.saturating_sub(delta.div_ceil(5) as u8)
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    #[test]
    fn transport_cooldown_reopens_at_exactly_two_seconds() {
        let start = Instant::now();
        let mut app = App::default();

        assert_eq!(
            app.handle_key_at(key('n'), start),
            Some(ClientCommand::NextTrack)
        );
        assert_eq!(
            app.handle_key_at(key('p'), start + Duration::from_millis(1_999)),
            None
        );
        assert_eq!(
            app.handle_key_at(key('p'), start + Duration::from_secs(2)),
            Some(ClientCommand::PreviousTrack)
        );
    }

    #[test]
    fn completed_transport_restarts_the_full_cooldown() {
        let start = Instant::now();
        let mut cooldown = TransportCooldown::default();

        assert!(cooldown.try_acquire(start));
        cooldown.restart_at(start + Duration::from_millis(750));
        assert!(cooldown.is_active_at(start + Duration::from_millis(2_749)));
        assert!(!cooldown.is_active_at(start + Duration::from_millis(2_750)));
    }

    fn key(character: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)
    }
}
