//! Terminal setup/teardown and handing the screen to a child process.

use std::{
    io::stdout,
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

use crate::{
    config::{AppSpec, Config, load_or_exit},
    error::{Result, err},
};

/// An app `launch` can start.
pub trait TuiApp: Sized {
    const SPEC: AppSpec;
    /// What the app needs from its command line.
    type Args;

    /// Build the app once its config has loaded.
    ///
    /// # Errors
    ///
    /// Whatever stops the app from starting.
    fn build(config: Config, args: Self::Args) -> Result<Self>;

    /// Run until the user quits.
    ///
    /// # Errors
    ///
    /// Whatever ends the app early.
    fn start(&mut self, terminal: DefaultTerminal) -> Result<()>;
}

/// Load the config, init the terminal with bracketed paste, run the app, then
/// restore the terminal (also on panic). A bad config exits before the TUI.
///
/// # Errors
///
/// Returns the error from enabling bracketed paste, or the app's own error.
pub fn launch<A: TuiApp>(args: A::Args) -> Result<()> {
    let config = load_or_exit(&A::SPEC);
    let terminal = ratatui::init();
    execute!(stdout(), EnableBracketedPaste)?;
    let prev_hook = take_hook();
    set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableBracketedPaste);
        prev_hook(info);
    }));
    let result = A::build(config, args).and_then(|mut app| app.start(terminal));
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
    // The TUI hides the cursor; the next `draw` hides it again on return.
    terminal.show_cursor().map_err(|e| err(e.to_string()))?;
    let out = child();
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    execute!(stdout(), EnableBracketedPaste)?;
    terminal.clear().map_err(|e| err(e.to_string()))?;
    Ok(out)
}
