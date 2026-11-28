use std::{env, process};

use a4s::app::App;
#[cfg(unix)]
use a4s::hooks::notify;
use k9tui::error::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    // Hooks must never fail the calling agent, so bad arity still exits 0.
    if args.first().is_some_and(|cmd| cmd == "notify") {
        #[cfg(unix)]
        let code = if let [_, state] = args.as_slice() {
            notify(state)
        } else {
            eprintln!("usage: a4s notify <working|blocked|done>");
            0
        };
        #[cfg(not(unix))]
        let code = 0;
        process::exit(code);
    }
    k9tui::launch::<App>(())
}
