use k9tui::config::AppSpec;
#[cfg(test)]
use k9tui::config::defaults;

/// `d7s` definitions live in `config.toml`.
pub const SPEC: AppSpec = AppSpec {
    name: "d7s",
    config: include_str!("config.toml"),
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
    fn builtin_files_validate() {
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
