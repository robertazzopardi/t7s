use crossterm::event::KeyEvent;
use k9tui::{event::is_ctrl_c, keymap::plain};

use crate::app::{App, View};

impl App {
    pub fn on_key_event(&mut self, key: KeyEvent) {
        match self.view {
            View::Sessions => self.on_key_sessions(key),
            // Agent keys are forwarded to the PTY in a later task.
            View::Agent => {}
        }
    }

    fn on_key_sessions(&mut self, key: KeyEvent) {
        if is_ctrl_c(key)
            || self.config.keymap.action("sessions", plain(key)) == "quit"
        {
            self.quit();
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};

    use super::*;

    #[test]
    fn q_quits_sessions_view() {
        let mut app = App::new();
        app.running = true;
        app.on_key_event(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(!app.running);
    }
}
