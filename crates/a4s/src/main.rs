mod app;
#[cfg(test)]
mod default_bars_test;
mod defaults;
mod event_handlers;
mod keymap;
mod rendering;
mod session;

use app::App;
use k9tui::error::Result;

#[tokio::main]
async fn main() -> Result<()> {
    k9tui::launch::<App>(())
}
