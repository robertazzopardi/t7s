use crossterm::event::KeyCode;
use k9tui::keymap::Keymap;

use crate::app::PKG_NAME;

/// Default bindings for the actions shown in the hotkey bar (see
/// `ui::widgets::hotkeys` and `ui::widgets::global_hotkeys`). Vim-style
/// navigation (h/j/k/l, g/G, 0/$, /), quit, help, and Esc are not
/// remappable — only the actions listed here and documented in the README.
const DEFAULTS: &[(&str, KeyCode)] = &[
    // Connection list
    ("new_connection", KeyCode::Char('n')),
    ("edit_connection", KeyCode::Char('e')),
    ("delete_connection", KeyCode::Char('D')),
    ("describe", KeyCode::Char('d')),
    ("open_connection", KeyCode::Char('o')),
    // Database view
    ("sql_editor", KeyCode::Char('e')),
    ("table_structure", KeyCode::Char('t')),
    ("run_sql", KeyCode::Char('E')),
    ("copy_value", KeyCode::Char('y')),
    // Table data view
    ("refresh", KeyCode::Char('r')),
    ("new_row", KeyCode::Char('n')),
    ("duplicate_row", KeyCode::Char('c')),
    ("commit_row", KeyCode::Char('s')),
    ("delete_row", KeyCode::Char('D')),
];

/// Built-in bindings only; never touches the filesystem (tests and
/// `App::new` use this).
#[must_use]
pub fn defaults() -> Keymap {
    Keymap::new(DEFAULTS)
}

/// Defaults plus `~/.config/d7s/keys.yml` overrides. Returns warnings
/// to show the user; problems never prevent startup.
#[must_use]
pub fn load() -> (Keymap, Vec<String>) {
    let mut keymap = defaults();
    let warnings = keymap.load_overrides(PKG_NAME);
    (keymap, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::hotkeys::{
        CONNECTION_HOTKEYS, DATABASE_HOTKEYS, TABLE_DATA_VIEW_HOTKEYS,
    };

    /// A no-config run must reproduce the pre-config hardcoded keys.
    #[test]
    fn defaults_match_the_documented_key_set() {
        let km = defaults();
        let expected = [
            ("new_connection", 'n'),
            ("edit_connection", 'e'),
            ("delete_connection", 'D'),
            ("describe", 'd'),
            ("open_connection", 'o'),
            ("sql_editor", 'e'),
            ("table_structure", 't'),
            ("run_sql", 'E'),
            ("copy_value", 'y'),
            ("refresh", 'r'),
            ("new_row", 'n'),
            ("duplicate_row", 'c'),
            ("commit_row", 's'),
            ("delete_row", 'D'),
        ];
        assert_eq!(DEFAULTS.len(), expected.len());
        for (action, c) in expected {
            assert_eq!(km.get(action), KeyCode::Char(c), "{action}");
        }
    }

    /// Tagged hotkey-bar entries name a real action whose default key
    /// is the one the bar displays, so relabelling is a no-op without config.
    #[test]
    fn hotkey_bar_is_unchanged_by_default_keymap() {
        let bar: Vec<_> = CONNECTION_HOTKEYS
            .iter()
            .chain(DATABASE_HOTKEYS.iter())
            .chain(TABLE_DATA_VIEW_HOTKEYS.iter())
            .cloned()
            .collect();
        let out = defaults().relabel(&bar);
        for (a, b) in bar.iter().zip(&out) {
            assert_eq!(
                a.keycode,
                b.keycode,
                "{}",
                a.description.display_suffix()
            );
            if let Some(action) = a.action {
                assert!(DEFAULTS.iter().any(|(n, _)| *n == action), "{action}");
            }
        }
    }

    #[test]
    fn overrides_flow_into_hotkey_bar() {
        let mut km = defaults();
        let (action, _) = DEFAULTS.first().copied().unwrap();
        let w = km.apply_yaml(&format!("{action}: f5"), "t");
        assert_eq!(w, Vec::<String>::new());
        let bar: Vec<_> = CONNECTION_HOTKEYS
            .iter()
            .chain(DATABASE_HOTKEYS.iter())
            .chain(TABLE_DATA_VIEW_HOTKEYS.iter())
            .cloned()
            .collect();
        let out = km.relabel(&bar);
        assert!(out.iter().any(|h| h.keycode == KeyCode::F(5) && h.action == Some(action)));
    }
}
