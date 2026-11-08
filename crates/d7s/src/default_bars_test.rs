//! Today's default hotkey bars and help, so moving their definitions can't
//! change them.
use std::fmt::Write as _;

use crossterm::event::KeyCode;
use k9tui::{config::defaults, widgets::hotkey::Hotkey};

use crate::{
    app_state::{AppState, DatabaseExplorerState},
    db::connection::ConnectionType,
    keymap::SPEC,
    rendering::database_bar,
    ui::widgets::help_content::help_rows,
};

fn section(out: &mut String, title: &str, hotkeys: &[Hotkey]) {
    writeln!(out, "[{title}]").unwrap();
    for hotkey in hotkeys {
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
    for row in help_rows(app, state, &defaults(&SPEC)) {
        let mark = if row.section_header { "# " } else { "" };
        writeln!(out, "{mark}{} | {}", row.key, row.desc).unwrap();
    }
}

#[test]
fn default_bars_are_unchanged() {
    let config = defaults(&SPEC);
    let mut out = String::new();
    section(&mut out, "connections", &config.bar("connections"));
    section(
        &mut out,
        "database-postgres",
        &database_bar(&config, ConnectionType::Postgres),
    );
    section(
        &mut out,
        "database-sqlite",
        &database_bar(&config, ConnectionType::Sqlite),
    );
    section(&mut out, "table-data", &config.bar("table-data"));
    section(&mut out, "cell-edit", &config.bar("cell-edit"));
    section(
        &mut out,
        "global-connections",
        &config.bar("global-connections"),
    );
    section(&mut out, "global-explorer", &config.bar("global-explorer"));
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

#[test]
fn activity_row_hides_by_action_when_remapped() {
    let mut config = defaults(&SPEC);
    assert!(config.keymap.set("activity", KeyCode::Char('z')));
    let has_activity = |t| {
        database_bar(&config, t)
            .iter()
            .any(|h| h.action.as_deref() == Some("activity"))
    };
    assert!(has_activity(ConnectionType::Postgres));
    assert!(!has_activity(ConnectionType::Sqlite));
}
