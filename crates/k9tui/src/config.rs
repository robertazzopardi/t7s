use std::{
    collections::BTreeMap,
    env,
    ffi::OsStr,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process,
};

use crossterm::event::KeyCode;
use directories::BaseDirs;
use ratatui::style::Color;
use toml::{Table, Value};

use crate::keymap::{Keymap, ScreenActions, parse_key};

/// Built-in defaults, applied through the same path as a user's file.
const DEFAULT_CONFIG: &str = include_str!("default_config.toml");

// TODO: make navigation keys remappable (j k g G, h l b w 0 $, digits, #, Esc).
/// Actions every app has, so they can be set in `[t7s.keys]`.
pub const SHARED_ACTIONS: [&str; 3] = ["quit", "search", "command"];

const COLOUR_NAMES: [&str; 15] = [
    "background",
    "border",
    "text",
    "focus",
    "on-focus",
    "selection-fg",
    "selection-bg",
    "bg-alt",
    "draft",
    "multi-select",
    "info",
    "error",
    "success",
    "warning",
    "link",
];

/// What an app tells the config loader about itself.
pub struct AppSpec {
    /// The app's table name, such as `d7s`.
    pub name: &'static str,
    /// The app's own actions and their default keys.
    pub actions: &'static [(&'static str, KeyCode)],
    /// Actions live at the same time, per screen, for conflict checks.
    pub screens: &'static [(&'static str, &'static [&'static str])],
    /// Keys an action can't be remapped onto.
    pub fixed: &'static [KeyCode],
}

/// The merged result for one app.
#[derive(Debug, Clone)]
pub struct Config {
    /// Reserved: stored, but layout does not change yet.
    pub skin: Option<String>,
    pub theme: Option<String>,
    pub colours: BTreeMap<String, Color>,
    pub keymap: Keymap,
    /// Problems that don't stop startup, for the status line.
    pub warnings: Vec<String>,
}

fn parse_colour(spec: &str) -> Option<Color> {
    if let Some(hex) = spec.strip_prefix('#') {
        // `from_str_radix` accepts a leading '+', so check digits explicitly.
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |range| u8::from_str_radix(hex.get(range)?, 16).ok();
        return Some(Color::Rgb(
            channel(0..2)?,
            channel(2..4)?,
            channel(4..6)?,
        ));
    }
    Some(match spec.to_ascii_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "dark-gray" | "dark-grey" => Color::DarkGray,
        "light-red" => Color::LightRed,
        "light-green" => Color::LightGreen,
        "light-yellow" => Color::LightYellow,
        "light-blue" => Color::LightBlue,
        "light-magenta" => Color::LightMagenta,
        "light-cyan" => Color::LightCyan,
        "white" => Color::White,
        _ => return None,
    })
}

/// Merge state while the levels are applied in order.
struct Merge<'a> {
    spec: &'a AppSpec,
    config: Config,
    errors: Vec<String>,
}

