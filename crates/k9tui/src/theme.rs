use core::cell::Cell;
use std::sync::OnceLock;

use ratatui::{
    Frame,
    style::{Color, Modifier, Style},
};

use crate::config::builtin_palette;

/// Colours for every UI element, one field per config colour name. Grouped by
/// role, not by widget: several widgets share a role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub background: Color,
    pub border: Color,
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
    /// The built-in colours from `config.toml`.
    fn default() -> Self {
        builtin_palette()
    }
}

impl Palette {
    /// Every colour unset; `config.toml` supplies the real defaults.
    pub(crate) const BLANK: Self = Self {
        background: Color::Reset,
        border: Color::Reset,
        text: Color::Reset,
        focus: Color::Reset,
        on_focus: Color::Reset,
        selection_fg: Color::Reset,
        selection_bg: Color::Reset,
        bg_alt: Color::Reset,
        draft: Color::Reset,
        multi_select: Color::Reset,
        info: Color::Reset,
        error: Color::Reset,
        success: Color::Reset,
        warning: Color::Reset,
        link: Color::Reset,
    };

    /// Set a colour by its config name. Returns false if the name is unknown.
    pub const fn set(&mut self, name: &str, colour: Color) -> bool {
        let field = match name.as_bytes() {
            b"background" => &mut self.background,
            b"border" => &mut self.border,
            b"text" => &mut self.text,
            b"focus" => &mut self.focus,
            b"on-focus" => &mut self.on_focus,
            b"selection-fg" => &mut self.selection_fg,
            b"selection-bg" => &mut self.selection_bg,
            b"bg-alt" => &mut self.bg_alt,
            b"draft" => &mut self.draft,
            b"multi-select" => &mut self.multi_select,
            b"info" => &mut self.info,
            b"error" => &mut self.error,
            b"success" => &mut self.success,
            b"warning" => &mut self.warning,
            b"link" => &mut self.link,
            _ => return false,
        };
        *field = colour;
        true
    }
}

static PALETTE: OnceLock<Palette> = OnceLock::new();

thread_local! {
    static THREAD_PALETTE: Cell<Option<Palette>> = const { Cell::new(None) };
}

/// Set the app's palette. Call once at startup, before the first draw; later
/// calls are ignored.
pub fn install(palette: Palette) {
    let _ = PALETTE.set(palette);
}

/// Use `palette` on this thread only, so render tests don't share state.
#[doc(hidden)]
pub fn override_for_thread(palette: Option<Palette>) {
    THREAD_PALETTE.set(palette);
}

fn palette() -> Palette {
    THREAD_PALETTE
        .get()
        .unwrap_or_else(|| *PALETTE.get_or_init(Palette::default))
}

/// Text a widget leaves unstyled: the theme's colour when a theme or text colour
/// is set, otherwise the terminal's own, so the default look doesn't change.
fn base_text(style: Style, palette: Palette) -> Style {
    if palette.background != Color::Reset
        || palette.text != Palette::default().text
    {
        style.fg(palette.text)
    } else {
        style
    }
}

#[must_use]
pub fn background() -> Style {
    let palette = palette();
    base_text(Style::default().bg(palette.background), palette)
}

/// Fill the frame with the theme's background. Call first when drawing.
pub fn paint_background(frame: &mut Frame) {
    let area = frame.area();
    frame.buffer_mut().set_style(area, background());
}

/// Modals stand out in black on the terminal's own background; under a theme
/// they use its background instead.
#[must_use]
pub fn modal_background() -> Style {
    let palette = palette();
    let background = if palette.background == Color::Reset {
        Color::Black
    } else {
        palette.background
    };
    base_text(Style::default().bg(background), palette)
}

#[must_use]
pub fn modal_text() -> Style {
    info_value().patch(modal_background())
}

/// [`selection_row`] without the weight, for buttons and dropdown rows.
#[must_use]
pub fn selection_plain() -> Style {
    Style::default()
        .fg(palette().selection_fg)
        .bg(palette().selection_bg)
}

#[must_use]
pub fn button_idle() -> Style {
    Style::default().fg(palette().text).bg(palette().bg_alt)
}

#[must_use]
pub fn warning() -> Style {
    Style::default().fg(palette().warning)
}

#[must_use]
pub fn border() -> Style {
    Style::default().fg(palette().border)
}

#[must_use]
pub fn title() -> Style {
    Style::default()
        .fg(palette().text)
        .add_modifier(Modifier::BOLD)
}

#[must_use]
pub fn muted() -> Style {
    border()
}

// No hue here by design — color is reserved for state (env tags, errors,
// the cursor row), not decoration. Weight carries emphasis instead.
#[must_use]
pub fn accent() -> Style {
    title()
}

#[must_use]
pub fn info_label() -> Style {
    border()
}

#[must_use]
pub fn info_value() -> Style {
    Style::default().fg(palette().text)
}

#[must_use]
pub fn hotkey_key() -> Style {
    info_value()
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
    border()
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
    error()
}

#[must_use]
pub fn modal_confirm_border() -> Style {
    warning()
}

#[must_use]
pub fn modal_connection_border() -> Style {
    Style::default().fg(palette().link)
}

#[must_use]
pub fn header_row() -> Style {
    // Plain, unbold — no background bar, no weight.
    info_value()
}

#[must_use]
pub fn null_cell() -> Style {
    border()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_matches_todays_styles() {
        assert_eq!(border(), Style::default().fg(Color::DarkGray));
        assert_eq!(error(), Style::default().fg(Color::Red));
        assert_eq!(
            focus_field(),
            Style::default().fg(Color::Yellow).bg(Color::DarkGray)
        );
        assert_eq!(multi_select_bg(), Color::Blue);
        assert_eq!(background(), Style::default().bg(Color::Reset));
        assert_eq!(modal_background(), Style::default().bg(Color::Black));
        assert_eq!(
            selection_plain(),
            Style::default().fg(Color::Black).bg(Color::Cyan)
        );
        assert_eq!(
            button_idle(),
            Style::default().fg(Color::White).bg(Color::DarkGray)
        );
    }

    #[test]
    fn text_override_reaches_default_styled_text() {
        let palette = Palette {
            text: Color::Red,
            ..Palette::default()
        };
        override_for_thread(Some(palette));
        assert_eq!(background().fg, Some(Color::Red));
        assert_eq!(modal_background().fg, Some(Color::Red));
        override_for_thread(None);
    }

    #[test]
    fn default_palette_leaves_terminal_foreground() {
        assert_eq!(background().fg, None);
        assert_eq!(modal_background().fg, None);
    }

    #[test]
    fn thread_override_changes_styles() {
        let palette = Palette {
            border: Color::Red,
            ..Palette::default()
        };
        override_for_thread(Some(palette));
        assert_eq!(border(), Style::default().fg(Color::Red));
        override_for_thread(None);
        assert_eq!(border(), Style::default().fg(Color::DarkGray));
    }
}
