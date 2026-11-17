//! Built-in a4s constants (chords, agent commands). Filled in by later tasks.

/// Lines of scrollback kept per agent terminal.
pub const SCROLLBACK_LINES: usize = 10_000;

use core::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};

/// A working agent that prints nothing for this long is considered done.
pub const IDLE_AFTER: Duration = Duration::from_secs(5);

/// Hook socket file name for the a4s process with this id.
#[must_use]
pub fn socket_name(pid: u32) -> String {
    format!("a4s-{pid}.sock")
}

/// Agent-mode chord prefix (`Ctrl+a`, then a command key).
pub const PREFIX: (KeyCode, KeyModifiers) =
    (KeyCode::Char('a'), KeyModifiers::CONTROL);
/// Agent-mode quit chord.
pub const QUIT: (KeyCode, KeyModifiers) =
    (KeyCode::Char('q'), KeyModifiers::CONTROL);