impl Merge<'_> {
    fn apply_level(&mut self, name: &str, level: &Table) {
        if let Some(value) = level.get("theme") {
            if let Some(theme) = value.as_str() {
                self.config.theme = Some(theme.to_string());
                // The more specific level's theme replaces inherited colours.
                self.config.colours.clear();
            } else {
                self.warn_shape(name, "theme", "a string");
            }
        }
        for (key, value) in level {
            match key.as_str() {
                "theme" => {}
                "skin" => {
                    if let Some(skin) = value.as_str() {
                        self.config.skin = Some(skin.to_string());
                    } else {
                        self.warn_shape(name, "skin", "a string");
                    }
                }
                "colours" => {
                    if let Some(colours) = value.as_table() {
                        self.apply_colours(name, colours);
                    } else {
                        self.warn_shape(name, "colours", "a table");
                    }
                }
                "keys" => {
                    if let Some(keys) = value.as_table() {
                        self.apply_keys(name, keys);
                    } else {
                        self.errors
                            .push(format!("[{name}] 'keys' must be a table"));
                    }
                }
                other => self.config.warnings.push(format!(
                    "[{name}] unknown setting '{other}' ignored"
                )),
            }
        }
    }

    fn warn_shape(&mut self, name: &str, key: &str, expected: &str) {
        self.config
            .warnings
            .push(format!("[{name}] '{key}' must be {expected}; ignored"));
    }

    fn apply_colours(&mut self, name: &str, colours: &Table) {
        for (key, value) in colours {
            if !COLOUR_NAMES.contains(&key.as_str()) {
                self.config.warnings.push(format!(
                    "[{name}.colours] unknown colour '{key}' ignored"
                ));
            } else if let Some(colour) = value.as_str().and_then(parse_colour) {
                self.config.colours.insert(key.clone(), colour);
            } else {
                self.config.warnings.push(format!(
                    "[{name}.colours] '{key}': bad colour {value}; keeping \
                     inherited"
                ));
            }
        }
    }

    fn apply_keys(&mut self, name: &str, keys: &Table) {
        for (action, value) in keys {
            let shared = SHARED_ACTIONS.contains(&action.as_str());
            let own = name == self.spec.name
                && self.spec.actions.iter().any(|(a, _)| a == action);
            if !shared && !own {
                self.config.warnings.push(format!(
                    "[{name}.keys] unknown action '{action}' ignored"
                ));
            } else if let Some(code) = value.as_str().and_then(parse_key) {
                self.config.keymap.set(action, code);
            } else {
                self.errors
                    .push(format!("[{name}.keys] '{action}': bad key {value}"));
            }
        }
    }

    fn apply_table(&mut self, file: &Table, name: &str) {
        match file.get(name) {
            None => {}
            Some(Value::Table(level)) => self.apply_level(name, level),
            Some(_) => self.errors.push(format!("'{name}' must be a table")),
        }
    }
}

/// Merge built-in defaults, then `[t7s]`, then `[<app>]` from `file`.
/// `source` labels errors. A missing file is `None`.
///
/// # Errors
///
/// Every problem that could change how the app behaves: bad TOML, bad key
/// names, key conflicts and remaps onto fixed keys. Lesser problems end up in
/// [`Config::warnings`] instead.
///
/// # Panics
///
/// If the built-in default config is invalid.
pub fn resolve(
    file: Option<&str>,
    spec: &AppSpec,
    source: &str,
) -> Result<Config, Vec<String>> {
    let builtin: Table = DEFAULT_CONFIG
        .parse()
        .expect("built-in config is valid TOML");
    let keymap = Keymap::new(
        spec.actions
            .iter()
            .map(|(action, code)| ((*action).to_owned(), vec![*code]))
            .chain(
                SHARED_ACTIONS
                    .map(|action| (action.to_owned(), vec![KeyCode::Null])),
            ),
    );
    let mut merge = Merge {
        spec,
        config: Config {
            skin: None,
            theme: None,
            colours: BTreeMap::new(),
            keymap,
            warnings: Vec::new(),
        },
        errors: Vec::new(),
    };
    merge.apply_table(&builtin, "t7s");
    assert!(
        merge.errors.is_empty() && merge.config.warnings.is_empty(),
        "built-in config is invalid: {:?}",
        merge.errors
    );

    if let Some(text) = file {
        match text.parse::<Table>() {
            Ok(table) => {
                merge.apply_table(&table, "t7s");
                merge.apply_table(&table, spec.name);
            }
            Err(error) => return Err(vec![format!("{source}: {error}")]),
        }
    }
    merge.config.keymap.set_screens(
        spec.screens
            .iter()
            .map(|(name, actions)| ScreenActions {
                name: (*name).to_owned(),
                actions: actions.iter().map(|a| (*a).to_owned()).collect(),
                typing: false,
            })
            .collect(),
    );
    merge.errors.extend(merge.config.keymap.validate());
    if merge.errors.is_empty() {
        Ok(merge.config)
    } else {
        Err(merge
            .errors
            .iter()
            .map(|error| format!("{source}: {error}"))
            .collect())
    }
}

