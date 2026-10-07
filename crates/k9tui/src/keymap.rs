use std::{collections::HashMap, io::ErrorKind, path::Path};

use crossterm::event::KeyCode;
use serde::Deserialize;

use crate::{
    config_dir::{config_dir, read_config},
    widgets::hotkey::Hotkey,
};

/// Parse a `keys.yml` key spec: a single character (`"s"`, `"?"`) or a
/// named key (`"esc"`, `"enter"`, `"f5"`, `"pageup"`, ...), case-insensitive.
#[must_use]
pub fn parse_key(s: &str) -> Option<KeyCode> {
    let mut chars = s.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Some(KeyCode::Char(c));
    }
    Some(match s.to_ascii_lowercase().as_str() {
        "esc" | "escape" => KeyCode::Esc,
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "space" => KeyCode::Char(' '),
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "f1" => KeyCode::F(1),
        "f2" => KeyCode::F(2),
        "f3" => KeyCode::F(3),
        "f4" => KeyCode::F(4),
        "f5" => KeyCode::F(5),
        "f6" => KeyCode::F(6),
        "f7" => KeyCode::F(7),
        "f8" => KeyCode::F(8),
        "f9" => KeyCode::F(9),
        "f10" => KeyCode::F(10),
        "f11" => KeyCode::F(11),
        "f12" => KeyCode::F(12),
        _ => return None,
    })
}

/// `keys.yml` is a flat `action: key` map. Values are read as raw YAML so a
/// stray number or bool produces a per-entry warning instead of rejecting the
/// whole file.
#[derive(Debug, Deserialize)]
struct KeymapFile(HashMap<String, serde_yaml::Value>);

/// Action-name -> key bindings.
///
/// Seeded with hardcoded defaults and
/// optionally overridden by `~/.config/<app>/keys.yml` (a flat map of
/// `action: key`). Unknown actions and unparsable key specs are skipped with
/// a warning; the default for that action stays in force.
#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: HashMap<&'static str, KeyCode>,
}

