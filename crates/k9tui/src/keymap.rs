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
        "back-tab" => KeyCode::BackTab,
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

/// Actions live at the same time on one screen.
#[derive(Debug, Clone)]
pub struct ScreenActions {
    pub name: String,
    pub actions: Vec<String>,
    /// Plain characters must reach a text buffer here, so can't be bound.
    pub typing: bool,
}

/// Current keys for each named action. The first key is the primary.
#[derive(Debug, Clone, Default)]
pub struct Keymap {
    bindings: HashMap<String, Vec<KeyCode>>,
    screens: Vec<ScreenActions>,
}

impl Keymap {
    #[must_use]
    pub fn new(
        defaults: impl IntoIterator<Item = (String, Vec<KeyCode>)>,
    ) -> Self {
        Self {
            bindings: defaults.into_iter().collect(),
            screens: Vec::new(),
        }
    }

    /// Every key bound to `action`.
    ///
    /// # Panics
    ///
    /// If `action` is unknown, which is a programmer error rather than a
    /// config error: user input only reaches the keymap through [`Self::set`].
    #[must_use]
    pub fn keys(&self, action: &str) -> &[KeyCode] {
        self.bindings
            .get(action)
            .unwrap_or_else(|| panic!("unknown keymap action: {action}"))
    }

    /// The primary key bound to `action`.
    ///
    /// # Panics
    ///
    /// If `action` is unknown; see [`Self::keys`].
    #[must_use]
    pub fn get(&self, action: &str) -> KeyCode {
        self.keys(action).first().copied().unwrap_or(KeyCode::Null)
    }

    /// The primary key for `action`, as the hotkey bar shows it.
    ///
    /// # Panics
    ///
    /// If `action` is unknown; see [`Self::keys`].
    #[must_use]
    pub fn label(&self, action: &str) -> String {
        Hotkey::code(self.get(action), "").key_label()
    }

    /// Every key for `action`, as help shows them.
    ///
    /// # Panics
    ///
    /// If `action` is unknown; see [`Self::keys`].
    #[must_use]
    pub fn labels(&self, action: &str) -> String {
        self.keys(action)
            .iter()
            .map(|code| Hotkey::code(*code, "").key_label())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    /// True if `code` is one of the keys bound to `action`.
    #[must_use]
    pub fn is(&self, action: &str, code: KeyCode) -> bool {
        self.keys(action).contains(&code)
    }

    /// True if `code` fires `action` on `screen`. A secondary key gives way
    /// to another action's primary key on the same screen.
    #[must_use]
    pub fn fires(&self, screen: &str, action: &str, code: KeyCode) -> bool {
        let keys = self.keys(action);
        if !keys.contains(&code) {
            return false;
        }
        if keys.first() == Some(&code) {
            return true;
        }
        self.screens
            .iter()
            .find(|candidate| candidate.name == screen)
            .is_none_or(|found| {
                !found.actions.iter().any(|other| {
                    other != action && self.primary(other) == Some(code)
                })
            })
    }

    fn primary(&self, action: &str) -> Option<KeyCode> {
        self.bindings.get(action)?.first().copied()
    }

    /// True if any action is currently bound to `code`.
    #[must_use]
    pub fn binds(&self, code: KeyCode) -> bool {
        self.bindings.values().flatten().any(|bound| *bound == code)
    }

    /// Bind `action` to one key. Returns false, changing nothing, if it is
    /// unknown.
    pub fn set(&mut self, action: &str, code: KeyCode) -> bool {
        self.set_keys(action, vec![code])
    }

    /// Replace `action`'s keys. Returns false, changing nothing, if it is
    /// unknown.
    pub fn set_keys(&mut self, action: &str, keys: Vec<KeyCode>) -> bool {
        self.bindings
            .get_mut(action)
            .map(|bound| *bound = keys)
            .is_some()
    }

    pub fn set_screens(&mut self, screens: Vec<ScreenActions>) {
        self.screens = screens;
    }

    /// Check final bindings: every action has a key, no two primaries or two
    /// secondaries share a key on one screen, and `typing` screens leave
    /// plain characters alone.
    #[must_use]
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut actions: Vec<_> = self.bindings.iter().collect();
        actions.sort_unstable_by_key(|(action, _)| action.as_str());
        for (action, keys) in actions {
            if keys.is_empty() {
                errors.push(format!("'{action}' has no keys"));
            }
        }
        for screen in &self.screens {
            let known: Vec<_> = screen
                .actions
                .iter()
                .filter_map(|a| Some((a, self.bindings.get(a)?)))
                .collect();
            for (i, (a, a_keys)) in known.iter().enumerate() {
                if screen.typing
                    && let Some(code) = a_keys
                        .iter()
                        .find(|code| matches!(code, KeyCode::Char(_)))
                {
                    errors.push(format!(
                            "'{a}': {} is a plain character on the {} screen, where typing needs it",
                            key_name(*code),
                            screen.name
                    ));
                }
                for (b, b_keys) in known.iter().skip(i + 1) {
                    if let Some(code) = shared(a_keys, b_keys) {
                        errors.push(format!(
                            "'{a}' and '{b}' both use {} on the {} screen",
                            key_name(code),
                            screen.name
                        ));
                    }
                }
            }
        }
        errors
    }
}

