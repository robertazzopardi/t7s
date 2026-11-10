use std::{
    collections::{BTreeMap, BTreeSet},
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
use serde::{Deserialize, Deserializer};

use crate::{
    keymap::{Keymap, ScreenActions, parse_key},
    theme::Palette,
    theme_source,
    widgets::hotkey::Hotkey,
};

/// Built-in globals, applied through the same path as a user's file.
const GLOBAL_CONFIG: &str = include_str!("config.toml");

/// What an app tells the config loader about itself.
pub struct AppSpec {
    /// The app's table name, such as `d7s`.
    pub name: &'static str,
    /// The app's built-in TOML, merged over the global one.
    pub config: &'static str,
}

/// A TOML string, or a list of strings.
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl From<OneOrMany> for Vec<String> {
    fn from(value: OneOrMany) -> Self {
        match value {
            OneOrMany::One(one) => vec![one],
            OneOrMany::Many(many) => many,
        }
    }
}

fn one_or_many<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    OneOrMany::deserialize(deserializer).map(Into::into)
}

/// One line of a hotkey bar or help section.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Row {
    /// Shows the current keys of `actions`, after `key` if it is set.
    Action {
        #[serde(rename = "action", deserialize_with = "one_or_many")]
        actions: Vec<String>,
        key: Option<String>,
        label: String,
    },
    /// Display text for something Rust handles, such as a Ctrl chord.
    Key { key: String, label: String },
}

impl Row {
    fn actions(&self) -> &[String] {
        match self {
            Self::Action { actions, .. } => actions,
            Self::Key { .. } => &[],
        }
    }
}

/// One titled group of rows in the help table.
#[derive(Debug, Clone, Default)]
pub struct HelpSection {
    pub title: String,
    pub rows: Vec<Row>,
}

/// The bar for one screen and the actions live on it.
#[derive(Debug, Clone, Default)]
pub struct Screen {
    pub hotkeys: Vec<Row>,
    /// Live on this screen but not on its bar.
    pub hidden: Vec<String>,
    /// Plain characters here must reach a text buffer.
    pub typing: bool,
}

impl Screen {
    fn actions(&self, name: &str) -> ScreenActions {
        let mut actions: Vec<String> = self
            .hotkeys
            .iter()
            .flat_map(Row::actions)
            .chain(&self.hidden)
            .cloned()
            .collect();
        actions.sort_unstable();
        actions.dedup();
        ScreenActions {
            name: name.to_owned(),
            actions,
            typing: self.typing,
        }
    }
}

/// The merged result for one app.
#[derive(Debug, Clone)]
pub struct Config {
    /// Reserved: stored, but layout does not change yet.
    pub skin: Option<String>,
    pub theme: Option<String>,
    pub colours: BTreeMap<String, Color>,
    /// A user file set `text`, which then reaches unstyled text too.
    pub user_text: bool,
    pub keymap: Keymap,
    pub screens: BTreeMap<String, Screen>,
    /// Help sections, by name.
    pub help: BTreeMap<String, HelpSection>,
}

impl Config {
    /// The theme's colours, or the defaults, with colour overrides applied.
    #[must_use]
    pub fn palette(&self) -> Palette {
        let mut palette = self
            .theme
            .as_deref()
            .and_then(theme_source::palette)
            .unwrap_or_default();
        for (name, colour) in &self.colours {
            palette.set(name, *colour);
        }
        palette.themed_text |= self.user_text;
        palette
    }
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
        "reset" => Color::Reset,
        _ => return None,
    })
}

#[derive(Deserialize)]
#[serde(try_from = "OneOrMany")]
struct Keys(Vec<KeyCode>);

impl TryFrom<OneOrMany> for Keys {
    type Error = String;

    fn try_from(value: OneOrMany) -> Result<Self, String> {
        Vec::<String>::from(value)
            .iter()
            .map(|key| parse_key(key).ok_or_else(|| format!("bad key '{key}'")))
            .collect::<Result<_, _>>()
            .map(Self)
    }
}

