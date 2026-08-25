use crossterm::event::KeyCode;

use crate::styles::WaveStyle;

pub struct Gallery {
    selected: usize,
    expanded: bool,
    paused: bool,
    intensity: u16,
    tick: u64,
    should_quit: bool,
}

impl Default for Gallery {
    fn default() -> Self {
        Self {
            selected: 0,
            expanded: false,
            paused: false,
            intensity: 100,
            tick: 0,
            should_quit: false,
        }
    }
}

impl Gallery {
    pub fn handle_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('1'..='6') => {
                let KeyCode::Char(number) = key else {
                    return;
                };
                self.selected = usize::from(number as u8 - b'1');
            }
            KeyCode::Left => {
                self.selected = self
                    .selected
                    .checked_sub(1)
                    .unwrap_or(WaveStyle::ALL.len() - 1);
            }
            KeyCode::Right => {
                self.selected = (self.selected + 1) % WaveStyle::ALL.len();
            }
            KeyCode::Enter => self.expanded = !self.expanded,
            KeyCode::Char(' ') => self.paused = !self.paused,
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.intensity = self.intensity.saturating_add(10).min(150);
            }
            KeyCode::Char('-') => {
                self.intensity = self.intensity.saturating_sub(10).max(30);
            }
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc if self.expanded => self.expanded = false,
            KeyCode::Esc => self.should_quit = true,
            _ => {}
        }
    }

    pub fn advance(&mut self) {
        if !self.paused {
            self.tick = self.tick.wrapping_add(1);
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_style(&self) -> WaveStyle {
        WaveStyle::ALL[self.selected]
    }

    pub fn expanded(&self) -> bool {
        self.expanded
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn intensity(&self) -> u16 {
        self.intensity
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_and_arrow_navigation_select_styles() {
        let mut gallery = Gallery::default();

        gallery.handle_key(KeyCode::Char('4'));
        assert_eq!(gallery.selected(), 3);
        gallery.handle_key(KeyCode::Right);
        assert_eq!(gallery.selected(), 4);
        gallery.handle_key(KeyCode::Left);
        assert_eq!(gallery.selected(), 3);
    }

    #[test]
    fn gallery_controls_freeze_expand_intensity_and_exit() {
        let mut gallery = Gallery::default();

        gallery.handle_key(KeyCode::Char(' '));
        assert!(gallery.paused());
        gallery.handle_key(KeyCode::Enter);
        assert!(gallery.expanded());
        gallery.handle_key(KeyCode::Char('+'));
        assert_eq!(gallery.intensity(), 110);
        gallery.handle_key(KeyCode::Esc);
        assert!(!gallery.expanded());
        gallery.handle_key(KeyCode::Char('q'));
        assert!(gallery.should_quit());
    }

    #[test]
    fn paused_gallery_does_not_advance_animation() {
        let mut gallery = Gallery::default();

        gallery.advance();
        assert_eq!(gallery.tick(), 1);
        gallery.handle_key(KeyCode::Char(' '));
        gallery.advance();
        assert_eq!(gallery.tick(), 1);
    }
}
