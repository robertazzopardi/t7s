use std::{collections::HashMap, io::ErrorKind, path::Path, sync::OnceLock};

use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

use crate::config_dir::{config_dir, read_config};

/// Named colors that make up a skin. Grouped by role, not by widget — several
/// widgets share a role (e.g. `muted` backs borders, labels, and idle status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub muted: Color,
    pub text: Color,
    pub focus: Color,
    pub on_focus: Color,
    pub selection_fg: Color,
    pub selection_bg: Color,
    pub bg_alt: Color,
    pub draft: Color,
    pub multi_select: Color,
    pub info: Color,
    pub error: Color,
    pub success: Color,
    pub warning: Color,
    pub link: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            muted: Color::DarkGray,
            text: Color::White,
            focus: Color::Yellow,
            on_focus: Color::Black,
            selection_fg: Color::Black,
            selection_bg: Color::Cyan,
            bg_alt: Color::DarkGray,
            draft: Color::Green,
            multi_select: Color::Blue,
            info: Color::Cyan,
            error: Color::Red,
            success: Color::Green,
            warning: Color::Yellow,
            link: Color::Blue,
        }
    }
}

static PALETTE: OnceLock<Palette> = OnceLock::new();

/// Install the active palette. Only the first call takes effect; safe to
/// call once at startup before any rendering happens.
pub fn set_palette(palette: Palette) {
    let _ = PALETTE.set(palette);
}

#[cfg(test)]
thread_local! {
    /// Per-thread palette override so tests can render with a skin without
    /// touching the process-wide `OnceLock`.
    static TEST_PALETTE: std::cell::Cell<Option<Palette>> =
        const { std::cell::Cell::new(None) };
}

fn palette() -> Palette {
    #[cfg(test)]
    if let Some(p) = TEST_PALETTE.with(std::cell::Cell::get) {
        return p;
    }
    *PALETTE.get_or_init(Palette::default)
}

/// Parse a skin color name: a named ANSI color (case-insensitive, e.g.
/// `"darkgray"`, `"lightred"`) or a `#rrggbb` hex triplet.
#[must_use]
pub fn parse_color(s: &str) -> Option<Color> {
    if let Some(hex) = s.strip_prefix('#') {
        // `from_str_radix` accepts a leading '+', so check digits explicitly.
        if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            let r = u8::from_str_radix(hex.get(0..2)?, 16).ok()?;
            let g = u8::from_str_radix(hex.get(2..4)?, 16).ok()?;
            let b = u8::from_str_radix(hex.get(4..6)?, 16).ok()?;
            return Some(Color::Rgb(r, g, b));
        }
        return None;
    }
    Some(match s.to_ascii_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        "white" => Color::White,
        _ => return None,
    })
}

#[must_use]
pub fn border() -> Style {
    Style::default().fg(palette().muted)
}

#[must_use]
pub fn title() -> Style {
    Style::default()
        .fg(palette().text)
        .add_modifier(Modifier::BOLD)
}

#[must_use]
pub fn muted() -> Style {
    Style::default().fg(palette().muted)
}

// No hue here by design — color is reserved for state (env tags, errors,
// the cursor row), not decoration. Weight carries emphasis instead.
#[must_use]
pub fn accent() -> Style {
    Style::default()
        .fg(palette().text)
        .add_modifier(Modifier::BOLD)
}

#[must_use]
pub fn info_label() -> Style {
    Style::default().fg(palette().muted)
}

#[must_use]
pub fn info_value() -> Style {
    Style::default().fg(palette().text)
}

#[must_use]
pub fn hotkey_key() -> Style {
    Style::default().fg(palette().text)
}

#[must_use]
pub fn focus_field() -> Style {
    Style::default().fg(palette().focus).bg(palette().bg_alt)
}

#[must_use]
pub fn focus_cursor() -> Style {
    Style::default().bg(palette().focus).fg(palette().on_focus)
}

