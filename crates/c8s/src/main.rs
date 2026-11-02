extern crate alloc;

mod app;
mod app_state;
mod command;
mod docker;
mod event_handlers;
mod in_flight;
mod keymap;
mod log_dedup;
mod rendering;
mod ui;

use app::App;
use k9tui::{
    config_dir::summarize_warnings, error::Result, terminal::run,
    theme::load_skin,
};

#[tokio::main]
async fn main() -> Result<()> {
    let mut warnings = load_skin(app::PKG_NAME);
    let (keymap, key_warnings) = keymap::load();
    warnings.extend(key_warnings);
    let result = run(|terminal| {
        let mut app = App::new();
        app.keymap = keymap;
        if let Some(summary) = summarize_warnings(&warnings) {
            app.status_line.set_message(summary);
        }
        app.run(terminal)
    });
    for w in &warnings {
        eprintln!("c8s: {w}");
    }
    result
}
