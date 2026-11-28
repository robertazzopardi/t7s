//! Agent-mode key router: decides which keys a4s keeps and which go to
//! the agent's PTY. The agent gets every key except these single chords.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::defaults::{BACK, QUIT};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppAction {
    Quit,
    /// 1-based session number.
    Focus(usize),
    Sessions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Agent(KeyEvent),
    App(AppAction),
    Ignore,
}

/// Route one key.
///
/// `exited` is true when the focused agent is gone, so Esc has no one to
/// interrupt and goes back to the sessions list instead. `enhanced` is whether the terminal has kitty keyboard enhancement.
#[must_use]
pub fn route(key: KeyEvent, exited: bool, enhanced: bool) -> Route {
    if key.kind != KeyEventKind::Press {
        return Route::Ignore;
    }
    let chord = (key.code, key.modifiers);
    if chord == QUIT {
        return Route::App(AppAction::Quit);
    }
    if chord == BACK || (exited && chord == (KeyCode::Esc, KeyModifiers::NONE))
    {
        return Route::App(AppAction::Sessions);
    }
    // Legacy Ctrl+] is byte 0x1D, the same as Ctrl+5, so Ctrl+5 means back there.
    // Legacy Ctrl+4/6/7 are really Ctrl+\ ^ _ (0x1C/0x1E/0x1F), so they go to the agent.
    if !enhanced && chord == (KeyCode::Char('5'), KeyModifiers::CONTROL) {
        return Route::App(AppAction::Sessions);
    }
    if key.modifiers == KeyModifiers::CONTROL
        && let KeyCode::Char(c) = key.code
        && let Some(n) = c.to_digit(10).filter(|d| *d >= 1)
        && let Ok(n) = usize::try_from(n)
        && (enhanced || !matches!(c, '4' | '6' | '7'))
    {
        return Route::App(AppAction::Focus(n));
    }
    Route::Agent(key)
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
    fn esc() -> KeyEvent {
        press(KeyCode::Esc, KeyModifiers::NONE)
    }

    #[test]
    fn agent_keys_pass_through() {
        for k in [ctrl('c'), ctrl('a'), esc(), ctrl('0')] {
            assert_eq!(route(k, false, true), Route::Agent(k));
        }
    }

    #[test]
    fn single_chords() {
        assert_eq!(route(ctrl('q'), false, true), Route::App(AppAction::Quit));
        assert_eq!(
            route(ctrl(']'), false, true),
            Route::App(AppAction::Sessions)
        );
        assert_eq!(
            route(ctrl('3'), false, true),
            Route::App(AppAction::Focus(3))
        );
    }

    #[test]
    fn legacy_ctrl5_goes_back() {
        assert_eq!(
            route(ctrl('5'), false, false),
            Route::App(AppAction::Sessions)
        );
        assert_eq!(
            route(ctrl('5'), false, true),
            Route::App(AppAction::Focus(5))
        );
    }

    #[test]
    fn legacy_ctrl_punct_reaches_agent() {
        for (c, n) in [('4', 4), ('6', 6), ('7', 7)] {
            assert_eq!(route(ctrl(c), false, false), Route::Agent(ctrl(c)));
            assert_eq!(
                route(ctrl(c), false, true),
                Route::App(AppAction::Focus(n))
            );
        }
    }

    #[test]
    fn esc_goes_back_only_after_exit() {
        assert_eq!(route(esc(), true, true), Route::App(AppAction::Sessions));
        let shift_esc = press(KeyCode::Esc, KeyModifiers::SHIFT);
        assert_eq!(route(shift_esc, true, true), Route::Agent(shift_esc));
    }

    #[test]
    fn release_events_ignored() {
        let mut k = ctrl('q');
        k.kind = KeyEventKind::Release;
        assert_eq!(route(k, false, true), Route::Ignore);
        k.kind = KeyEventKind::Repeat;
        assert_eq!(route(k, false, true), Route::Ignore);
    }
}
