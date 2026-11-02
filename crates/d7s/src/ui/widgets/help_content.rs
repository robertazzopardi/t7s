use k9tui::{
    keymap::Keymap,
    widgets::{help_view::HelpRow, hotkey::Hotkey},
};

use crate::app_state::{AppState, DatabaseExplorerState};

struct HelpEntry {
    /// Key label; `{}` stands for the current key when `action` is set.
    key: &'static str,
    desc: &'static str,
    action: Option<&'static str>,
}

const GLOBAL_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "{}",
        desc: "Toggle help",
        action: Some("help"),
    },
    HelpEntry {
        key: "{}",
        desc: "Quit",
        action: Some("quit"),
    },
    HelpEntry {
        key: "Ctrl+c",
        desc: "Quit",
        action: None,
    },
    HelpEntry {
        key: "{}",
        desc: "Copy cell value",
        action: Some("copy"),
    },
    HelpEntry {
        key: "{}",
        desc: "Describe selected object",
        action: Some("describe"),
    },
    HelpEntry {
        key: "Esc",
        desc: "Back / clear filter / close help",
        action: None,
    },
];

const VIM_NAV_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "j / ↓",
        desc: "Move down",
        action: None,
    },
    HelpEntry {
        key: "k / ↑",
        desc: "Move up",
        action: None,
    },
    HelpEntry {
        key: "h / ←",
        desc: "Move left (columns)",
        action: None,
    },
    HelpEntry {
        key: "l / →",
        desc: "Move right (columns)",
        action: None,
    },
    HelpEntry {
        key: "g",
        desc: "Jump to top",
        action: None,
    },
    HelpEntry {
        key: "G",
        desc: "Jump to bottom",
        action: None,
    },
    HelpEntry {
        key: "0",
        desc: "First column",
        action: None,
    },
    HelpEntry {
        key: "$",
        desc: "Last column",
        action: None,
    },
    HelpEntry {
        key: "{}",
        desc: "Search filter",
        action: Some("search"),
    },
    HelpEntry {
        key: "{}",
        desc: "Toggle column width (Fit / Content)",
        action: Some("width"),
    },
];

const CONNECTION_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "{}",
        desc: "New connection",
        action: Some("new-connection"),
    },
    HelpEntry {
        key: "{}",
        desc: "Edit connection",
        action: Some("edit-connection"),
    },
    HelpEntry {
        key: "{}",
        desc: "Delete connection",
        action: Some("delete-connection"),
    },
    HelpEntry {
        key: "{}",
        desc: "Open connection",
        action: Some("open-connection"),
    },
    HelpEntry {
        key: "{}",
        desc: "Reconnect last connection",
        action: Some("reconnect"),
    },
    HelpEntry {
        key: "{}",
        desc: "Ping all connections (reachability)",
        action: Some("ping"),
    },
    HelpEntry {
        key: "Enter",
        desc: "Connect",
        action: None,
    },
    HelpEntry {
        key: "{}",
        desc: "Command bar (:q, :help, :log; Tab completes)",
        action: Some("command"),
    },
];

const DATABASE_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "{}",
        desc: "SQL editor (external)",
        action: Some("sql-editor"),
    },
    HelpEntry {
        key: "{}",
        desc: "Table structure",
        action: Some("table-structure"),
    },
    HelpEntry {
        key: "{}",
        desc: "Describe selected table/column/row",
        action: Some("describe"),
    },
    HelpEntry {
        key: "{}",
        desc: "Run SQL",
        action: Some("run-sql"),
    },
    HelpEntry {
        key: "Enter",
        desc: "Drill into selection",
        action: None,
    },
    HelpEntry {
        key: "1–5",
        desc: "Open recent table",
        action: None,
    },
    HelpEntry {
        key: "{}",
        desc: "Command bar (:tables, :sql, :log, :table/:schema <name>, :table conn.db.schema.table, :123 ... Tab completes)",
        action: Some("command"),
    },
    HelpEntry {
        key: "{}",
        desc: "Query log (everything d7s ran this session)",
        action: Some("query-log"),
    },
];

