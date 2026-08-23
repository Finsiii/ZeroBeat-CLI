use crossterm::event::{KeyCode, KeyEvent};
use zerobeat_core::Route;
use zerobeat_protocol::{AppSnapshot, ClientCommand};

#[derive(Default)]
pub struct App {
    snapshot: AppSnapshot,
    search_focused: bool,
    should_quit: bool,
}

impl App {
    pub fn new(snapshot: AppSnapshot) -> Self {
        Self {
            snapshot,
            ..Self::default()
        }
    }

    pub fn replace_snapshot(&mut self, snapshot: AppSnapshot) {
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
            KeyCode::Char('2') => self.navigate(Route::Search),
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
            _ => None,
        }
    }

    fn navigate(&mut self, route: Route) -> Option<ClientCommand> {
        self.open(route);
        Some(ClientCommand::Navigate(route))
    }

    fn handle_search_key(&mut self, code: KeyCode) -> Option<ClientCommand> {
        match code {
            KeyCode::Esc | KeyCode::Enter => {
                self.search_focused = false;
                None
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
