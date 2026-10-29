extern crate alloc;

mod app;
mod app_state;
mod command;
mod docker;
mod error;
mod event_handlers;
mod in_flight;
mod log_dedup;
mod rendering;
mod ui;

use app::App;
use k9tui::terminal::run;

#[tokio::main]
async fn main() -> error::Result<()> {
    run(|terminal| App::new().run(terminal))
}
