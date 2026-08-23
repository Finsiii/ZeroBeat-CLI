use crossterm::event::{KeyCode, KeyEvent};
use zerobeat_core::Route;
use zerobeat_protocol::AppSnapshot;

#[derive(Default)]
pub struct App {
    snapshot: AppSnapshot,
    search_focused: bool,
    should_quit: bool,
}

impl App {
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

    pub fn handle_key(&mut self, event: KeyEvent) {
        if self.search_focused {
            self.handle_search_key(event.code);
            return;
        }

        match event.code {
            KeyCode::Char('/') => {
                self.open(Route::Search);
                self.search_focused = true;
            }
            KeyCode::Char('1') => self.open(Route::Home),
            KeyCode::Char('2') => self.open(Route::Search),
            KeyCode::Char('3') => self.open(Route::Library),
            KeyCode::Char('4') => self.open(Route::Downloads),
            KeyCode::Char('5') => self.open(Route::Settings),
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => self.snapshot.navigation.back(),
            _ => {}
        }
    }

    fn handle_search_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc | KeyCode::Enter => self.search_focused = false,
            KeyCode::Backspace => {
                let mut query = self.search_query().to_owned();
                query.pop();
                self.snapshot.navigation.update_search(query);
            }
            KeyCode::Char(character) => {
                let mut query = self.search_query().to_owned();
                query.push(character);
                self.snapshot.navigation.update_search(query);
            }
            _ => {}
        }
    }
}
