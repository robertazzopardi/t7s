use crossterm::event::KeyCode;
use k9tui::widgets::hotkey::Hotkey;

use crate::db::connection::ConnectionType;

pub const CONNECTION_HOTKEYS: [Hotkey; 8] = [
    Hotkey::new('n', "New Connection").action("new-connection"),
    Hotkey::new('e', "Edit Connection").action("edit-connection"),
    Hotkey::new('D', "Delete Connection").action("delete-connection"),
    Hotkey::new('o', "Open Connection").action("open-connection"),
    Hotkey::new('p', "Ping Health").action("ping"),
    Hotkey::new('d', "Describe").action("describe"),
    Hotkey::new('/', "Search").action("search"),
    Hotkey::new(':', "Command").action("command"),
];

pub const DATABASE_HOTKEYS: [Hotkey; 9] = [
    Hotkey::new('e', "SQL Editor").action("sql-editor"),
    Hotkey::new('t', "Table structure").action("table-structure"),
    Hotkey::new('d', "Describe").action("describe"),
    Hotkey::new('E', "Run SQL").action("run-sql"),
    Hotkey::new('/', "Search").action("search"),
    Hotkey::new(':', "Command").action("command"),
    Hotkey::new('y', "Copy value").action("copy"),
    Hotkey::new('A', "Activity (Postgres)").action("activity"),
    Hotkey::new('L', "Query log").action("query-log"),
];

/// [`DATABASE_HOTKEYS`] for a backend: the activity view (`A`) is Postgres-only.
#[must_use]
pub fn database_hotkeys(connection_type: ConnectionType) -> Vec<Hotkey> {
    DATABASE_HOTKEYS
        .iter()
        .filter(|h| {
            h.keycode != KeyCode::Char('A')
                || connection_type.has_activity_view()
        })
        .cloned()
        .collect()
}

/// Shown in addition to [`DATABASE_HOTKEYS`] while viewing table row data.
pub const TABLE_DATA_VIEW_HOTKEYS: [Hotkey; 6] = [
    Hotkey::new('n', "New row").action("new-row"),
    Hotkey::new('N', "Defaults").action("defaults"),
    Hotkey::new('c', "Duplicate").action("duplicate"),
    Hotkey::new('s', "Commit").action("commit"),
    Hotkey::new('D', "Delete").action("delete-row"),
    Hotkey::new('i', "Edit").action("edit"),
];

/// Replaces [`TABLE_DATA_VIEW_HOTKEYS`] in the top bar during inline cell edit.
pub const CELL_EDIT_HOTKEYS: [Hotkey; 6] = [
    Hotkey::code(KeyCode::Esc, "Cancel"),
    Hotkey::code(KeyCode::Enter, "Done"),
    Hotkey::code(KeyCode::Tab, "Next col"),
    Hotkey::code(KeyCode::BackTab, "Prev col"),
    Hotkey::labeled("Ctrl+e", KeyCode::Char('e'), "Modal"),
    Hotkey::code(KeyCode::Backspace, "Delete char"),
];