const TABLE_DATA_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "{}",
        desc: "New row (blank draft)",
        action: Some("new-row"),
    },
    HelpEntry {
        key: "a",
        desc: "Alias for new row (blank draft)",
        action: None,
    },
    HelpEntry {
        key: "{}",
        desc: "New row (column defaults)",
        action: Some("defaults"),
    },
    HelpEntry {
        key: "{}",
        desc: "Duplicate row as draft",
        action: Some("duplicate"),
    },
    HelpEntry {
        key: "{}",
        desc: "Commit draft row",
        action: Some("commit"),
    },
    HelpEntry {
        key: "{}",
        desc: "Delete row(s)",
        action: Some("delete-row"),
    },
    HelpEntry {
        key: "{}",
        desc: "Edit selected cell",
        action: Some("edit"),
    },
    HelpEntry {
        key: "{}",
        desc: "Refresh",
        action: Some("refresh"),
    },
];

const TABLE_DATA_CELL_EDIT_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "Esc",
        desc: "Cancel edit (revert cell)",
        action: None,
    },
    HelpEntry {
        key: "Enter",
        desc: "Commit cell",
        action: None,
    },
    HelpEntry {
        key: "Tab / Shift+Tab",
        desc: "Next / previous editable column",
        action: None,
    },
    HelpEntry {
        key: "Ctrl+e",
        desc: "Multiline editor (modal)",
        action: None,
    },
    HelpEntry {
        key: "Backspace",
        desc: "Delete character in buffer",
        action: None,
    },
];

/// Keys for table data not shown in the top hotkey bar.
const TABLE_DATA_HIDDEN_BAR: &[HelpEntry] = &[
    HelpEntry {
        key: "Space",
        desc: "Toggle multi-select",
        action: None,
    },
    HelpEntry {
        key: "Enter",
        desc: "Edit cell",
        action: None,
    },
    HelpEntry {
        key: "{}",
        desc: "Copy row (TSV)",
        action: Some("copy-row"),
    },
    HelpEntry {
        key: "{} / #",
        desc: "Jump to row number",
        action: Some("command"),
    },
    HelpEntry {
        key: "g / G",
        desc: "Jump to top / bottom",
        action: None,
    },
    HelpEntry {
        key: "0 / $",
        desc: "First / last column",
        action: None,
    },
];

const SQL_RESULTS_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "{}",
        desc: "Run SQL from editor",
        action: Some("run-sql"),
    },
    HelpEntry {
        key: "Esc",
        desc: "Return to SQL editor",
        action: None,
    },
    HelpEntry {
        key: "{}",
        desc: "Copy row (TSV)",
        action: Some("copy-row"),
    },
    HelpEntry {
        key: "Ctrl+s / {}",
        desc: "Export results to temp TSV",
        action: Some("export"),
    },
];

const SQL_RESULTS_HIDDEN_BAR: &[HelpEntry] = &[
    HelpEntry {
        key: "{}",
        desc: "Copy row (TSV)",
        action: Some("copy-row"),
    },
    HelpEntry {
        key: "Ctrl+s / {}",
        desc: "Export results to temp TSV",
        action: Some("export"),
    },
    HelpEntry {
        key: "Esc",
        desc: "Return to explorer",
        action: None,
    },
];

const FILTER_NOTE: &[HelpEntry] = &[HelpEntry {
    key: "",
    desc: "Filter matches loaded page only (client-side)",
    action: None,
}];

