use a4s::app::App;
use k9tui::error::Result;

#[tokio::main]
async fn main() -> Result<()> {
    k9tui::launch::<App>(())
}