#[derive(Deserialize)]
#[serde(try_from = "String")]
struct Colour(Color);

impl TryFrom<String> for Colour {
    type Error = String;

    fn try_from(spec: String) -> Result<Self, String> {
        parse_colour(&spec)
            .map(Self)
            .ok_or_else(|| format!("bad colour '{spec}'"))
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ScreenLayer {
    hotkeys: Option<Vec<Row>>,
    hidden: Option<Vec<String>>,
    typing: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HelpLayer {
    title: Option<String>,
    rows: Option<Vec<Row>>,
}

/// One `[t7s]` or `[<app>]` table.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Layer {
    theme: Option<String>,
    skin: Option<String>,
    colours: BTreeMap<String, Colour>,
    keys: BTreeMap<String, Keys>,
    screens: BTreeMap<String, ScreenLayer>,
    help: BTreeMap<String, HelpLayer>,
}

type File = BTreeMap<String, Layer>;

/// The action names in `[<name>.keys]` of a built-in file.
fn action_names(file: &File, name: &str) -> BTreeSet<String> {
    file.get(name)
        .map(|layer| layer.keys.keys().cloned().collect())
        .unwrap_or_default()
}

fn colour_names(global: &File) -> BTreeSet<String> {
    global
        .get("t7s")
        .map(|layer| layer.colours.keys().cloned().collect())
        .unwrap_or_default()
}

/// The built-in colours, for widgets drawn before any config loads.
///
/// # Panics
///
/// If the global built-in config is invalid.
#[must_use]
pub fn builtin_palette() -> Palette {
    let mut global: File =
        toml::from_str(GLOBAL_CONFIG).expect("global config is valid");
    let colours = global
        .remove("t7s")
        .expect("global config has [t7s]")
        .colours;
    let mut palette = Palette::BLANK;
    for (name, Colour(colour)) in colours {
        assert!(palette.set(&name, colour), "unknown colour '{name}'");
    }
    palette
}

/// Merge state while the levels are applied in order.
struct Merge<'a> {
    spec: &'a AppSpec,
    config: Config,
    errors: Vec<String>,
    /// Actions from the global built-in file.
    shared: BTreeSet<String>,
    /// Actions from the app's built-in file.
    own: BTreeSet<String>,
    /// Colours from the global built-in file.
    colour_names: BTreeSet<String>,
    /// True while the built-in files apply; they may add screens.
    builtin: bool,
}

impl Merge<'_> {
    fn apply(&mut self, name: &str, layer: Layer) {
        if let Some(theme) = layer.theme {
            if theme_source::palette(&theme).is_none() {
                self.errors
                    .push(format!("[{name}] unknown theme '{theme}'"));
            }
            self.config.theme = Some(theme);
            // A more specific level's theme replaces inherited colours.
            self.config.colours.clear();
            self.config.user_text = false;
        }
        if layer.skin.is_some() {
            self.config.skin = layer.skin;
        }
        for (key, Colour(colour)) in layer.colours {
            if self.colour_names.contains(&key) {
                self.config.user_text |= !self.builtin && key == "text";
                self.config.colours.insert(key, colour);
            } else {
                self.errors
                    .push(format!("[{name}.colours] unknown colour '{key}'"));
            }
        }
        for (action, Keys(keys)) in layer.keys {
            if self.shared.contains(&action)
                || (name == self.spec.name && self.own.contains(&action))
            {
                self.config.keymap.set_keys(&action, keys);
            } else {
                self.errors
                    .push(format!("[{name}.keys] unknown action '{action}'"));
            }
        }
        if name != self.spec.name {
            if !layer.screens.is_empty() || !layer.help.is_empty() {
                self.errors.push(format!(
                    "[{name}] 'screens' and 'help' belong in an app's table"
                ));
            }
            return;
        }
        for (screen, part) in layer.screens {
            self.apply_screen(name, screen, part);
        }
        for (section, part) in layer.help {
            self.apply_help(name, section, part);
        }
    }

