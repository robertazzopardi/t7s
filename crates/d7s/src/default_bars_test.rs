//! Today's default hotkey bars and help, so moving their definitions can't
//! change them.
use std::fmt::Write as _;

use k9tui::{config::defaults, widgets::hotkey::Hotkey};

use crate::{
    app_state::{AppState, DatabaseExplorerState},
    db::connection::ConnectionType,
    keymap::SPEC,
    ui::widgets::{
        global_hotkeys::global_hotkeys,
        help_content::help_rows,
        hotkeys::{
            CELL_EDIT_HOTKEYS, CONNECTION_HOTKEYS, TABLE_DATA_VIEW_HOTKEYS,
            database_hotkeys,
        },
    },
};

fn section(out: &mut String, title: &str, hotkeys: &[Hotkey]) {
    writeln!(out, "[{title}]").unwrap();
    for hotkey in defaults(&SPEC).keymap.relabel(hotkeys) {
        writeln!(
            out,
            "{} {}",
            hotkey.key_label(),
            hotkey.description.display_suffix()
        )
        .unwrap();
    }
}

fn help(out: &mut String, app: AppState, state: &DatabaseExplorerState) {
    writeln!(out, "[{state:?}]").unwrap();
    for row in help_rows(app, state, &defaults(&SPEC).keymap) {
        let mark = if row.section_header { "# " } else { "" };
        writeln!(out, "{mark}{} | {}", row.key, row.desc).unwrap();
    }
}

#[test]
fn default_bars_are_unchanged() {
    let mut out = String::new();
    section(&mut out, "connections", &CONNECTION_HOTKEYS);
    section(
        &mut out,
        "database-postgres",
        &database_hotkeys(ConnectionType::Postgres),
    );
    section(
        &mut out,
        "database-sqlite",
        &database_hotkeys(ConnectionType::Sqlite),
    );
    section(&mut out, "table-data", &TABLE_DATA_VIEW_HOTKEYS);
    section(&mut out, "cell-edit", &CELL_EDIT_HOTKEYS);
    section(&mut out, "global-connections", &global_hotkeys(true));
    section(&mut out, "global-explorer", &global_hotkeys(false));
    assert_eq!(out, include_str!("default_bars.golden"));
}

#[test]
fn default_help_is_unchanged() {
    let mut out = String::new();
    help(
        &mut out,
        AppState::ConnectionList,
        &DatabaseExplorerState::Connections,
    );
    help(
        &mut out,
        AppState::DatabaseConnected,
        &DatabaseExplorerState::Databases,
    );
    help(
        &mut out,
        AppState::DatabaseConnected,
        &DatabaseExplorerState::TableData("s".into(), "t".into()),
    );
    help(
        &mut out,
        AppState::DatabaseConnected,
        &DatabaseExplorerState::SqlResults("q".into()),
    );
    assert_eq!(out, include_str!("default_help.golden"));
}
