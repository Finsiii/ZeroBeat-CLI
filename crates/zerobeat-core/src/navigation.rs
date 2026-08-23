use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum Route {
    #[default]
    Home,
    Search,
    Library,
    Downloads,
    Settings,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct NavigationState {
    active: Route,
    history: Vec<Route>,
    search_query: String,
}

impl NavigationState {
    pub fn active_route(&self) -> Route {
        self.active
    }

    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    pub fn open(&mut self, route: Route) {
        if route == self.active {
            return;
        }

        self.history.push(self.active);
        self.active = route;
    }

    pub fn back(&mut self) {
        if let Some(route) = self.history.pop() {
            self.active = route;
        }
    }

    pub fn update_search(&mut self, query: impl Into<String>) {
        self.search_query = query.into();
    }
}