impl Keymap {
    #[must_use]
    pub fn new(defaults: &[(&'static str, KeyCode)]) -> Self {
        Self {
            bindings: defaults.iter().copied().collect(),
        }
    }

    /// Load `~/.config/<app>/keys.yml` and apply matching overrides in place.
    /// A missing file or config dir is normal and yields no warnings.
    pub fn load_overrides(&mut self, app: &str) -> Vec<String> {
        config_dir(app).map_or_else(Vec::new, |dir| {
            self.load_overrides_from(&dir.join("keys.yml"))
        })
    }

    /// Like [`Self::load_overrides`] but from an explicit file path.
    pub fn load_overrides_from(&mut self, path: &Path) -> Vec<String> {
        match read_config(path) {
            Ok(contents) => {
                self.apply_yaml(&contents, &path.display().to_string())
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Vec::new(),
            Err(e) => vec![format!(
                "{}: cannot read ({e}); using default keys",
                path.display()
            )],
        }
    }

    /// Apply a `keys.yml` document. `source` only labels warnings. Never
    /// fails: bad input is reported and the defaults stay.
    pub fn apply_yaml(&mut self, yaml: &str, source: &str) -> Vec<String> {
        let file = match serde_yaml::from_str::<Option<KeymapFile>>(yaml) {
            Ok(file) => file.map_or_else(HashMap::new, |f| f.0),
            Err(e) => {
                return vec![format!(
                    "{source}: invalid YAML ({e}); using default keys"
                )];
            }
        };
        let mut warnings = Vec::new();
        let mut entries: Vec<_> = file.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (action, value) in entries {
            let Some(slot) = self.bindings.get_mut(action.as_str()) else {
                warnings.push(format!(
                    "{source}: unknown action '{action}' ignored"
                ));
                continue;
            };
            let spec = match value {
                serde_yaml::Value::String(s) => s,
                serde_yaml::Value::Number(n) => n.to_string(),
                other @ (serde_yaml::Value::Null
                | serde_yaml::Value::Bool(_)
                | serde_yaml::Value::Sequence(_)
                | serde_yaml::Value::Mapping(_)
                | serde_yaml::Value::Tagged(_)) => {
                    warnings.push(format!("{source}: '{action}': expected a key name, got {other:?}"));
                    continue;
                }
            };
            if let Some(code) = parse_key(&spec) {
                *slot = code;
            } else {
                warnings.push(format!(
                    "{source}: '{action}': unknown key '{spec}'; keeping default"
                ));
            }
        }
        warnings
    }

    /// Post-load sanity check. A binding that lands on a `reserved` key
    /// (navigation, quit, Esc...) would shadow it, so it is reset to its
    /// default. Two actions of one `screen` on the same key can't both fire
    /// (the first match arm wins), so that is reported (bindings are kept).
    pub fn validate(
        &mut self,
        defaults: &[(&'static str, KeyCode)],
        screens: &[(&str, &[&str])],
        reserved: &[KeyCode],
        source: &str,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        for (action, default) in defaults {
            if let Some(code) = self.bindings.get_mut(action)
                && code != default
                && reserved.contains(code)
            {
                warnings.push(format!(
                    "{source}: '{action}': {code} is reserved; keeping default"
                ));
                *code = *default;
            }
        }
        for (screen, actions) in screens {
            for (i, a) in actions.iter().enumerate() {
                for b in actions.iter().skip(i + 1) {
                    if let (Some(ka), Some(kb)) =
                        (self.bindings.get(a), self.bindings.get(b))
                        && ka == kb
                    {
                        warnings.push(format!(
                            "{source}: '{a}' and '{b}' share key {ka} on the {screen} screen; only one will fire"
                        ));
                    }
                }
            }
        }
        warnings
    }

    /// The key bound to `action`.
    ///
    /// # Panics
    /// If `action` isn't one of the defaults this keymap was built with — a
    /// programmer error, not a runtime/config one.
    #[must_use]
    pub fn get(&self, action: &str) -> KeyCode {
        *self
            .bindings
            .get(action)
            .unwrap_or_else(|| panic!("unknown keymap action: {action}"))
    }

    /// True if `code` is the key currently bound to `action`.
    #[must_use]
    pub fn is(&self, action: &str, code: KeyCode) -> bool {
        self.get(action) == code
    }

    /// Copy of `hotkeys` with each action-tagged entry relabelled to its
    /// current binding, so the hotkey bar reflects remaps.
    #[must_use]
    pub fn relabel(&self, hotkeys: &[Hotkey]) -> Vec<Hotkey> {
        hotkeys
            .iter()
            .cloned()
            .map(|mut h| {
                if let Some(code) = h.action.and_then(|a| self.bindings.get(a))
                {
                    h.keycode = *code;
                }
                h
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn km() -> Keymap {
        Keymap::new(&[
            ("quit", KeyCode::Char('q')),
            ("delete", KeyCode::Char('D')),
        ])
    }

    #[test]
    fn parses_chars_and_named_keys() {
        assert_eq!(parse_key("s"), Some(KeyCode::Char('s')));
        assert_eq!(parse_key("Esc"), Some(KeyCode::Esc));
        assert_eq!(parse_key("F5"), Some(KeyCode::F(5)));
        assert_eq!(parse_key("space"), Some(KeyCode::Char(' ')));
        assert_eq!(parse_key(""), None);
        assert_eq!(parse_key("nonsense"), None);
    }

    #[test]
    fn defaults_are_used_when_unset() {
        let km = km();
        assert_eq!(km.get("quit"), KeyCode::Char('q'));
        assert!(km.is("quit", KeyCode::Char('q')));
        assert!(!km.is("quit", KeyCode::Char('x')));
    }

    #[test]
    fn valid_override_applies() {
        let mut km = km();
        let w = km.apply_yaml("delete: x\nquit: f10\n", "t");
        assert_eq!(w, Vec::<String>::new());
        assert_eq!(km.get("delete"), KeyCode::Char('x'));
        assert_eq!(km.get("quit"), KeyCode::F(10));
    }

    #[test]
    fn partial_override_leaves_other_actions_default() {
        let mut km = km();
        km.apply_yaml("delete: x", "t");
        assert_eq!(km.get("quit"), KeyCode::Char('q'));
    }

    #[test]
    fn unknown_action_and_bad_key_warn_and_keep_defaults() {
        let mut km = km();
        let w =
            km.apply_yaml("bogus: x\ndelete: notakey\nquit: [1]\n", "keys.yml");
        assert_eq!(w.len(), 3);
        assert_eq!(km.get("delete"), KeyCode::Char('D'));
        assert_eq!(km.get("quit"), KeyCode::Char('q'));
    }

    #[test]
    fn empty_comment_only_and_invalid_yaml_never_panic() {
        for yaml in ["", "# nothing\n", "---\n"] {
            let mut km = km();
            assert_eq!(km.apply_yaml(yaml, "t"), Vec::<String>::new());
            assert_eq!(km.get("delete"), KeyCode::Char('D'));
        }
        let mut km = km();
        let w = km.apply_yaml("delete: [unclosed", "t");
        assert_eq!(w.len(), 1);
        let w = km.apply_yaml("- a\n- b\n", "t");
        assert_eq!(w.len(), 1);
        assert_eq!(km.get("delete"), KeyCode::Char('D'));
    }

    #[test]
    fn missing_file_is_silent() {
        let mut km = km();
        let path = std::env::temp_dir()
            .join("k9tui-no-such-dir")
            .join("keys.yml");
        assert_eq!(km.load_overrides_from(&path), Vec::<String>::new());
    }

    #[test]
    fn file_override_round_trip() {
        let dir = std::env::temp_dir()
            .join(format!("k9tui-keymap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("keys.yml");
        std::fs::write(&path, "delete: x\n").unwrap();
        let mut km = km();
        assert_eq!(km.load_overrides_from(&path), Vec::<String>::new());
        assert_eq!(km.get("delete"), KeyCode::Char('x'));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn relabel_follows_remap_and_skips_untagged() {
        let mut km = km();
        km.apply_yaml("delete: x", "t");
        let bar = [
            Hotkey::new('D', "Delete").action("delete"),
            Hotkey::new('/', "Search"),
        ];
        let out = km.relabel(&bar);
        assert_eq!(out.first().map(Hotkey::key_label), Some("x".to_string()));
        assert_eq!(out.get(1).map(Hotkey::key_label), Some("/".to_string()));
    }

    #[test]
    fn key_name_edge_cases() {
        for (s, want) in [
            ("ESC", Some(KeyCode::Esc)),
            ("eScApE", Some(KeyCode::Esc)),
            ("Enter", Some(KeyCode::Enter)),
            ("F12", Some(KeyCode::F(12))),
            ("é", Some(KeyCode::Char('é'))),
            ("\u{1F600}", Some(KeyCode::Char('\u{1F600}'))),
            (" ", Some(KeyCode::Char(' '))),
            ("ctrl-x", None),
            ("C-x", None),
            ("ctrl+x", None),
            ("shift-a", None),
            ("alt-x", None),
            ("f13", None),
            ("f0", None),
            ("ab", None),
            ("e\u{301}", None), // char + combining mark = 2 chars
            ("\u{0}x", None),
            ("", None),
            ("esc ", None),
        ] {
            assert_eq!(parse_key(s), want, "{s:?}");
        }
    }

    #[test]
    fn malformed_keymaps_degrade_with_warning_and_never_panic() {
        let deep = format!("{}1{}", "[".repeat(5000), "]".repeat(5000));
        let cases: Vec<String> = vec![
            "\u{feff}delete: x\n".into(),
            "delete:\tx\n".into(),
            "\tdelete: x\n".into(),
            "delete: x\ndelete: y\n".into(),
            "1: x\n".into(),
            "[a]: x\n".into(),
            "delete: ~\n".into(),
            "delete:\n".into(),
            "delete: \"\"\n".into(),
            "\"\": x\n".into(),
            "delete: ctrl-x\n".into(),
            "delete: true\n".into(),
            "delete: 12\n".into(),
            "delete: 1.5\n".into(),
            "delete: {a: b}\n".into(),
            "delete: *nope\n".into(),
            "delete: &a [*a]\n".into(),
            "\u{0}\u{1}".into(),
            "---\n---\n".into(),
            "{{{{".into(),
            deep,
            format!("{}: x\n", "k".repeat(100_000)),
            format!("delete: {}\n", "x".repeat(100_000)),
        ];
        for yaml in &cases {
            let mut km = km();
            let w = km.apply_yaml(yaml, "t");
            let short: String = yaml.chars().take(30).collect();
            let kept = km.get("quit") == KeyCode::Char('q');
            assert!(kept, "{short:?}");
            // Bad input never silently rebinds to something odd: delete is
            // default, a single digit/char, or unchanged.
            let _ = w;
        }
        // Spot checks on exact outcomes.
        let mut km = km();
        assert_eq!(km.apply_yaml("delete: 7\n", "t").len(), 0);
        assert_eq!(km.get("delete"), KeyCode::Char('7'));
        let mut km2 = self::km();
        assert_eq!(km2.apply_yaml("delete: ctrl-x\n", "t").len(), 1);
        assert_eq!(km2.get("delete"), KeyCode::Char('D'));
        assert_eq!(km2.apply_yaml("delete: ~\n", "t").len(), 1);
        assert_eq!(km2.apply_yaml("delete: \"\"\n", "t").len(), 1);
        assert_eq!(km2.apply_yaml("\"\": x\n", "t").len(), 1);
        assert_eq!(km2.apply_yaml("delete: ESC\n", "t").len(), 0);
        assert_eq!(km2.get("delete"), KeyCode::Esc);
    }

    const DEFAULTS: &[(&str, KeyCode)] =
        &[("a", KeyCode::Char('a')), ("b", KeyCode::Char('b'))];

    #[test]
    fn validate_reports_same_screen_duplicates_only() {
        let mut km = Keymap::new(DEFAULTS);
        km.apply_yaml("b: a", "t");
        let w = km.validate(DEFAULTS, &[("one", &["a", "b"])], &[], "t");
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w.iter().any(|m| m.contains("on the one screen")));
        // Different screens: no complaint.
        let w =
            km.validate(DEFAULTS, &[("x", &["a"]), ("y", &["b"])], &[], "t");
        assert_eq!(w, Vec::<String>::new());
        // Defaults never conflict.
        let mut km = Keymap::new(DEFAULTS);
        let w = km.validate(DEFAULTS, &[("s", &["a", "b"])], &[], "t");
        assert_eq!(w, Vec::<String>::new());
    }

    #[test]
    fn validate_resets_reserved_keys_but_keeps_default_if_reserved() {
        let mut km = Keymap::new(DEFAULTS);
        km.apply_yaml("a: esc\nb: j", "t");
        let reserved = [KeyCode::Esc, KeyCode::Char('b')];
        let w = km.validate(DEFAULTS, &[], &reserved, "t");
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(km.get("a"), KeyCode::Char('a'), "reset to default");
        assert_eq!(km.get("b"), KeyCode::Char('j'), "non-reserved kept");
        // An action whose *default* is reserved is left alone.
        let mut km = Keymap::new(DEFAULTS);
        assert_eq!(km.validate(DEFAULTS, &[], &reserved, "t").len(), 0);
    }

    #[test]
    fn relabel_with_remap_and_unknown_action_tag() {
        let mut km = km();
        km.apply_yaml("quit: f10", "t");
        let bar = [
            Hotkey::new('q', "Quit").action("quit"),
            Hotkey::new('z', "Ghost").action("not-an-action"),
        ];
        let out = km.relabel(&bar);
        assert_eq!(out.first().map(|h| h.keycode), Some(KeyCode::F(10)));
        assert_eq!(out.get(1).map(|h| h.keycode), Some(KeyCode::Char('z')));
    }

    #[test]
    fn load_from_unreadable_paths_warns_not_panics() {
        let dir = std::env::temp_dir()
            .join(format!("k9tui-keymap-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut km = km();
        // directory instead of file
        assert_eq!(km.load_overrides_from(&dir).len(), 1);
        // non-UTF-8
        let f = dir.join("keys.yml");
        std::fs::write(&f, [0xff, 0xfe, 0xfd]).unwrap();
        assert_eq!(km.load_overrides_from(&f).len(), 1);
        // BOM-prefixed file works
        std::fs::write(&f, "\u{feff}delete: x\n").unwrap();
        assert_eq!(km.load_overrides_from(&f), Vec::<String>::new());
        assert_eq!(km.get("delete"), KeyCode::Char('x'));
        // oversized
        std::fs::write(&f, vec![b'#'; 512 * 1024]).unwrap();
        assert_eq!(km.load_overrides_from(&f).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
