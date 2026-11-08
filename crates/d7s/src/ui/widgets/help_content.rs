use k9tui::{
    config::{Config, Row},
    keymap::Keymap,
    widgets::help_view::HelpRow,
};

use crate::app_state::{AppState, DatabaseExplorerState};

/// Names of the `[d7s.help]` sections shown in this state, in order.
fn help_sections(
    app_state: AppState,
    explorer_state: &DatabaseExplorerState,
) -> Vec<&'static str> {
    let mut sections = vec!["global", "navigation"];

    match (app_state, explorer_state) {
        (AppState::ConnectionList, DatabaseExplorerState::Connections) => {
            sections.push("connections");
        }
        (
            AppState::DatabaseConnected,
            DatabaseExplorerState::TableData(_, _),
        ) => {
            sections.extend([
                "database",
                "table-data",
                "cell-edit",
                "table-data-hidden",
                "filter",
            ]);
        }
        (AppState::DatabaseConnected, DatabaseExplorerState::SqlResults(_)) => {
            sections.extend(["database", "sql-results", "sql-results-hidden"]);
        }
        (AppState::DatabaseConnected, _) => {
            sections.push("database");
        }
        _ => {}
    }

    sections
}

fn row_text(row: &Row, keymap: &Keymap) -> (String, String) {
    match row {
        Row::Key { key, label } => (key.clone(), label.clone()),
        Row::Action {
            actions,
            key,
            label,
        } => {
            let keys: Vec<_> = key
                .iter()
                .cloned()
                .chain(actions.iter().map(|action| keymap.labels(action)))
                .collect();
            (keys.join(" / "), label.clone())
        }
    }
}

/// # Panics
///
/// If a section name in [`help_sections`] is missing from `config.toml`, which
/// is a programmer error.
#[must_use]
pub fn help_rows(
    app_state: AppState,
    explorer_state: &DatabaseExplorerState,
    config: &Config,
) -> Vec<HelpRow> {
    let mut rows = Vec::new();
    for name in help_sections(app_state, explorer_state) {
        let section = config
            .help
            .get(name)
            .unwrap_or_else(|| panic!("unknown help section: {name}"));
        // A leading row with no key describes the whole section.
        let (group_desc, entries) = match section.rows.as_slice() {
            [Row::Key { key, label }, rest @ ..] if key.is_empty() => {
                (label.clone(), rest)
            }
            entries => (String::new(), entries),
        };
        rows.push(HelpRow {
            key: section.title.clone().into(),
            desc: group_desc.into(),
            section_header: true,
        });
        for row in entries {
            let (key, desc) = row_text(row, &config.keymap);
            rows.push(HelpRow {
                key: key.into(),
                desc: desc.into(),
                section_header: false,
            });
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use k9tui::config::{Config, defaults};

    use super::*;
    use crate::keymap::SPEC;

    fn table_data() -> DatabaseExplorerState {
        DatabaseExplorerState::TableData("public".into(), "users".into())
    }

    fn body(state: &DatabaseExplorerState, config: &Config) -> String {
        help_rows(AppState::DatabaseConnected, state, config)
            .iter()
            .filter(|r| !r.section_header)
            .map(|r| format!("{} {}", r.key, r.desc))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn table_data_help_lists_inline_edit_keys() {
        let body = body(&table_data(), &defaults(&SPEC));
        assert!(body.contains("n / a New row (blank draft)"));
        assert!(body.contains("N New row (column defaults)"));
        assert!(body.contains("i Edit selected cell"));
        assert!(body.contains("Tab / BackTab"));
        assert!(body.contains("Ctrl+e Multiline editor (modal)"));
        assert!(body.contains(": / # Jump to row number"));
    }

    #[test]
    fn sql_help_shows_default_export_key() {
        let state = DatabaseExplorerState::SqlResults(String::new());
        let body = body(&state, &defaults(&SPEC));
        assert!(body.contains("Ctrl+s / x Export results"), "{body}");
    }

    #[test]
    fn help_shows_remapped_key() {
        let mut config = defaults(&SPEC);
        assert!(config.keymap.set("delete-row", KeyCode::Char('z')));
        let body = body(&table_data(), &config);
        assert!(body.contains("z Delete row(s)"), "{body}");
        assert!(!body.contains("D Delete row(s)"), "{body}");
    }

    #[test]
    fn help_shows_remapped_key_inside_a_combined_label() {
        let mut config = defaults(&SPEC);
        assert!(config.keymap.set("export", KeyCode::F(5)));
        assert!(config.keymap.set("command", KeyCode::Char('z')));
        let state = DatabaseExplorerState::SqlResults(String::new());
        assert!(body(&state, &config).contains("Ctrl+s / F5 Export results"));
        assert!(body(&table_data(), &config).contains("z / # Jump to row"));
    }
}
