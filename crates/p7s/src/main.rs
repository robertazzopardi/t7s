extern crate alloc;

use std::{
    env,
    io::stdout,
    panic::{set_hook, take_hook},
    process::exit,
    sync::Arc,
};

mod app;
mod app_state;
mod command;
mod error;
mod event_handlers;
mod proc;
mod rendering;
mod ui;

use app::{App, PKG_NAME, PKG_VERSION};
use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
};
use proc::monitor::{DemoSource, ProcessSource, SystemMonitor};

fn print_usage() {
    eprintln!(
        "\
{PKG_NAME} — TUI process viewer

Usage:
  {PKG_NAME} [OPTIONS]

Options:
      --demo     Show canned demo data (no access to real processes)
  -h, --help     Show this help
  -V, --version  Print version and exit
"
    );
}

/// `true` when `--demo` was requested; exits the process on --help/--version.
fn parse_args() -> bool {
    let mut demo = false;
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                exit(0);
            }
            "-V" | "--version" => {
                println!("{PKG_NAME} {PKG_VERSION}");
                exit(0);
            }
            "--demo" => demo = true,
            other => {
                eprintln!("Unknown argument: {other}\n");
                print_usage();
                exit(1);
            }
        }
    }
    demo
}

#[tokio::main]
async fn main() -> error::Result<()> {
    let demo = parse_args();

    let terminal = ratatui::init();
    execute!(stdout(), EnableBracketedPaste)?;
    let prev_hook = take_hook();
    set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableBracketedPaste);
        prev_hook(info);
    }));

    let source: Arc<dyn ProcessSource> = if demo {
        Arc::new(DemoSource::new())
    } else {
        Arc::new(SystemMonitor::new())
    };
    let mut app = App::new(source);
    let result = app.run(terminal);

    let _ = execute!(stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}