/// ponytail: static slices per context; no runtime catalog builder.
fn help_sections(
    app_state: AppState,
    explorer_state: &DatabaseExplorerState,
) -> Vec<(&'static str, &[HelpEntry])> {
    let mut sections =
        vec![("Global", GLOBAL_HELP), ("Navigation", VIM_NAV_HELP)];

    match (app_state, explorer_state) {
        (AppState::ConnectionList, DatabaseExplorerState::Connections) => {
            sections.push(("Connections", CONNECTION_HELP));
        }
        (
            AppState::DatabaseConnected,
            DatabaseExplorerState::TableData(_, _),
        ) => {
            sections.push(("Database", DATABASE_HELP));
            sections.push(("Table data", TABLE_DATA_HELP));
            sections.push(("Cell edit", TABLE_DATA_CELL_EDIT_HELP));
            sections.push(("Not in top bar", TABLE_DATA_HIDDEN_BAR));
            sections.push(("Filter", FILTER_NOTE));
        }
        (AppState::DatabaseConnected, DatabaseExplorerState::SqlResults(_)) => {
            sections.push(("Database", DATABASE_HELP));
            sections.push(("SQL results", SQL_RESULTS_HELP));
            sections.push(("Not in top bar", SQL_RESULTS_HIDDEN_BAR));
        }
        (AppState::DatabaseConnected, _) => {
            sections.push(("Database", DATABASE_HELP));
        }
        _ => {}
    }

    sections
}

#[must_use]
pub fn help_rows(
    app_state: AppState,
    explorer_state: &DatabaseExplorerState,
    keymap: &Keymap,
) -> Vec<HelpRow> {
    let mut rows = Vec::new();
    for (title, entries) in help_sections(app_state, explorer_state) {
        let (group_desc, entries) = match entries {
            [HelpEntry { key: "", desc, .. }, rest @ ..] => (*desc, rest),
            _ => ("", entries),
        };
        rows.push(HelpRow {
            key: title.into(),
            desc: group_desc,
            section_header: true,
        });
        for entry in entries {
            rows.push(HelpRow {
                key: entry.action.map_or_else(
                    || entry.key.into(),
                    |action| {
                        let label =
                            Hotkey::code(keymap.get(action), "").key_label();
                        entry.key.replace("{}", &label).into()
                    },
                ),
                desc: entry.desc,
                section_header: false,
            });
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use k9tui::config::defaults;

    use super::*;
    use crate::keymap::SPEC;

    fn table_data() -> DatabaseExplorerState {
        DatabaseExplorerState::TableData("public".into(), "users".into())
    }

    fn body(state: &DatabaseExplorerState, keymap: &Keymap) -> String {
        help_rows(AppState::DatabaseConnected, state, keymap)
            .iter()
            .filter(|r| !r.section_header)
            .map(|r| format!("{} {}", r.key, r.desc))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn table_data_help_lists_inline_edit_keys() {
        let body = body(&table_data(), &defaults(&SPEC).keymap);
        assert!(body.contains("n New row (blank draft)"));
        assert!(body.contains("N New row (column defaults)"));
        assert!(body.contains("i Edit selected cell"));
        assert!(body.contains("Tab / Shift+Tab"));
        assert!(body.contains("Ctrl+e Multiline editor (modal)"));
        assert!(body.contains(": / # Jump to row number"));
    }

    #[test]
    fn sql_help_shows_default_export_key() {
        let state = DatabaseExplorerState::SqlResults(String::new());
        let body = body(&state, &defaults(&SPEC).keymap);
        assert!(body.contains("Ctrl+s / x Export results"), "{body}");
    }

    #[test]
    fn help_shows_remapped_key() {
        let mut keymap = defaults(&SPEC).keymap;
        assert!(keymap.set("delete-row", KeyCode::Char('z')));
        let body = body(&table_data(), &keymap);
        assert!(body.contains("z Delete row(s)"), "{body}");
        assert!(!body.contains("D Delete row(s)"), "{body}");
    }

    #[test]
    fn help_shows_remapped_key_inside_a_combined_label() {
        let mut keymap = defaults(&SPEC).keymap;
        assert!(keymap.set("export", KeyCode::F(5)));
        assert!(keymap.set("command", KeyCode::Char('z')));
        let state = DatabaseExplorerState::SqlResults(String::new());
        assert!(body(&state, &keymap).contains("Ctrl+s / F5 Export results"));
        assert!(body(&table_data(), &keymap).contains("z / # Jump to row"));
    }
}