/// The key two actions can't both have: equal primaries, or a secondary in
/// common.
fn shared(a: &[KeyCode], b: &[KeyCode]) -> Option<KeyCode> {
    if !a.is_empty() && a.first() == b.first() {
        return a.first().copied();
    }
    a.iter()
        .skip(1)
        .find(|code| b.iter().skip(1).any(|other| other == *code))
        .copied()
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;

    use super::*;

    fn char_map(pairs: &[(&'static str, char)]) -> Keymap {
        Keymap::new(
            pairs
                .iter()
                .map(|(a, c)| ((*a).to_owned(), vec![KeyCode::Char(*c)])),
        )
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

    fn list_map(pairs: &[(&str, &[KeyCode])]) -> Keymap {
        Keymap::new(pairs.iter().map(|(a, k)| ((*a).to_owned(), k.to_vec())))
    }

    fn screen(name: &str, actions: &[&str], typing: bool) -> ScreenActions {
        ScreenActions {
            name: name.to_owned(),
            actions: actions.iter().map(|a| (*a).to_owned()).collect(),
            typing,
        }
    }

    fn watch_map() -> Keymap {
        let mut km = list_map(&[
            ("watch", &[KeyCode::Char('w')]),
            ("right", &[KeyCode::Char('l'), KeyCode::Char('w')]),
        ]);
        km.set_screens(vec![screen("sql", &["watch", "right"], false)]);
        km
    }

    #[test]
    fn fires_any_listed_key() {
        let km = watch_map();
        assert!(km.fires("sql", "right", KeyCode::Char('l')));
        assert!(km.fires("elsewhere", "right", KeyCode::Char('w')));
        assert!(!km.fires("sql", "right", KeyCode::Char('z')));
    }

    #[test]
    fn primary_beats_another_actions_secondary() {
        let km = watch_map();
        assert!(km.fires("sql", "watch", KeyCode::Char('w')));
        assert!(!km.fires("sql", "right", KeyCode::Char('w')));
    }

    #[test]
    fn secondary_fires_when_primary_moves_away() {
        let mut km = watch_map();
        km.set("watch", KeyCode::Char('z'));
        assert!(km.fires("sql", "right", KeyCode::Char('w')));
    }

    #[test]
    fn get_and_labels_use_the_list() {
        let km = watch_map();
        assert_eq!(km.get("right"), KeyCode::Char('l'));
        assert_eq!(km.keys("right").len(), 2);
        assert_eq!(km.label("right"), "l");
        assert_eq!(km.labels("right"), "l / w");
    }

    #[test]
    fn two_primaries_conflict() {
        let mut km = char_map(&[("commit", 's'), ("delete-row", 'D')]);
        km.set("commit", KeyCode::Char('D'));
        km.set_screens(vec![screen("table", &["commit", "delete-row"], false)]);
        let errors = km.validate();
        assert_eq!(errors.len(), 1, "{errors:?}");
        let error = errors.first().unwrap();
        for part in ["commit", "delete-row", "'D'", "table"] {
            assert!(error.contains(part), "{part} missing from {error}");
        }
    }

    #[test]
    fn two_secondaries_conflict() {
        let mut km = list_map(&[
            ("a", &[KeyCode::Char('x'), KeyCode::Char('z')]),
            ("b", &[KeyCode::Char('y'), KeyCode::Char('z')]),
        ]);
        km.set_screens(vec![screen("s", &["a", "b"], false)]);
        assert_eq!(km.validate().len(), 1);
    }

    #[test]
    fn primary_over_secondary_is_not_a_conflict() {
        assert_eq!(watch_map().validate(), Vec::<String>::new());
    }

    #[test]
    fn same_key_on_different_screens_is_allowed() {
        let mut km = char_map(&[("edit-connection", 'e'), ("sql-editor", 'e')]);
        km.set_screens(vec![
            screen("list", &["edit-connection"], false),
            screen("db", &["sql-editor"], false),
        ]);
        assert_eq!(km.validate(), Vec::<String>::new());
    }

    #[test]
    fn swapped_keys_are_valid() {
        let mut km = char_map(&[("commit", 's'), ("delete-row", 'D')]);
        km.set("commit", KeyCode::Char('D'));
        km.set("delete-row", KeyCode::Char('s'));
        km.set_screens(vec![screen("table", &["commit", "delete-row"], false)]);
        assert_eq!(km.validate(), Vec::<String>::new());
    }

    #[test]
    fn empty_list_is_an_error() {
        let mut km = char_map(&[("quit", 'q')]);
        assert!(km.set_keys("quit", Vec::new()));
        let errors = km.validate();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors.first().unwrap().contains("quit"));
    }

    #[test]
    fn typing_screen_rejects_plain_characters() {
        let mut km = char_map(&[("cell-next", 'n')]);
        km.set_screens(vec![screen("cell-edit", &["cell-next"], true)]);
        let errors = km.validate();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors.first().unwrap().contains("cell-next"));
    }

    #[test]
    fn typing_screen_allows_named_keys() {
        let mut km = list_map(&[("cell-next", &[KeyCode::Tab])]);
        km.set_screens(vec![screen("cell-edit", &["cell-next"], true)]);
        assert_eq!(km.validate(), Vec::<String>::new());
    }

    #[test]
    fn parse_key_reads_back_tab() {
        assert_eq!(parse_key("back-tab"), Some(KeyCode::BackTab));
    }
}
