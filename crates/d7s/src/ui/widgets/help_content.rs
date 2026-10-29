use k9tui::widgets::help_view::HelpRow;

use crate::app_state::{AppState, DatabaseExplorerState};

struct HelpEntry {
    key: &'static str,
    desc: &'static str,
}

const GLOBAL_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "?",
        desc: "Toggle help",
    },
    HelpEntry {
        key: "q",
        desc: "Quit",
    },
    HelpEntry {
        key: "Ctrl+c",
        desc: "Quit",
    },
    HelpEntry {
        key: "y",
        desc: "Copy cell value",
    },
    HelpEntry {
        key: "d",
        desc: "Describe selected object",
    },
    HelpEntry {
        key: "Esc",
        desc: "Back / clear filter / close help",
    },
];

const VIM_NAV_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "j / ↓",
        desc: "Move down",
    },
    HelpEntry {
        key: "k / ↑",
        desc: "Move up",
    },
    HelpEntry {
        key: "h / ←",
        desc: "Move left (columns)",
    },
    HelpEntry {
        key: "l / →",
        desc: "Move right (columns)",
    },
    HelpEntry {
        key: "g",
        desc: "Jump to top",
    },
    HelpEntry {
        key: "G",
        desc: "Jump to bottom",
    },
    HelpEntry {
        key: "0",
        desc: "First column",
    },
    HelpEntry {
        key: "$",
        desc: "Last column",
    },
    HelpEntry {
        key: "/",
        desc: "Search filter",
    },
    HelpEntry {
        key: "W",
        desc: "Toggle column width (Fit / Content)",
    },
];

const CONNECTION_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "n",
        desc: "New connection",
    },
    HelpEntry {
        key: "e",
        desc: "Edit connection",
    },
    HelpEntry {
        key: "D",
        desc: "Delete connection",
    },
    HelpEntry {
        key: "o",
        desc: "Open connection",
    },
    HelpEntry {
        key: "O",
        desc: "Reconnect last connection",
    },
    HelpEntry {
        key: "p",
        desc: "Ping all connections (reachability)",
    },
    HelpEntry {
        key: "Enter",
        desc: "Connect",
    },
    HelpEntry {
        key: ":",
        desc: "Command bar (:q, :help, :log; Tab completes)",
    },
];

const DATABASE_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "e",
        desc: "SQL editor (external)",
    },
    HelpEntry {
        key: "t",
        desc: "Table structure",
    },
    HelpEntry {
        key: "d",
        desc: "Describe selected table/column/row",
    },
    HelpEntry {
        key: "E",
        desc: "Run SQL",
    },
    HelpEntry {
        key: "Enter",
        desc: "Drill into selection",
    },
    HelpEntry {
        key: "1–5",
        desc: "Open recent table",
    },
    HelpEntry {
        key: ":",
        desc: "Command bar (:tables, :sql, :log, :table/:schema <name>, :table conn.db.schema.table, :123 ... Tab completes)",
    },
    HelpEntry {
        key: "L",
        desc: "Query log (everything d7s ran this session)",
    },
];

const TABLE_DATA_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "n",
        desc: "New row (blank draft)",
    },
    HelpEntry {
        key: "a",
        desc: "Alias for n (blank draft)",
    },
    HelpEntry {
        key: "N",
        desc: "New row (column defaults)",
    },
    HelpEntry {
        key: "c",
        desc: "Duplicate row as draft",
    },
    HelpEntry {
        key: "s",
        desc: "Commit draft row",
    },
    HelpEntry {
        key: "D",
        desc: "Delete row(s)",
    },
    HelpEntry {
        key: "i",
        desc: "Edit selected cell",
    },
    HelpEntry {
        key: "r",
        desc: "Refresh",
    },
];

const TABLE_DATA_CELL_EDIT_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "Esc",
        desc: "Cancel edit (revert cell)",
    },
    HelpEntry {
        key: "Enter",
        desc: "Commit cell",
    },
    HelpEntry {
        key: "Tab / Shift+Tab",
        desc: "Next / previous editable column",
    },
    HelpEntry {
        key: "Ctrl+e",
        desc: "Multiline editor (modal)",
    },
    HelpEntry {
        key: "Backspace",
        desc: "Delete character in buffer",
    },
];

/// Keys for table data not shown in the top hotkey bar.
const TABLE_DATA_HIDDEN_BAR: &[HelpEntry] = &[
    HelpEntry {
        key: "Space",
        desc: "Toggle multi-select",
    },
    HelpEntry {
        key: "Enter",
        desc: "Edit cell",
    },
    HelpEntry {
        key: "Y",
        desc: "Copy row (TSV)",
    },
    HelpEntry {
        key: ": / #",
        desc: "Jump to row number",
    },
    HelpEntry {
        key: "g / G",
        desc: "Jump to top / bottom",
    },
    HelpEntry {
        key: "0 / $",
        desc: "First / last column",
    },
];

const SQL_RESULTS_HELP: &[HelpEntry] = &[
    HelpEntry {
        key: "E",
        desc: "Run SQL from editor",
    },
    HelpEntry {
        key: "Esc",
        desc: "Return to SQL editor",
    },
    HelpEntry {
        key: "Y",
        desc: "Copy row (TSV)",
    },
    HelpEntry {
        key: "Ctrl+s / x",
        desc: "Export results to temp TSV",
    },
];

const SQL_RESULTS_HIDDEN_BAR: &[HelpEntry] = &[
    HelpEntry {
        key: "Y",
        desc: "Copy row (TSV)",
    },
    HelpEntry {
        key: "Ctrl+s / x",
        desc: "Export results to temp TSV",
    },
    HelpEntry {
        key: "Esc",
        desc: "Return to explorer",
    },
];

const FILTER_NOTE: &[HelpEntry] = &[HelpEntry {
    key: "",
    desc: "Filter matches loaded page only (client-side)",
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
) -> Vec<HelpRow> {
    let mut rows = Vec::new();
    for (title, entries) in help_sections(app_state, explorer_state) {
        let (group_desc, entries) = match entries {
            [HelpEntry { key: "", desc }, rest @ ..] => (*desc, rest),
            _ => ("", entries),
        };
        rows.push(HelpRow {
            key: title,
            desc: group_desc,
            section_header: true,
        });
        for entry in entries {
            rows.push(HelpRow {
                key: entry.key,
                desc: entry.desc,
                section_header: false,
            });
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_data_help_lists_inline_edit_keys() {
        let rows = help_rows(
            AppState::DatabaseConnected,
            &DatabaseExplorerState::TableData("public".into(), "users".into()),
        );
        let body = rows
            .iter()
            .filter(|r| !r.section_header)
            .map(|r| format!("{} {}", r.key, r.desc))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(body.contains("n New row (blank draft)"));
        assert!(body.contains("N New row (column defaults)"));
        assert!(body.contains("i Edit selected cell"));
        assert!(body.contains("Tab / Shift+Tab"));
        assert!(body.contains("Ctrl+e Multiline editor (modal)"));
    }
}
