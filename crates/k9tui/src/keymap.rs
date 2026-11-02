use std::collections::HashMap;

use crossterm::event::KeyCode;

use crate::widgets::hotkey::Hotkey;

/// Parse a key spec: one character (case-sensitive) or a kebab-case key name
/// such as `esc`, `page-up` or `f5` (case-insensitive).
#[must_use]
pub fn parse_key(spec: &str) -> Option<KeyCode> {
    let mut chars = spec.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(KeyCode::Char(c));
    }
    let name = spec.to_ascii_lowercase();
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse().ok()) {
        return (1..=12).contains(&n).then_some(KeyCode::F(n));
    }
    Some(match name.as_str() {
        "esc" => KeyCode::Esc,
        "enter" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "page-up" => KeyCode::PageUp,
        "page-down" => KeyCode::PageDown,
        _ => return None,
    })
}

fn key_name(code: KeyCode) -> String {
    format!("'{}'", Hotkey::code(code, "").key_label())
}

/// Current key for each named action.
#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: HashMap<&'static str, KeyCode>,
}

impl Keymap {
    #[must_use]
    pub fn new(
        defaults: impl IntoIterator<Item = (&'static str, KeyCode)>,
    ) -> Self {
        Self {
            bindings: defaults.into_iter().collect(),
        }
    }

    /// The key bound to `action`.
    ///
    /// # Panics
    ///
    /// If `action` is unknown, which is a programmer error rather than a
    /// config error: user input only reaches the keymap through [`Self::set`].
    #[must_use]
    pub fn get(&self, action: &str) -> KeyCode {
        *self
            .bindings
            .get(action)
            .unwrap_or_else(|| panic!("unknown keymap action: {action}"))
    }

    /// The current key for `action`, as the hotkey bar shows it.
    ///
    /// # Panics
    ///
    /// If `action` is unknown; see [`Self::get`].
    #[must_use]
    pub fn label(&self, action: &str) -> String {
        Hotkey::code(self.get(action), "").key_label()
    }

    /// True if `code` is the key currently bound to `action`.
    #[must_use]
    pub fn is(&self, action: &str, code: KeyCode) -> bool {
        self.get(action) == code
    }

    /// True if any action is currently bound to `code`.
    #[must_use]
    pub fn binds(&self, code: KeyCode) -> bool {
        self.bindings.values().any(|bound| *bound == code)
    }

    /// Rebind `action`. Returns false, changing nothing, if it is unknown.
    pub fn set(&mut self, action: &str, code: KeyCode) -> bool {
        self.bindings
            .get_mut(action)
            .map(|bound| *bound = code)
            .is_some()
    }

    /// Copy of `hotkeys` with each action-tagged entry showing its current key.
    #[must_use]
    pub fn relabel(&self, hotkeys: &[Hotkey]) -> Vec<Hotkey> {
        hotkeys
            .iter()
            .cloned()
            .map(|mut hotkey| {
                if let Some(code) =
                    hotkey.action.and_then(|a| self.bindings.get(a))
                {
                    hotkey.keycode = *code;
                }
                hotkey
            })
            .collect()
    }

    /// Check final bindings: no two actions on one key within a screen, and
    /// no remap onto a key the app keeps fixed. A default may sit on a fixed
    /// key (`w` is both `d7s` watch and next column), so only remaps count.
    #[must_use]
    pub fn validate(
        &self,
        defaults: &Self,
        screens: &[(&str, &[&str])],
        fixed: &[KeyCode],
    ) -> Vec<String> {
        let mut errors = Vec::new();
        let mut actions: Vec<_> = self.bindings.iter().collect();
        actions.sort_unstable_by_key(|(action, _)| **action);
        for (action, code) in actions {
            if defaults.bindings.get(action) != Some(code)
                && fixed.contains(code)
            {
                errors.push(format!(
                    "'{action}': {} is a fixed key and can't be rebound",
                    key_name(*code)
                ));
            }
        }
        for (screen, names) in screens {
            for (i, a) in names.iter().enumerate() {
                for b in names.iter().skip(i + 1) {
                    if let (Some(ka), Some(kb)) =
                        (self.bindings.get(a), self.bindings.get(b))
                        && ka == kb
                    {
                        errors.push(format!(
                            "'{a}' and '{b}' both use {} on the {screen} screen",
                            key_name(*ka)
                        ));
                    }
                }
            }
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;

    use super::*;
    use crate::widgets::hotkey::Hotkey;

    fn char_map(pairs: &[(&'static str, char)]) -> Keymap {
        Keymap::new(pairs.iter().map(|(a, c)| (*a, KeyCode::Char(*c))))
    }

    #[test]
    fn parse_key_reads_chars_case_sensitively() {
        assert_eq!(parse_key("s"), Some(KeyCode::Char('s')));
        assert_eq!(parse_key("Q"), Some(KeyCode::Char('Q')));
    }

    #[test]
    fn parse_key_reads_named_keys_in_any_case() {
        assert_eq!(parse_key("F5"), Some(KeyCode::F(5)));
        assert_eq!(parse_key("f5"), Some(KeyCode::F(5)));
        assert_eq!(parse_key("f12"), Some(KeyCode::F(12)));
        assert_eq!(parse_key("page-up"), Some(KeyCode::PageUp));
        assert_eq!(parse_key("PAGE-DOWN"), Some(KeyCode::PageDown));
        assert_eq!(parse_key("space"), Some(KeyCode::Char(' ')));
        assert_eq!(parse_key("esc"), Some(KeyCode::Esc));
    }

    #[test]
    fn parse_key_rejects_empty_chords_and_unknown_names() {
        assert_eq!(parse_key(""), None);
        assert_eq!(parse_key("ctrl+s"), None);
        assert_eq!(parse_key("f13"), None);
        assert_eq!(parse_key("nonsense"), None);
    }

    #[test]
    fn label_shows_the_current_key() {
        let mut km = char_map(&[("quit", 'q')]);
        assert_eq!(km.label("quit"), "q");
        km.set("quit", KeyCode::F(5));
        assert_eq!(km.label("quit"), "F5");
    }

    #[test]
    fn set_rebinds_known_actions_only() {
        let mut km = char_map(&[("quit", 'q')]);
        assert!(km.set("quit", KeyCode::Char('z')));
        assert!(km.is("quit", KeyCode::Char('z')));
        assert!(!km.is("quit", KeyCode::Char('q')));
        assert!(!km.set("nope", KeyCode::Char('z')));
    }

    #[test]
    fn binds_tracks_current_bindings() {
        let mut km = char_map(&[("quit", 'q')]);
        assert!(km.binds(KeyCode::Char('q')));
        km.set("quit", KeyCode::Char('z'));
        assert!(km.binds(KeyCode::Char('z')));
        assert!(!km.binds(KeyCode::Char('q')));
    }

    #[test]
    fn relabel_follows_remap_and_skips_untagged() {
        let mut km = char_map(&[("remove", 'D')]);
        km.set("remove", KeyCode::F(5));
        let bar = [
            Hotkey::new('D', "remove").action("remove"),
            Hotkey::new('W', "width"),
        ];
        let out = km.relabel(&bar);
        assert_eq!(out.first().map(|h| h.keycode), Some(KeyCode::F(5)));
        assert_eq!(out.get(1).map(|h| h.keycode), Some(KeyCode::Char('W')));
    }

    #[test]
    fn same_screen_duplicate_is_an_error() {
        let defaults = char_map(&[("commit", 's'), ("delete-row", 'D')]);
        let mut km = defaults.clone();
        km.set("commit", KeyCode::Char('D'));
        let errors = km.validate(
            &defaults,
            &[("table", &["commit", "delete-row"])],
            &[],
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        let error = errors.first().unwrap();
        for part in ["commit", "delete-row", "'D'", "table"] {
            assert!(error.contains(part), "{part} missing from {error}");
        }
    }

    #[test]
    fn same_key_on_different_screens_is_allowed() {
        let defaults =
            char_map(&[("edit-connection", 'e'), ("sql-editor", 'e')]);
        let errors = defaults.validate(
            &defaults,
            &[("list", &["edit-connection"]), ("db", &["sql-editor"])],
            &[],
        );
        assert_eq!(errors, Vec::<String>::new());
    }

    #[test]
    fn swapped_keys_are_valid() {
        let defaults = char_map(&[("commit", 's'), ("delete-row", 'D')]);
        let mut km = defaults.clone();
        km.set("commit", KeyCode::Char('D'));
        km.set("delete-row", KeyCode::Char('s'));
        let errors = km.validate(
            &defaults,
            &[("table", &["commit", "delete-row"])],
            &[],
        );
        assert_eq!(errors, Vec::<String>::new());
    }

    #[test]
    fn remap_onto_fixed_key_is_an_error() {
        let defaults = char_map(&[("search", '/')]);
        let mut km = defaults.clone();
        km.set("search", KeyCode::Char('j'));
        let errors = km.validate(&defaults, &[], &[KeyCode::Char('j')]);
        assert_eq!(errors.len(), 1, "{errors:?}");
        let error = errors.first().unwrap();
        assert!(error.contains("search") && error.contains("'j'"), "{error}");
    }

    #[test]
    fn default_on_a_fixed_key_is_allowed() {
        let defaults = char_map(&[("watch", 'w')]);
        let errors = defaults.validate(&defaults, &[], &[KeyCode::Char('w')]);
        assert_eq!(errors, Vec::<String>::new());
    }
}