#[must_use]
pub fn selection_row() -> Style {
    Style::default()
        .fg(palette().selection_fg)
        .bg(palette().selection_bg)
        .add_modifier(Modifier::BOLD)
}

#[must_use]
pub fn selection_col() -> Style {
    // No UNDERLINED here — ratatui applies column_highlight_style to every
    // row in the column, so underline would run down the whole column.
    Style::default().fg(palette().selection_bg)
}

#[must_use]
pub fn selection_cell() -> Style {
    Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(palette().text)
        .bg(palette().bg_alt)
}

#[must_use]
pub fn draft_row() -> Style {
    Style::default().fg(palette().draft)
}

#[must_use]
pub fn multi_select_bg() -> Color {
    palette().multi_select
}

#[must_use]
pub fn status_idle() -> Style {
    Style::default().fg(palette().muted)
}

#[must_use]
pub fn status_message() -> Style {
    Style::default().fg(palette().info)
}

#[must_use]
pub fn error() -> Style {
    Style::default().fg(palette().error)
}

#[must_use]
pub fn success() -> Style {
    Style::default().fg(palette().success)
}

#[must_use]
pub fn modal_default_border() -> Style {
    Style::default().fg(palette().info)
}

#[must_use]
pub fn modal_danger_border() -> Style {
    Style::default().fg(palette().error)
}

#[must_use]
pub fn modal_confirm_border() -> Style {
    Style::default().fg(palette().warning)
}

#[must_use]
pub fn modal_connection_border() -> Style {
    Style::default().fg(palette().link)
}

#[must_use]
pub fn header_row() -> Style {
    // Plain, unbold — no background bar, no weight.
    Style::default().fg(palette().text)
}

#[must_use]
pub fn null_cell() -> Style {
    Style::default().fg(palette().muted)
}

/// `skin.yml`: `colors:` maps palette field names to color specs. Unknown
/// top-level keys are collected only so they can be reported.
#[derive(Debug, Deserialize, Default)]
struct SkinFile {
    #[serde(default)]
    colors: HashMap<String, serde_yaml::Value>,
    #[serde(flatten)]
    other: HashMap<String, serde_yaml::Value>,
}

/// Load `~/.config/<app>/skin.yml` and install the resulting palette.
///
/// A
/// missing file/dir is normal (built-in theme, no warnings); an unreadable or
/// invalid file falls back to the built-in theme and reports why. Individual
/// bad colors only skip that color.
///
/// Must be called before the first `theme::*()` call to take effect (the
/// palette installs once, at first use). Returns human-readable warnings for
/// the caller to surface.
#[must_use]
pub fn load_skin(app: &str) -> Vec<String> {
    let (palette, warnings) = config_dir(app).map_or_else(
        || (Palette::default(), Vec::new()),
        |dir| load_skin_file(&dir.join("skin.yml")),
    );
    set_palette(palette);
    warnings
}

fn load_skin_file(path: &Path) -> (Palette, Vec<String>) {
    match read_config(path) {
        Ok(contents) => parse_skin(&contents, &path.display().to_string()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            (Palette::default(), Vec::new())
        }
        Err(e) => (
            Palette::default(),
            vec![format!(
                "{}: cannot read ({e}); using default theme",
                path.display()
            )],
        ),
    }
}

/// Parse a `skin.yml` document into a palette. `source` only labels warnings.
/// Never fails: on invalid YAML the default palette is returned.
#[must_use]
pub fn parse_skin(yaml: &str, source: &str) -> (Palette, Vec<String>) {
    let mut palette = Palette::default();
    let skin = match serde_yaml::from_str::<Option<SkinFile>>(yaml) {
        Ok(skin) => skin.unwrap_or_default(),
        Err(e) => {
            return (
                palette,
                vec![format!(
                    "{source}: invalid YAML ({e}); using default theme"
                )],
            );
        }
    };
    let mut warnings: Vec<String> = skin
        .other
        .keys()
        .map(|k| format!("{source}: unknown top-level key '{k}' ignored"))
        .collect();
    let mut colors: Vec<_> = skin.colors.into_iter().collect();
    colors.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, value) in colors {
        let Some(slot) = palette.slot_mut(&name) else {
            warnings.push(format!("{source}: unknown color '{name}' ignored"));
            continue;
        };
        match value.as_str().and_then(parse_color) {
            Some(c) => *slot = c,
            None => warnings.push(format!(
                "{source}: color '{name}': invalid value {value:?}; keeping default"
            )),
        }
    }
    warnings.sort();
    (palette, warnings)
}

