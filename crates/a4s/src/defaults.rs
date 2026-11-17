//! Built-in a4s constants (chords, agent commands). Filled in by later tasks.

/// Lines of scrollback kept per agent terminal.
pub const SCROLLBACK_LINES: usize = 10_000;

use crossterm::event::{KeyCode, KeyModifiers};

/// Agent-mode chord prefix (`Ctrl+a`, then a command key).
pub const PREFIX: (KeyCode, KeyModifiers) =
    (KeyCode::Char('a'), KeyModifiers::CONTROL);
/// Agent-mode quit chord.
pub const QUIT: (KeyCode, KeyModifiers) =
    (KeyCode::Char('q'), KeyModifiers::CONTROL);