/// Built-in config only; never reads a file, so tests and constructors can
/// use it freely.
///
/// # Panics
///
/// If the app's own defaults conflict, which is a programmer error.
#[must_use]
pub fn defaults(spec: &AppSpec) -> Config {
    resolve(None, spec, "built-in config")
        .unwrap_or_else(|errors| panic!("{}", errors.join("\n")))
}

/// `$XDG_CONFIG_HOME/t7s/config.toml` if set and non-empty, otherwise
/// `<home>/.config/t7s/config.toml`, the same on every platform.
#[must_use]
pub fn config_path(xdg: Option<&OsStr>, home: &Path) -> PathBuf {
    xdg.filter(|dir| !dir.is_empty())
        .map_or_else(|| home.join(".config"), PathBuf::from)
        .join("t7s")
        .join("config.toml")
}

/// Read and resolve the user's config file. A missing file is not an error.
///
/// # Errors
///
/// See [`resolve`]; also an unreadable file.
pub fn load(spec: &AppSpec) -> Result<Config, Vec<String>> {
    let Some(dirs) = BaseDirs::new() else {
        return Ok(defaults(spec));
    };
    let path =
        config_path(env::var_os("XDG_CONFIG_HOME").as_deref(), dirs.home_dir());
    load_file(&path, spec)
}

fn load_file(path: &Path, spec: &AppSpec) -> Result<Config, Vec<String>> {
    let source = path.display().to_string();
    match fs::read_to_string(path) {
        Ok(text) => resolve(Some(&text), spec, &source),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            resolve(None, spec, &source)
        }
        Err(error) => Err(vec![format!("{source}: {error}")]),
    }
}

/// [`load`], or print every error to stderr and exit 1. Call before the
/// terminal enters the TUI.
#[must_use]
pub fn load_or_exit(spec: &AppSpec) -> Config {
    load(spec).unwrap_or_else(|errors| {
        for error in errors {
            eprintln!("{}: {error}", spec.name);
        }
        process::exit(1);
    })
}