impl Palette {
    fn slot_mut(&mut self, name: &str) -> Option<&mut Color> {
        Some(match name {
            "muted" => &mut self.muted,
            "text" => &mut self.text,
            "focus" => &mut self.focus,
            "on_focus" => &mut self.on_focus,
            "selection_fg" => &mut self.selection_fg,
            "selection_bg" => &mut self.selection_bg,
            "bg_alt" => &mut self.bg_alt,
            "draft" => &mut self.draft,
            "multi_select" => &mut self.multi_select,
            "info" => &mut self.info,
            "error" => &mut self.error,
            "success" => &mut self.success,
            "warning" => &mut self.warning,
            "link" => &mut self.link,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::widgets::{hotkey::Hotkey, hotkey_view::HotkeyView};

    #[test]
    fn parses_named_and_hex_colors() {
        assert_eq!(parse_color("Red"), Some(Color::Red));
        assert_eq!(parse_color("darkgray"), Some(Color::DarkGray));
        assert_eq!(parse_color("#ff00aa"), Some(Color::Rgb(0xff, 0x00, 0xaa)));
        assert_eq!(parse_color("not-a-color"), None);
        assert_eq!(parse_color("#zzzzzz"), None);
        assert_eq!(parse_color("#fff"), None);
    }

    #[test]
    fn no_skin_is_the_default_palette() {
        for yaml in ["", "# comment\n", "colors: {}\n"] {
            let (p, w) = parse_skin(yaml, "t");
            assert_eq!(w, Vec::<String>::new());
            assert_eq!(p, Palette::default());
        }
    }

    #[test]
    fn partial_skin_overrides_only_named_colors() {
        let (p, w) = parse_skin(
            "colors:\n  focus: \"#ffcc00\"\n  error: lightred\n",
            "t",
        );
        assert_eq!(w, Vec::<String>::new());
        assert_eq!(p.focus, Color::Rgb(0xff, 0xcc, 0x00));
        assert_eq!(p.error, Color::LightRed);
        assert_eq!(p.text, Palette::default().text);
    }

    #[test]
    fn bad_values_unknown_names_and_invalid_yaml_degrade() {
        let (p, w) = parse_skin(
            "colors:\n  focus: nope\n  bogus: red\n  text: 5\nextra: 1\n",
            "t",
        );
        assert_eq!(w.len(), 4);
        assert_eq!(p.focus, Palette::default().focus);
        let (p, w) = parse_skin("colors: [unclosed", "t");
        assert_eq!(w.len(), 1);
        assert_eq!(p.text, Palette::default().text);
        let (_, w) = parse_skin("- a\n- b\n", "t");
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn missing_skin_file_is_silent() {
        let path = std::env::temp_dir()
            .join("k9tui-no-such-dir")
            .join("skin.yml");
        let (_, w) = load_skin_file(&path);
        assert_eq!(w, Vec::<String>::new());
    }

    /// Render the hotkey bar and return the fg of the first key-label cell.
    fn rendered_key_fg() -> Color {
        let keys = [Hotkey::new('n', "new")];
        let mut term = Terminal::new(TestBackend::new(20, 3)).unwrap();
        term.draw(|f| f.render_widget(HotkeyView::new(&keys), f.area()))
            .unwrap();
        let buf = term.backend().buffer();
        let cell = buf
            .content()
            .iter()
            .find(|c| c.symbol() == "n")
            .expect("key label rendered");
        cell.fg
    }

    /// Installs a thread-local skin; restores the default even if the test
    /// panics, so a failure can't leak into later tests on this thread.
    struct SkinGuard;

    impl SkinGuard {
        fn new(yaml: &str) -> Self {
            let (skin, _) = parse_skin(yaml, "t");
            TEST_PALETTE.with(|p| p.set(Some(skin)));
            Self
        }
    }

    impl Drop for SkinGuard {
        fn drop(&mut self) {
            TEST_PALETTE.with(|p| p.set(None));
        }
    }

    fn draw(
        f: impl FnOnce(&mut ratatui::Frame<'_>),
    ) -> ratatui::buffer::Buffer {
        let mut term = Terminal::new(TestBackend::new(40, 12)).unwrap();
        term.draw(f).unwrap();
        term.backend().buffer().clone()
    }

    fn cell<'a>(
        buf: &'a ratatui::buffer::Buffer,
        sym: &str,
    ) -> &'a ratatui::buffer::Cell {
        buf.content()
            .iter()
            .find(|c| c.symbol() == sym)
            .unwrap_or_else(|| panic!("no cell {sym}"))
    }

    #[test]
    fn skin_changes_rendered_cell_styles() {
        assert_eq!(rendered_key_fg(), Palette::default().text);
        {
            let _g = SkinGuard::new("colors:\n  text: \"#102030\"\n");
            assert_eq!(rendered_key_fg(), Color::Rgb(0x10, 0x20, 0x30));
        }
        assert_eq!(rendered_key_fg(), Palette::default().text, "guard resets");
    }

    #[test]
    fn skin_reaches_table_top_bar_and_modal_cells() {
        use crate::widgets::{
            modal::TextPromptModal,
            table::{DataTable, TableData, TableDataState},
            top_bar::TopBarView,
        };

        #[derive(Debug, Clone)]
        struct Item;
        impl TableData for Item {
            fn title() -> &'static str {
                "items"
            }
            fn ref_array(&self) -> Vec<String> {
                vec!["alpha".into()]
            }
            fn num_columns(&self) -> usize {
                1
            }
            fn cols() -> Vec<&'static str> {
                vec!["Name"]
            }
        }

        let _g = SkinGuard::new(
            "colors:\n  text: \"#010203\"\n  muted: \"#040506\"\n  warning: \"#070809\"\n  selection_bg: \"#0a0b0c\"\n  selection_fg: \"#0d0e0f\"\n",
        );

        // Table: header text uses `text`; the
        // selected row uses selection_fg/bg.
        let mut state = TableDataState::new(vec![Item]);
        let buf = draw(|f| {
            f.render_stateful_widget(
                DataTable::<Item>::default(),
                f.area(),
                &mut state,
            );
        });
        assert_eq!(cell(&buf, "N").fg, Color::Rgb(1, 2, 3));
        let row = cell(&buf, "l");
        assert_eq!(row.bg, Color::Rgb(0x0a, 0x0b, 0x0c));
        assert_eq!(row.fg, Color::Rgb(0x0d, 0x0e, 0x0f));

        // Top bar: app label / hotkey bar pick up the skin.
        let keys = [Hotkey::new('n', "new")];
        let buf = draw(|f| {
            f.render_widget(
                TopBarView {
                    summary: "sum",
                    recent_hotkeys: &[],
                    hotkeys: &keys,
                    global_hotkeys: &[],
                    app_name: "app",
                    build_info: None,
                },
                f.area(),
            );
        });
        assert_eq!(cell(&buf, "n").fg, Color::Rgb(1, 2, 3));

        // Modal: border uses `warning`.
        let buf = draw(|f| {
            f.render_widget(TextPromptModal::new("t", 20, 5), f.area());
        });
        assert_eq!(cell(&buf, "\u{250c}").fg, Color::Rgb(7, 8, 9));
    }

    #[test]
    fn color_edge_cases() {
        // uppercase hex and mixed-case names are fine
        assert_eq!(parse_color("#FFAA00"), Some(Color::Rgb(255, 170, 0)));
        assert_eq!(parse_color("DarkGrey"), Some(Color::DarkGray));
        for bad in [
            "",
            "#",
            "#f",
            "#fff",
            "#ffff",
            "#fffffff",
            "#gggggg",
            "# 12345",
            "#+f+f+f",
            "#-1-1-1",
            "ff0000",
            "rgb(1,2,3)",
            "rgb(255, 0, 0)",
            "42",
            "0",
            "white ",
            " white",
            "#\u{e9}\u{e9}\u{e9}",
            "#\u{e9}fffff",
            "#ffff\u{e9}",
            "#\u{1F600}\u{1F600}",
            "\u{0}",
            "Ünicode",
        ] {
            assert_eq!(parse_color(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn malformed_skins_degrade_with_warning_and_never_panic() {
        let deep = format!("{}1{}", "[".repeat(5000), "]".repeat(5000));
        let huge_key = format!("colors:\n  {}: red\n", "k".repeat(100_000));
        let cases: Vec<String> = vec![
            "\u{feff}colors:\n  text: red\n".into(),
            "colors:\n\ttext: red\n".into(), // tab indent
            "colors:\n  text: red\n  text: blue\n".into(), // duplicate key
            "colors:\n  1: red\n".into(),    // non-string key
            "colors:\n  [a]: red\n".into(),  // complex key
            "colors:\n  text: ~\n".into(),   // null
            "colors:\n  text:\n".into(),
            "colors: ~\n".into(),
            "colors: red\n".into(),
            "colors:\n  \"\": red\n".into(), // empty key
            "colors:\n  text: \"\"\n".into(), // empty value
            "colors:\n  text: [red]\n".into(),
            "colors:\n  text: {a: b}\n".into(),
            "colors:\n  text: *nope\n".into(),
            "colors: &a\n  text: *a\n".into(), // recursive alias
            "\u{0}\u{1}\u{2}".into(),
            "{{{{".into(),
            "!!binary |\n  AAAA".into(),
            "---\n---\n".into(), // multiple documents
            deep,
            huge_key,
            "colors:\n  text: \"\\xff\"\n".into(),
            "colors:\n  text: 1e400\n".into(),
        ];
        for yaml in &cases {
            let (p, w) = parse_skin(yaml, "t");
            let short: String = yaml.chars().take(40).collect();
            // Anything that is not the valid-BOM/duplicate/valid cases must
            // leave `text` at default; none may panic.
            if short.starts_with("colors:\n  text: red")
                || short.starts_with('\u{feff}')
            {
                continue;
            }
            assert_eq!(p.text, Palette::default().text, "{short:?}");
            assert!(!w.is_empty() || p == Palette::default(), "{short:?}");
        }
    }

    #[test]
    fn duplicate_color_keys_never_panic() {
        let (_, w) = parse_skin("colors:\n  text: red\n  text: blue\n", "t");
        // serde_yaml either rejects (1 warning) or last-wins (0); both are ok.
        assert!(w.len() <= 1, "{w:?}");
    }

    #[test]
    fn tests_never_touch_the_real_config_dir() {
        // Every load in this crate's tests goes through an explicit path or
        // an in-memory string; prove those paths live outside the user's
        // real config dir.
        let tmp = std::env::temp_dir();
        if let Some(real) = crate::config_dir::config_dir("k9tui-test") {
            assert!(!tmp.starts_with(&real), "{tmp:?} under {real:?}");
        }
        // Parsing is pure: a skin can't be loaded without an explicit source.
        let (p, _) = parse_skin("colors:\n  text: red\n", "t");
        assert_eq!(p.text, Color::Red);
        assert_eq!(Palette::default().text, Color::White);
    }
}
