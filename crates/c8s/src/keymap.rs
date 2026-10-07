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

/// Actions live at the same time (every view shows a subset of these).
const SCREENS: &[(&str, &[&str])] = &[(
    "resource list",
    &[
        "start_stop",
        "restart",
        "logs",
        "exec",
        "describe",
        "remove",
    ],
)];

/// Fixed keys an action must not take over (navigation, quit, search...).
const RESERVED: &[KeyCode] = &[
    KeyCode::Esc,
    KeyCode::Enter,
    KeyCode::Delete,
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::Char('q'),
    KeyCode::Char('/'),
    KeyCode::Char(':'),
    KeyCode::Char('a'),
    KeyCode::Char('p'),
    KeyCode::Char('i'),
    KeyCode::Char('W'),
    KeyCode::Char('j'),
    KeyCode::Char('k'),
    KeyCode::Char('g'),
    KeyCode::Char('G'),
    KeyCode::Char('S'),
    KeyCode::Char('1'),
    KeyCode::Char('2'),
    KeyCode::Char('3'),
    KeyCode::Char('4'),
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
    let mut warnings = keymap.load_overrides(PKG_NAME);
    warnings.extend(keymap.validate(DEFAULTS, SCREENS, RESERVED, "keys.yml"));
    (keymap, warnings)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

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

    const BAR_SETS: [&[k9tui::widgets::hotkey::Hotkey]; 2] =
        [&LIST_HOTKEYS, &RESOURCE_HOTKEYS];

    #[test]
    fn defaults_are_self_consistent() {
        let km = defaults();
        // No default sits on a reserved key, and no screen has duplicates.
        for (a, k) in DEFAULTS {
            assert!(!RESERVED.contains(k), "{a} default is reserved");
        }
        let mut km2 = km;
        assert_eq!(
            km2.validate(DEFAULTS, SCREENS, RESERVED, "t"),
            Vec::<String>::new()
        );
        // Every screen action exists; every action appears on a screen.
        for (_, actions) in SCREENS {
            for a in *actions {
                assert!(DEFAULTS.iter().any(|(n, _)| n == a), "{a}");
            }
        }
        for (a, _) in DEFAULTS {
            assert!(SCREENS.iter().any(|(_, s)| s.contains(a)), "{a}");
        }
    }

    /// Remapping every action to a distinct char relabels every tagged
    /// hotkey-bar entry, and nothing else.
    #[test]
    fn every_action_relabels_its_bar_entries() {
        let mut yaml = String::new();
        for (i, (a, _)) in DEFAULTS.iter().enumerate() {
            let c = char::from(b'A' + u8::try_from(i).unwrap_or(0));
            writeln!(yaml, "{a}: {c}").unwrap();
        }
        let mut km = defaults();
        assert_eq!(km.apply_yaml(&yaml, "t"), Vec::<String>::new());
        let bar: Vec<_> =
            BAR_SETS.iter().flat_map(|s| s.iter()).cloned().collect();
        let out = km.relabel(&bar);
        for (before, after) in bar.iter().zip(&out) {
            match before.action {
                Some(a) => assert_eq!(after.keycode, km.get(a), "{a}"),
                None => assert_eq!(after.keycode, before.keycode),
            }
        }
    }

    #[test]
    fn remapping_onto_reserved_or_same_screen_key_warns() {
        let first = SCREENS
            .first()
            .and_then(|(_, a)| a.first())
            .copied()
            .unwrap();
        let second = SCREENS
            .first()
            .and_then(|(_, a)| a.get(1))
            .copied()
            .unwrap();
        let mut km = defaults();
        km.apply_yaml(
            &format!("{first}: esc\n{second}: f9\n{first}: esc\n"),
            "t",
        );
        let w = km.validate(DEFAULTS, SCREENS, RESERVED, "t");
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(
            km.get(first),
            DEFAULTS.iter().find(|(n, _)| *n == first).unwrap().1
        );
        let mut km = defaults();
        km.apply_yaml(&format!("{first}: f9\n{second}: f9\n"), "t");
        let w = km.validate(DEFAULTS, SCREENS, RESERVED, "t");
        assert_eq!(w.len(), 1, "{w:?}");
    }
}