/// One status-line message for any number of warnings; the full list goes to
/// stderr on exit.
#[must_use]
pub fn summarize(warnings: &[String]) -> Option<String> {
    let (first, rest) = warnings.split_first()?;
    Some(if rest.is_empty() {
        format!("Config: {first}")
    } else {
        format!("Config: {first} (+{} more, see stderr on exit)", rest.len())
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    const SPEC: AppSpec = AppSpec {
        name: "app",
        actions: &[("refresh", KeyCode::Char('r'))],
        screens: &[("main", &["refresh", "quit", "search", "command"])],
        fixed: &[KeyCode::Char('j')],
    };

    fn ok(file: &str) -> Config {
        resolve(Some(file), &SPEC, "cfg.toml").unwrap()
    }

    fn fatal(file: &str) -> String {
        let errors = resolve(Some(file), &SPEC, "cfg.toml").unwrap_err();
        for error in &errors {
            assert!(error.contains("cfg.toml"), "no source in {error}");
        }
        errors.join("\n")
    }

    #[test]
    fn unreadable_file_is_fatal() {
        // A directory exists but can't be read as a file.
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let errors = load_file(dir, &SPEC).unwrap_err();
        assert!(errors.join("\n").contains(env!("CARGO_MANIFEST_DIR")));
    }

    fn c(ch: char) -> KeyCode {
        KeyCode::Char(ch)
    }

    #[test]
    fn no_file_and_empty_file_give_defaults() {
        let none = resolve(None, &SPEC, "cfg.toml").unwrap();
        for config in [none, ok(""), ok("# only a comment\n")] {
            assert_eq!(config.keymap.get("quit"), c('q'));
            assert_eq!(config.keymap.get("search"), c('/'));
            assert_eq!(config.keymap.get("command"), c(':'));
            assert_eq!(config.keymap.get("refresh"), c('r'));
            assert_eq!(config.warnings, Vec::<String>::new());
            assert!(config.colours.is_empty());
            assert_eq!(config.theme, None);
            assert_eq!(config.skin, None);
        }
    }

    #[test]
    fn app_key_overrides_t7s_key_overrides_default() {
        assert_eq!(ok("[t7s.keys]\nquit = \"z\"\n").keymap.get("quit"), c('z'));
        let both = ok("[t7s.keys]\nquit = \"z\"\n[app.keys]\nquit = \"x\"\n");
        assert_eq!(both.keymap.get("quit"), c('x'));
    }

    #[test]
    fn t7s_colours_carry_through_without_app_theme() {
        let config = ok("[t7s.colours]\nborder = \"blue\"\n");
        assert_eq!(config.colours.get("border"), Some(&Color::Blue));
    }

    #[test]
    fn app_colours_override_t7s_colours() {
        let config = ok("[t7s.colours]\nborder = \"blue\"\ntext = \"red\"\n\
             [app.colours]\nborder = \"#ff0000\"\n");
        assert_eq!(config.colours.get("border"), Some(&Color::Rgb(0xff, 0, 0)));
        assert_eq!(config.colours.get("text"), Some(&Color::Red));
    }

    #[test]
    fn app_theme_replaces_inherited_colours() {
        let inherited = "[t7s]\ntheme = \"rose-pine\"\n\
                         [t7s.colours]\nborder = \"blue\"\n\
                         [app]\ntheme = \"github\"\n";
        let config = ok(inherited);
        assert_eq!(config.theme.as_deref(), Some("github"));
        assert_eq!(config.colours.get("border"), None);

        let adjusted = format!("{inherited}[app.colours]\nborder = \"red\"\n");
        assert_eq!(ok(&adjusted).colours.get("border"), Some(&Color::Red));
    }

    #[test]
    fn other_app_tables_are_ignored() {
        let config = ok("[other]\nnonsense = 1\n[other.keys]\nnope = \"!!\"\n");
        assert_eq!(config.warnings, Vec::<String>::new());
    }

    #[test]
    fn skin_is_stored_and_most_specific_wins() {
        assert_eq!(ok("[t7s]\nskin = \"a\"\n").skin.as_deref(), Some("a"));
        let config = ok("[t7s]\nskin = \"a\"\n[app]\nskin = \"b\"\n");
        assert_eq!(config.skin.as_deref(), Some("b"));
    }

    #[test]
    fn syntax_error_is_fatal() {
        fatal("[t7s\n");
    }

    #[test]
    fn duplicate_theme_key_and_table_is_fatal() {
        fatal("[t7s]\ntheme = \"rose-pine\"\n[t7s.theme]\n");
    }

    #[test]
    fn unknown_key_name_is_fatal() {
        let error = fatal("[t7s.keys]\nquit = \"ctrl+s\"\n");
        assert!(
            error.contains("quit") && error.contains("ctrl+s"),
            "{error}"
        );
    }

    #[test]
    fn non_string_key_value_is_fatal() {
        assert!(fatal("[t7s.keys]\nquit = 3\n").contains("quit"));
    }

    #[test]
    fn keys_with_wrong_shape_is_fatal() {
        assert!(fatal("[t7s]\nkeys = \"x\"\n").contains("keys"));
    }

    #[test]
    fn conflict_is_fatal() {
        let error = fatal("[app.keys]\nrefresh = \"q\"\n");
        assert!(
            error.contains("refresh") && error.contains("quit"),
            "{error}"
        );
    }

    #[test]
    fn inherited_conflict_is_fatal_and_app_override_fixes_it() {
        let inherited = "[t7s.keys]\nsearch = \"r\"\n";
        fatal(inherited);
        let fixed = format!("{inherited}[app.keys]\nrefresh = \"z\"\n");
        assert_eq!(ok(&fixed).keymap.get("search"), c('r'));
    }

    #[test]
    fn level_with_wrong_shape_is_fatal() {
        assert!(fatal("t7s = 3\n").contains("t7s"));
        assert!(fatal("app = 3\n").contains("app"));
    }

    #[test]
    fn every_fatal_error_is_reported_at_once() {
        let error =
            fatal("[t7s.keys]\nquit = \"ctrl+s\"\nsearch = \"ctrl+f\"\n");
        assert!(
            error.contains("quit") && error.contains("search"),
            "{error}"
        );
    }

    #[test]
    fn bad_colour_warns_and_keeps_inherited() {
        for bad in ["#12345", "#+12345", "notacolour"] {
            let config = ok(&format!(
                "[t7s.colours]\nborder = \"blue\"\n\
                 [app.colours]\nborder = \"{bad}\"\n"
            ));
            assert_eq!(config.colours.get("border"), Some(&Color::Blue));
            assert_eq!(config.warnings.len(), 1, "{:?}", config.warnings);
            let warning = config.warnings.first().unwrap();
            assert!(warning.contains("border"), "{warning}");
        }
    }

    #[test]
    fn non_string_colour_warns() {
        assert_eq!(ok("[t7s.colours]\nborder = 3\n").warnings.len(), 1);
    }

    #[test]
    fn unknown_colour_name_warns() {
        let config = ok("[t7s.colours]\nnope = \"red\"\n");
        assert!(config.colours.is_empty());
        assert_eq!(config.warnings.len(), 1, "{:?}", config.warnings);
    }

    #[test]
    fn colours_with_wrong_shape_warns() {
        let config = ok("[t7s]\ncolours = 3\n");
        assert_eq!(config.warnings.len(), 1, "{:?}", config.warnings);
    }

    #[test]
    fn unknown_setting_warns() {
        let config = ok("[t7s]\ntypo = 1\n");
        let warning = config.warnings.first().unwrap();
        assert!(warning.contains("typo"), "{warning}");
    }

    #[test]
    fn non_string_theme_and_skin_warn() {
        let config = ok("[t7s]\ntheme = 1\nskin = 2\n");
        assert_eq!(config.theme, None);
        assert_eq!(config.skin, None);
        assert_eq!(config.warnings.len(), 2, "{:?}", config.warnings);
    }

    #[test]
    fn app_action_in_t7s_keys_warns() {
        let config = ok("[t7s.keys]\nrefresh = \"z\"\n");
        assert_eq!(config.keymap.get("refresh"), c('r'));
        let warning = config.warnings.first().unwrap();
        assert!(warning.contains("refresh"), "{warning}");
    }

    #[test]
    fn unknown_action_in_app_keys_warns() {
        let config = ok("[app.keys]\nnope = \"z\"\n");
        assert_eq!(config.warnings.len(), 1, "{:?}", config.warnings);
    }

    #[test]
    fn defaults_never_read_a_file() {
        assert_eq!(defaults(&SPEC).keymap.get("refresh"), c('r'));
    }

    #[test]
    fn config_path_prefers_xdg() {
        assert_eq!(
            config_path(Some(OsStr::new("/xdg")), Path::new("/home/u")),
            Path::new("/xdg/t7s/config.toml")
        );
    }

    #[test]
    fn config_path_treats_empty_xdg_as_unset() {
        assert_eq!(
            config_path(Some(OsStr::new("")), Path::new("/home/u")),
            Path::new("/home/u/.config/t7s/config.toml")
        );
    }

    #[test]
    fn config_path_uses_home_dot_config() {
        assert_eq!(
            config_path(None, Path::new("/home/u")),
            Path::new("/home/u/.config/t7s/config.toml")
        );
    }

    #[test]
    fn summary_is_one_line() {
        assert_eq!(summarize(&[]), None);
        let one = vec!["a".to_string()];
        assert_eq!(summarize(&one).as_deref(), Some("Config: a"));
        let many: Vec<String> = (0..50).map(|i| i.to_string()).collect();
        let summary = summarize(&many).unwrap();
        assert!(summary.contains("+49 more"), "{summary}");
        assert!(
            !summary.contains(';') && !summary.contains('\n'),
            "{summary}"
        );
    }
}
