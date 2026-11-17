use std::{env, process};

use a4s::app::App;
#[cfg(unix)]
use a4s::hooks::notify;
use k9tui::error::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if let [cmd, state] = args.as_slice()
        && cmd == "notify"
    {
        #[cfg(unix)]
        process::exit(notify(state));
        #[cfg(not(unix))]
        {
            let _ = state;
            process::exit(0);
        }
    }
    k9tui::launch::<App>(())
}
