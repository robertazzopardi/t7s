extern crate alloc;

mod app;
mod app_state;
mod command;
#[cfg(test)]
mod default_bars_test;
mod docker;
mod event_handlers;
mod in_flight;
mod keymap;
mod log_dedup;
mod rendering;
mod ui;

use app::App;
use k9tui::{
    config::{load_or_exit, summarize},
    error::Result,
    terminal::run,
};

#[tokio::main]
async fn main() -> Result<()> {
    let config = load_or_exit(&keymap::SPEC);
    let warnings = config.warnings.clone();
    let status = summarize(&warnings);
    let result = run(|terminal| {
        let mut app = App::new();
        app.config = config;
        if let Some(status) = status {
            app.status_line.set_message(status);
        }
        app.run(terminal)
    });
    for warning in &warnings {
        eprintln!("{}: {warning}", app::PKG_NAME);
    }
    result
}
