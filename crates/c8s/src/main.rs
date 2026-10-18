extern crate alloc;

use std::panic::{set_hook, take_hook};
mod app;
mod app_state;
mod command;
mod docker;
mod event_handlers;
mod log_dedup;
mod rendering;
mod ui;

use std::io::stdout;

use app::App;
use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
};

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let terminal = ratatui::init();
    execute!(stdout(), EnableBracketedPaste)?;
    let prev_hook = take_hook();
    set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableBracketedPaste);
        prev_hook(info);
    }));

    let mut app = App::new();
    let result = app.run(terminal).await;

    let _ = execute!(stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}
