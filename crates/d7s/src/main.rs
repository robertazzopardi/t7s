extern crate alloc;

use std::{env, process::exit};

mod app;
mod app_state;
mod auth;
mod bg;
mod cell_edit;
mod command;
mod command_bar;
mod connection_manager;
mod database_explorer;
mod database_explorer_state;
mod db;
mod event_handlers;
mod filtered_data;
mod filtering;
mod in_flight;
mod rendering;
mod services;
mod sql;
mod table_data_actions;
mod ui;
mod virtual_table;

use app::{App, PKG_NAME, PKG_VERSION};
use k9tui::{
    error::{Result, err},
    terminal::run,
};

fn print_usage() {
    eprintln!(
        "\
{PKG_NAME} — TUI database client

Usage:
  {PKG_NAME} [OPTIONS]

Options:
  -c, --connection NAME  Open named connection on launch
  -h, --help             Show this help
  -V, --version          Print version and exit
"
    );
}

/// `None` = run TUI; `Some(name)` = connect on launch; exits process on --help/--version.
fn parse_launch_args() -> Result<Option<String>> {
    let mut args = env::args().skip(1);
    let mut connection: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                exit(0);
            }
            "-V" | "--version" => {
                println!("{PKG_NAME} {PKG_VERSION}");
                exit(0);
            }
            "-c" | "--connection" => {
                let name = args
                    .next()
                    .ok_or_else(|| err("--connection requires a name"))?;
                connection = Some(name);
            }
            other => {
                eprintln!("Unknown argument: {other}\n");
                print_usage();
                exit(1);
            }
        }
    }
    Ok(connection)
}

#[tokio::main]
async fn main() -> Result<()> {
    let launch_connection = parse_launch_args()?;
    run(|terminal| {
        let mut app = App::default().init()?;
        if let Some(name) = launch_connection {
            app.connect_to_named_connection(&name);
        }
        app.run(terminal)
    })
}
