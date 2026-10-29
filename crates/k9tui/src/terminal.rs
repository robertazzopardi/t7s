//! Terminal setup/teardown and handing the screen to a child process.

use std::{
    io::{self, stdout},
    panic::{set_hook, take_hook},
};

use crossterm::{
    ExecutableCommand,
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use ratatui::{DefaultTerminal, Terminal, backend::Backend};

use crate::error::{Result, err};

/// Init the terminal with bracketed paste, run `app`, then restore it
/// (also on panic).
///
/// # Errors
///
/// Returns the error from enabling bracketed paste, or `app`'s own error.
pub fn run<T, E: From<io::Error>>(
    app: impl FnOnce(DefaultTerminal) -> Result<T, E>,
) -> Result<T, E> {
    let terminal = ratatui::init();
    execute!(stdout(), EnableBracketedPaste)?;
    let prev_hook = take_hook();
    set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableBracketedPaste);
        prev_hook(info);
    }));
    let result = app(terminal);
    let _ = execute!(stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}

/// Leave the TUI, run `child` (an editor, a shell) on the plain terminal,
/// then restore the TUI and return `child`'s result.
///
/// # Errors
///
/// Returns any error from leaving or re-entering the TUI.
pub fn suspend<B: Backend, R>(
    terminal: &mut Terminal<B>,
    child: impl FnOnce() -> R,
) -> Result<R> {
    execute!(stdout(), DisableBracketedPaste)?;
    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    let out = child();
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    execute!(stdout(), EnableBracketedPaste)?;
    terminal.clear().map_err(|e| err(e.to_string()))?;
    Ok(out)
}
