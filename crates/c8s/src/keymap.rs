use k9tui::config::AppSpec;
#[cfg(test)]
use k9tui::config::defaults;

/// `c8s` definitions live in `config.toml`.
pub const SPEC: AppSpec = AppSpec {
    name: "c8s",
    config: include_str!("config.toml"),
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
        for hotkey in bar() {
            if let Some(action) = hotkey.action {
                assert_eq!(keymap.get(action), hotkey.keycode, "{action}");
            }
        }
    }

    #[test]
    fn conflict_on_logs_screen_is_fatal() {
        let errors = resolve(Some("[c8s.keys]\ndedup = \"j\"\n"), &SPEC, "c")
            .unwrap_err();
        assert!(errors.join("\n").contains("dedup"));
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
