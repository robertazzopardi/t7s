//! Agent-mode chord router: decides which keys a4s keeps and which go to
//! the agent's PTY.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::defaults::{PREFIX, QUIT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppAction {
    Quit,
    /// 1-based session number.
    Focus(usize),
    Next,
    Prev,
    Sessions,
    New,
    Restart,
    Help,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Agent(KeyEvent),
    App(AppAction),
    Pending,
    Ignore,
}

#[derive(Default)]
pub struct Router {
    pending: bool,
}

impl Router {
    pub fn route(&mut self, key: KeyEvent) -> Route {
        if key.kind != KeyEventKind::Press {
            return Route::Ignore;
        }
        let chord = (key.code, key.modifiers);
        if self.pending {
            self.pending = false;
            if chord == PREFIX {
                return Route::Agent(key);
            }
            // `?` may arrive with SHIFT; other modifiers are not commands.
            let mods = key.modifiers - KeyModifiers::SHIFT;
            if let KeyCode::Char(c) = key.code
                && mods.is_empty()
            {
                return after_prefix(c).map_or(Route::Ignore, Route::App);
            }
            return Route::Ignore;
        }
        if chord == QUIT {
            return Route::App(AppAction::Quit);
        }
        if chord == PREFIX {
            self.pending = true;
            return Route::Pending;
        }
        if key.modifiers == KeyModifiers::CONTROL
            && let KeyCode::Char(c) = key.code
            && let Some(n) = digit(c)
        {
            return Route::App(AppAction::Focus(n));
        }
        Route::Agent(key)
    }
}

fn digit(c: char) -> Option<usize> {
    c.to_digit(10)
        .filter(|d| *d >= 1)
        .and_then(|d| usize::try_from(d).ok())
}

fn after_prefix(c: char) -> Option<AppAction> {
    match c {
        'n' => Some(AppAction::Next),
        'p' => Some(AppAction::Prev),
        's' => Some(AppAction::Sessions),
        'c' => Some(AppAction::New),
        'r' => Some(AppAction::Restart),
        '?' => Some(AppAction::Help),
        _ => digit(c).map(AppAction::Focus),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }
    fn ctrl(c: char) -> KeyEvent {
        press(KeyCode::Char(c), KeyModifiers::CONTROL)
    }
    fn plain(c: char) -> KeyEvent {
        press(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn ctrl_c_goes_to_agent() {
        let k = ctrl('c');
        assert_eq!(Router::default().route(k), Route::Agent(k));
    }

    #[test]
    fn ctrl_q_quits() {
        assert_eq!(
            Router::default().route(ctrl('q')),
            Route::App(AppAction::Quit)
        );
    }

    #[test]
    fn ctrl_3_focuses_3() {
        assert_eq!(
            Router::default().route(ctrl('3')),
            Route::App(AppAction::Focus(3))
        );
    }

    #[test]
    fn prefix_then_2_focuses_2() {
        let mut r = Router::default();
        assert_eq!(r.route(ctrl('a')), Route::Pending);
        assert_eq!(r.route(plain('2')), Route::App(AppAction::Focus(2)));
        assert_eq!(r.route(plain('2')), Route::Agent(plain('2')));
    }

    #[test]
    fn prefix_commands() {
        for (c, a) in [
            ('n', AppAction::Next),
            ('p', AppAction::Prev),
            ('s', AppAction::Sessions),
            ('c', AppAction::New),
            ('r', AppAction::Restart),
        ] {
            let mut r = Router::default();
            r.route(ctrl('a'));
            assert_eq!(r.route(plain(c)), Route::App(a));
        }
        let mut r = Router::default();
        r.route(ctrl('a'));
        assert_eq!(
            r.route(press(KeyCode::Char('?'), KeyModifiers::SHIFT)),
            Route::App(AppAction::Help)
        );
    }

    #[test]
    fn prefix_twice_sends_literal_prefix() {
        let mut r = Router::default();
        r.route(ctrl('a'));
        assert_eq!(r.route(ctrl('a')), Route::Agent(ctrl('a')));
        assert_eq!(r.route(plain('n')), Route::Agent(plain('n')));
    }

    #[test]
    fn prefix_then_unknown_is_ignored_and_clears() {
        let mut r = Router::default();
        r.route(ctrl('a'));
        assert_eq!(r.route(plain('z')), Route::Ignore);
        assert_eq!(r.route(plain('n')), Route::Agent(plain('n')));
        r.route(ctrl('a'));
        assert_eq!(
            r.route(press(KeyCode::Esc, KeyModifiers::NONE)),
            Route::Ignore
        );
    }

    #[test]
    fn release_events_ignored() {
        let mut k = ctrl('q');
        k.kind = KeyEventKind::Release;
        assert_eq!(Router::default().route(k), Route::Ignore);
        k.kind = KeyEventKind::Repeat;
        assert_eq!(Router::default().route(k), Route::Ignore);
    }
}
