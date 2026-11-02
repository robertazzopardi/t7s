use crossterm::event::KeyCode;
use k9tui::config::AppSpec;
#[cfg(test)]
use k9tui::config::defaults;

/// `c8s` config spec. Navigation, the number keys and aliases stay fixed.
pub const SPEC: AppSpec = AppSpec {
    name: "c8s",
    actions: &[
        ("running", KeyCode::Char('a')),
        ("processes", KeyCode::Char('p')),
        ("start-stop", KeyCode::Char('s')),
        ("restart", KeyCode::Char('r')),
        ("logs", KeyCode::Char('l')),
        ("exec", KeyCode::Char('e')),
        ("describe", KeyCode::Char('d')),
        ("remove", KeyCode::Char('D')),
        ("health", KeyCode::Char('i')),
        ("width", KeyCode::Char('W')),
    ],
    screens: &[(
        "resource list",
        &[
            "running",
            "processes",
            "start-stop",
            "restart",
            "logs",
            "exec",
            "describe",
            "remove",
            "health",
            "width",
            "quit",
            "search",
            "command",
        ],
    )],
    fixed: &[
        KeyCode::Esc,
        KeyCode::Enter,
        KeyCode::Delete,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('g'),
        KeyCode::Char('G'),
        KeyCode::Char('S'),
        KeyCode::Char('1'),
        KeyCode::Char('2'),
        KeyCode::Char('3'),
        KeyCode::Char('4'),
    ],
};

#[cfg(test)]
mod tests {
    use k9tui::{config::resolve, widgets::hotkey::Hotkey};

    use super::*;
    use crate::ui::widgets::hotkeys::{
        GLOBAL_HOTKEYS, LIST_HOTKEYS, RESOURCE_HOTKEYS,
    };

    fn bar() -> Vec<Hotkey> {
        LIST_HOTKEYS
            .iter()
            .chain(RESOURCE_HOTKEYS.iter())
            .chain(GLOBAL_HOTKEYS.iter())
            .cloned()
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
        for hotkey in bar() {
            if let Some(action) = hotkey.action {
                assert_eq!(keymap.get(action), hotkey.keycode, "{action}");
            }
        }
    }

    #[test]
    fn every_action_is_tagged_on_the_bar() {
        let bar = bar();
        let tagged: Vec<_> = bar.iter().filter_map(|h| h.action).collect();
        for (action, _) in SPEC.actions {
            assert!(tagged.contains(action), "{action} is not on the bar");
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