    fn check_actions<'r>(
        &mut self,
        context: &str,
        actions: impl Iterator<Item = &'r String>,
    ) {
        for action in actions {
            if !self.shared.contains(action) && !self.own.contains(action) {
                self.errors
                    .push(format!("[{context}] unknown action '{action}'"));
            }
        }
    }

    /// `bar` shows the first action's key, so every action row needs one.
    fn check_rows(&mut self, context: &str, rows: Option<&Vec<Row>>) {
        let empty = |row: &Row| matches!(row, Row::Action { actions, .. } if actions.is_empty());
        if rows.into_iter().flatten().any(empty) {
            self.errors.push(format!("[{context}] row has no action"));
        }
    }

    fn apply_screen(&mut self, name: &str, id: String, part: ScreenLayer) {
        let context = format!("{name}.screens.{id}");
        if !self.builtin && !self.config.screens.contains_key(&id) {
            self.errors.push(format!("[{context}] unknown screen"));
            return;
        }
        self.check_rows(&context, part.hotkeys.as_ref());
        let rows = part.hotkeys.iter().flatten().flat_map(Row::actions);
        self.check_actions(&context, rows.chain(part.hidden.iter().flatten()));
        let screen = self.config.screens.entry(id).or_default();
        if let Some(hotkeys) = part.hotkeys {
            screen.hotkeys = hotkeys;
        }
        if let Some(hidden) = part.hidden {
            screen.hidden = hidden;
        }
        if let Some(typing) = part.typing {
            screen.typing = typing;
        }
    }

    fn apply_help(&mut self, name: &str, id: String, part: HelpLayer) {
        let context = format!("{name}.help.{id}");
        if !self.builtin && !self.config.help.contains_key(&id) {
            self.errors.push(format!("[{context}] unknown section"));
            return;
        }
        self.check_rows(&context, part.rows.as_ref());
        let rows = part.rows.iter().flatten().flat_map(Row::actions);
        self.check_actions(&context, rows);
        let section = self.config.help.entry(id).or_default();
        if let Some(title) = part.title {
            section.title = title;
        }
        if let Some(rows) = part.rows {
            section.rows = rows;
        }
    }

    /// Apply `[t7s]`, then `[<app>]`. Other apps' tables stay unused.
    fn apply_file(&mut self, mut file: File) {
        for name in ["t7s", self.spec.name] {
            if let Some(layer) = file.remove(name) {
                self.apply(name, layer);
            }
        }
    }
}

