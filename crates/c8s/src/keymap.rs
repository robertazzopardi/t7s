use crossterm::event::KeyCode;
use k9tui::keymap::Keymap;

use crate::app::PKG_NAME;

/// Default bindings for the actions shown in the hotkey bar. Navigation
/// (j/k/g/G, arrows) and quit/back are not remappable — only the actions
/// listed here and documented in the README.
const DEFAULTS: &[(&str, KeyCode)] = &[
    ("start_stop", KeyCode::Char('s')),
    ("restart", KeyCode::Char('r')),
    ("logs", KeyCode::Char('l')),
    ("exec", KeyCode::Char('e')),
    ("describe", KeyCode::Char('d')),
    ("remove", KeyCode::Char('D')),
];

/// Built-in bindings only; never touches the filesystem (tests and
/// `App::new` use this).
#[must_use]
pub fn defaults() -> Keymap {
    Keymap::new(DEFAULTS)
}

/// Defaults plus `~/.config/c8s/keys.yml` overrides. Returns warnings
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
    use crate::ui::widgets::hotkeys::{LIST_HOTKEYS, RESOURCE_HOTKEYS};

    /// A no-config run must reproduce the pre-config hardcoded keys.
    #[test]
    fn defaults_match_the_documented_key_set() {
        let km = defaults();
        let expected = [
            ("start_stop", 's'),
            ("restart", 'r'),
            ("logs", 'l'),
            ("exec", 'e'),
            ("describe", 'd'),
            ("remove", 'D'),
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
        let bar: Vec<_> = LIST_HOTKEYS
            .iter()
            .chain(RESOURCE_HOTKEYS.iter())
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
        let bar: Vec<_> = LIST_HOTKEYS
            .iter()
            .chain(RESOURCE_HOTKEYS.iter())
            .cloned()
            .collect();
        let out = km.relabel(&bar);
        assert!(out.iter().any(|h| h.keycode == KeyCode::F(5) && h.action == Some(action)));
    }
}
