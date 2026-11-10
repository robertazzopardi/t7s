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

use app::App;
use k9tui::error::Result;

#[tokio::main]
async fn main() -> Result<()> {
    k9tui::launch::<App>(())
}