/// Merge the global built-in file, the app's built-in file, then `[t7s]` and
/// `[<app>]` from `file`. `source` labels errors. A missing file is `None`.
///
/// # Errors
///
/// Every problem in the user's file: bad TOML, unknown settings, names or
/// values, and key conflicts.
///
/// # Panics
///
/// If a built-in config is invalid.
pub fn resolve(
    file: Option<&str>,
    spec: &AppSpec,
    source: &str,
) -> Result<Config, Vec<String>> {
    let global: File =
        toml::from_str(GLOBAL_CONFIG).expect("global config is valid");
    let app: File = toml::from_str(spec.config).expect("app config is valid");
    let shared = action_names(&global, "t7s");
    let own = action_names(&app, spec.name);
    let keymap = Keymap::new(
        shared
            .iter()
            .chain(&own)
            .map(|action| (action.clone(), Vec::new())),
    );
    let mut merge = Merge {
        spec,
        config: Config {
            skin: None,
            theme: None,
            colours: BTreeMap::new(),
            user_text: false,
            keymap,
            screens: BTreeMap::new(),
            help: BTreeMap::new(),
        },
        errors: Vec::new(),
        shared,
        own,
        colour_names: colour_names(&global),
        builtin: true,
    };
    merge.apply_file(global);
    merge.apply_file(app);
    assert!(
        merge.errors.is_empty(),
        "built-in config is invalid: {:?}",
        merge.errors
    );
    // Taken before the user's file, so editing a bar can't hide a conflict.
    let screens = merge
        .config
        .screens
        .iter()
        .map(|(name, screen)| screen.actions(name))
        .collect();
    merge.config.keymap.set_screens(screens);
    merge.builtin = false;

    if let Some(text) = file {
        match toml::from_str(text) {
            Ok(user) => merge.apply_file(user),
            Err(error) => return Err(vec![format!("{source}: {error}")]),
        }
    }
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

impl Config {
    /// The hotkey bar for `screen`, showing current keys.
    ///
    /// # Panics
    ///
    /// If `screen` is unknown, which is a programmer error.
    #[must_use]
    pub fn bar(&self, screen: &str) -> Vec<Hotkey> {
        let found = self
            .screens
            .get(screen)
            .unwrap_or_else(|| panic!("unknown screen: {screen}"));
        found
            .hotkeys
            .iter()
            .map(|row| match row {
                Row::Action { actions, label, .. } => {
                    let action = actions.first().map_or("", String::as_str);
                    Hotkey::code(self.keymap.get(action), label).action(action)
                }
                Row::Key { key, label } => {
                    Hotkey::labeled(key, KeyCode::Null, label)
                }
            })
            .collect()
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

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

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

    const SPEC: AppSpec = AppSpec {
        name: "app",
        config: r#"
[app.keys]
refresh = "r"
logs = ["l", "L"]

[app.screens.main]
hotkeys = [
  { action = "refresh", label = "Refresh" },
  { key = "Ctrl+x", label = "Chord" },
]
hidden = ["quit", "search", "command"]
"#,
    };

    const OVERRIDING: AppSpec = AppSpec {
        name: "app",
        config: "[app.keys]\nquit = \"x\"\n",
    };

    #[test]
    fn app_builtin_overrides_k9tui() {
        assert_eq!(defaults(&OVERRIDING).keymap.get("quit"), c('x'));
    }

    #[test]
    fn user_t7s_overrides_app_builtin() {
        let config =
            resolve(Some("[t7s.keys]\nquit = \"z\"\n"), &OVERRIDING, "c")
                .unwrap();
        assert_eq!(config.keymap.get("quit"), c('z'));
    }

    #[test]
    fn user_app_overrides_user_t7s() {
        let file = "[t7s.keys]\nquit = \"z\"\n[app.keys]\nquit = \"y\"\n";
        let config = resolve(Some(file), &OVERRIDING, "c").unwrap();
        assert_eq!(config.keymap.get("quit"), c('y'));
    }

    #[test]
    fn list_replaces_list_below() {
        let config = ok("[app.keys]\nlogs = \"z\"\n");
        assert_eq!(config.keymap.keys("logs"), [c('z')]);
        let config = ok("[app.keys]\nlogs = [\"y\", \"z\"]\n");
        assert_eq!(config.keymap.keys("logs"), [c('y'), c('z')]);
    }

    #[test]
    fn actions_come_from_builtin_files() {
        let keymap = defaults(&SPEC).keymap;
        assert_eq!(keymap.keys("logs"), [c('l'), c('L')]);
        assert_eq!(keymap.get("top"), c('g'));
    }

    #[test]
    fn screens_are_read_from_the_app_file() {
        let screen = defaults(&SPEC).screens.remove("main").unwrap();
        assert_eq!(screen.hotkeys.len(), 2);
        assert_eq!(screen.hidden.len(), 3);
    }

    #[test]
    fn user_rows_replace_builtin_rows() {
        let file = "[app.screens.main]\nhotkeys = [\n\
                    { action = \"quit\", label = \"Leave\" },\n]\n";
        let mut config = ok(file);
        assert_eq!(config.screens.remove("main").unwrap().hotkeys.len(), 1);
    }

    #[test]
    fn user_row_with_unknown_action_is_fatal() {
        let file = "[app.screens.main]\nhotkeys = [\n\
                    { action = \"nope\", label = \"X\" },\n]\n";
        assert!(fatal(file).contains("nope"));
    }

    #[test]
    fn user_row_with_no_action_is_fatal() {
        let file = "[app.screens.main]\nhotkeys = [\n\
                    { action = [], label = \"X\" },\n]\n";
        assert!(fatal(file).contains("no action"));
    }

    #[test]
    fn user_unknown_screen_is_fatal() {
        assert!(fatal("[app.screens.nope]\nhidden = []\n").contains("nope"));
    }

    #[test]
    fn conflict_check_ignores_user_bar_edits() {
        let file = "[app.keys]\nrefresh = \"q\"\n\
                    [app.screens.main]\nhotkeys = []\nhidden = []\n";
        assert!(fatal(file).contains("refresh"));
    }

    #[test]
    fn empty_key_list_is_fatal() {
        assert!(fatal("[app.keys]\nlogs = []\n").contains("logs"));
    }

    #[test]
    fn bad_key_in_a_list_is_fatal() {
        assert!(fatal("[app.keys]\nlogs = [\"nonsense\"]\n").contains("logs"));
    }

    #[test]
    fn bar_shows_current_key_and_row_text() {
        let config = ok("[app.keys]
refresh = \"z\"
");
        let bar = config.bar("main");
        assert_eq!(bar.first().map(Hotkey::key_label).as_deref(), Some("z"));
        assert_eq!(
            bar.get(1).map(Hotkey::key_label).as_deref(),
            Some("Ctrl+x")
        );
    }

    #[test]
    fn user_row_edit_shows_on_bar() {
        let file = "[app.screens.main]
hotkeys = [
\
                    { action = \"refresh\", label = \"Again\" },
]
";
        let bar = ok(file).bar("main");
        assert_eq!(bar.len(), 1);
        assert_eq!(bar.first().unwrap().description.display_suffix(), "Again");
    }

    #[test]
    fn no_file_and_empty_file_give_defaults() {
        let none = resolve(None, &SPEC, "cfg.toml").unwrap();
        for config in [none, ok(""), ok("# only a comment\n")] {
            assert_eq!(config.keymap.get("quit"), c('q'));
            assert_eq!(config.keymap.get("search"), c('/'));
            assert_eq!(config.keymap.get("command"), c(':'));
            assert_eq!(config.keymap.get("refresh"), c('r'));
            assert_eq!(config.colours, defaults(&SPEC).colours);
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
    fn other_app_tables_are_not_applied() {
        let config = ok("[other.keys]\nquit = \"z\"\nnope = \"y\"\n");
        assert_eq!(config.keymap.get("quit"), c('q'));
    }

    #[test]
    fn other_app_tables_must_still_be_valid() {
        assert!(fatal("[other]\nnonsense = 1\n").contains("nonsense"));
    }

    #[test]
    fn text_colour_equal_to_builtin_still_themes_text() {
        assert_eq!(Palette::default().text, Color::White);
        assert!(!ok("").palette().themed_text);
        let config = ok("[t7s.colours]\ntext = \"white\"\n");
        assert!(config.palette().themed_text);
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
            error.contains("quit")
                && error.contains("ctrl+s")
                && error.contains("line 2"),
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
    fn every_name_error_is_reported_at_once() {
        let error = fatal("[app.keys]\nnope = \"z\"\nnada = \"y\"\n");
        assert!(error.contains("nope") && error.contains("nada"), "{error}");
    }

    #[test]
    fn bad_colour_is_fatal() {
        for bad in ["#12345", "#+12345", "notacolour"] {
            let error = fatal(&format!("[app.colours]\nborder = \"{bad}\"\n"));
            assert!(error.contains(bad), "{error}");
        }
    }

    #[test]
    fn non_string_colour_is_fatal() {
        assert!(fatal("[t7s.colours]\nborder = 3\n").contains("border"));
    }

    #[test]
    fn unknown_colour_name_is_fatal() {
        assert!(fatal("[t7s.colours]\nnope = \"red\"\n").contains("nope"));
    }

    #[test]
    fn colours_with_wrong_shape_is_fatal() {
        assert!(fatal("[t7s]\ncolours = 3\n").contains("colours"));
    }

    #[test]
    fn unknown_setting_is_fatal() {
        assert!(fatal("[t7s]\ntypo = 1\n").contains("typo"));
    }

    #[test]
    fn non_string_theme_and_skin_are_fatal() {
        assert!(fatal("[t7s]\ntheme = 1\n").contains("theme"));
        assert!(fatal("[t7s]\nskin = 2\n").contains("skin"));
    }

    #[test]
    fn app_action_in_t7s_keys_is_fatal() {
        assert!(fatal("[t7s.keys]\nrefresh = \"z\"\n").contains("refresh"));
    }

    #[test]
    fn unknown_action_in_app_keys_is_fatal() {
        assert!(fatal("[app.keys]\nnope = \"z\"\n").contains("nope"));
    }

    #[test]
    fn builtin_colours_match_todays_palette() {
        let config = defaults(&SPEC);
        for (name, colour) in [
            ("background", Color::Reset),
            ("border", Color::DarkGray),
            ("text", Color::White),
            ("focus", Color::Yellow),
            ("on-focus", Color::Black),
            ("selection-fg", Color::Black),
            ("selection-bg", Color::Cyan),
            ("bg-alt", Color::DarkGray),
            ("draft", Color::Green),
            ("multi-select", Color::Blue),
            ("info", Color::Cyan),
            ("error", Color::Red),
            ("success", Color::Green),
            ("warning", Color::Yellow),
            ("link", Color::Blue),
        ] {
            assert_eq!(config.colours.get(name), Some(&colour), "{name}");
        }
        assert_eq!(config.palette(), Palette::default());
    }

    #[test]
    fn theme_replaces_builtin_colours() {
        let config = ok("[t7s]\ntheme = \"github\"\n");
        assert_eq!(config.colours.len(), 0);
        assert_eq!(config.palette(), theme_source::palette("github").unwrap());
    }

    #[test]
    fn reset_is_a_colour() {
        let config = ok("[t7s.colours]\nborder = \"reset\"\n");
        assert_eq!(config.colours.get("border"), Some(&Color::Reset));
    }

    #[test]
    fn no_config_gives_default_palette() {
        assert_eq!(defaults(&SPEC).palette(), Palette::default());
    }

    #[test]
    fn t7s_colours_reach_the_palette() {
        let palette = ok("[t7s.colours]\nborder = \"red\"\n").palette();
        assert_eq!(palette.border, Color::Red);
        assert_eq!(palette.text, Palette::default().text);
    }

    #[test]
    fn app_theme_palette_beats_t7s_colours() {
        let palette = ok("[t7s.colours]\nborder = \"blue\"\n\
             [app]\ntheme = \"github-light\"\n")
        .palette();
        let github = theme_source::palette("github-light").unwrap();
        assert_eq!(palette, github);
        assert_ne!(palette.border, Color::Blue);
    }

    #[test]
    fn app_colours_adjust_app_theme() {
        let palette = ok("[app]\ntheme = \"github-light\"\n\
             [app.colours]\nborder = \"red\"\n")
        .palette();
        let github = theme_source::palette("github-light").unwrap();
        assert_eq!(palette.border, Color::Red);
        assert_eq!(palette.text, github.text);
    }

    #[test]
    fn unknown_theme_is_fatal() {
        assert!(fatal("[t7s]\ntheme = \"nope\"\n").contains("nope"));
    }

    #[test]
    fn colour_names_match_palette_fields() {
        let global: File = toml::from_str(GLOBAL_CONFIG).unwrap();
        let names = colour_names(&global);
        assert_eq!(names.len(), 15);
        for name in names {
            let mut palette = Palette::default();
            assert!(palette.set(&name, Color::Rgb(1, 2, 3)), "{name}");
            assert_ne!(palette, Palette::default(), "{name}");
        }
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
}
