use crossterm::event::KeyCode;
use k9tui::config::AppSpec;
#[cfg(test)]
use k9tui::config::defaults;

/// `d7s` config spec. Navigation, the number keys and aliases stay fixed.
pub const SPEC: AppSpec = AppSpec {
    name: "d7s",
    actions: &[
        ("new-connection", KeyCode::Char('n')),
        ("edit-connection", KeyCode::Char('e')),
        ("delete-connection", KeyCode::Char('D')),
        ("open-connection", KeyCode::Char('o')),
        ("reconnect", KeyCode::Char('O')),
        ("ping", KeyCode::Char('p')),
        ("describe", KeyCode::Char('d')),
        ("sql-editor", KeyCode::Char('e')),
        ("table-structure", KeyCode::Char('t')),
        ("run-sql", KeyCode::Char('E')),
        ("copy", KeyCode::Char('y')),
        ("copy-row", KeyCode::Char('Y')),
        ("help", KeyCode::Char('?')),
        ("watch", KeyCode::Char('w')),
        ("width", KeyCode::Char('W')),
        ("query-log", KeyCode::Char('L')),
        ("activity", KeyCode::Char('A')),
        ("export", KeyCode::Char('x')),
        ("refresh", KeyCode::Char('r')),
        ("new-row", KeyCode::Char('n')),
        ("defaults", KeyCode::Char('N')),
        ("edit", KeyCode::Char('i')),
        ("duplicate", KeyCode::Char('c')),
        ("commit", KeyCode::Char('s')),
        ("delete-row", KeyCode::Char('D')),
    ],
    screens: &[
        (
            "connection list",
            &[
                "new-connection",
                "edit-connection",
                "delete-connection",
                "open-connection",
                "reconnect",
                "ping",
                "describe",
                "copy",
                "copy-row",
                "width",
                "help",
                "quit",
                "search",
                "command",
            ],
        ),
        (
            "database",
            &[
                "sql-editor",
                "table-structure",
                "describe",
                "run-sql",
                "copy",
                "copy-row",
                "watch",
                "width",
                "query-log",
                "activity",
                "export",
                "help",
                "quit",
                "search",
                "command",
            ],
        ),
        (
            "table data",
            &[
                "sql-editor",
                "table-structure",
                "describe",
                "copy",
                "copy-row",
                "width",
                "query-log",
                "activity",
                "help",
                "quit",
                "search",
                "command",
                "refresh",
                "new-row",
                "defaults",
                "edit",
                "duplicate",
                "commit",
                "delete-row",
            ],
        ),
    ],
    fixed: &[
        KeyCode::Esc,
        KeyCode::Enter,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::Char(' '),
        KeyCode::Char('h'),
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('l'),
        KeyCode::Char('b'),
        KeyCode::Char('w'),
        KeyCode::Char('g'),
        KeyCode::Char('G'),
        KeyCode::Char('0'),
        KeyCode::Char('$'),
        KeyCode::Char('#'),
        KeyCode::Char('R'),
        KeyCode::Char('a'),
        KeyCode::Char('1'),
        KeyCode::Char('2'),
        KeyCode::Char('3'),
        KeyCode::Char('4'),
        KeyCode::Char('5'),
    ],
};

#[cfg(test)]
mod tests {
    use k9tui::{config::resolve, widgets::hotkey::Hotkey};

    use super::*;
    use crate::ui::widgets::{
        global_hotkeys::global_hotkeys,
        hotkeys::{
            CONNECTION_HOTKEYS, DATABASE_HOTKEYS, TABLE_DATA_VIEW_HOTKEYS,
        },
    };

    fn bar() -> Vec<Hotkey> {
        CONNECTION_HOTKEYS
            .iter()
            .chain(DATABASE_HOTKEYS.iter())
            .chain(TABLE_DATA_VIEW_HOTKEYS.iter())
            .cloned()
            .chain(global_hotkeys(true))
            .collect()
    }

    #[test]
    fn defaults_have_no_errors() {
        let config = resolve(None, &SPEC, "test").unwrap();
        assert_eq!(config.warnings, Vec::<String>::new());
    }

    #[test]
    fn hotkey_bar_is_unchanged_by_defaults() {
        let bar = bar();
        let relabelled = defaults(&SPEC).keymap.relabel(&bar);
        for (before, after) in bar.iter().zip(&relabelled) {
            assert_eq!(before.keycode, after.keycode, "{before}");
        }
    }

    #[test]
    fn every_tagged_hotkey_names_a_real_action() {
        let keymap = defaults(&SPEC).keymap;
        let bar = bar();
        assert!(bar.iter().any(|hotkey| hotkey.action.is_some()));
        for hotkey in bar {
            if let Some(action) = hotkey.action {
                assert_eq!(keymap.get(action), hotkey.keycode, "{action}");
            }
        }
    }

    /// Every example a user can copy from the README must start the app.
    #[test]
    fn readme_examples_are_valid() {
        let readme = include_str!("../README.md");
        let examples: Vec<_> = readme
            .split("```toml\n")
            .skip(1)
            .filter_map(|rest| rest.split("```").next())
            .collect();
        assert_ne!(examples.len(), 0);
        for example in examples {
            let config = resolve(Some(example), &SPEC, "README").unwrap();
            assert_eq!(config.warnings, Vec::<String>::new(), "{example}");
        }
    }
}
