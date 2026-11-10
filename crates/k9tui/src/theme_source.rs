use opaline::{OpalineColor, builtins::load_by_name};
use ratatui::style::Color;

use crate::theme::Palette;

/// The palette for a built-in theme name, in any case, if it exists.
///
/// Selections use an accent, not the theme's own selection background, because
/// `selection_bg` is also a foreground colour (`theme::selection_col`).
#[must_use]
pub fn palette(name: &str) -> Option<Palette> {
    let theme = match name.to_ascii_lowercase().as_str() {
        "github" => load_by_name("github-light"),
        other => load_by_name(other),
    }?;
    let colour = |token| {
        let OpalineColor { r, g, b } = theme.try_color(token)?;
        Some(Color::Rgb(r, g, b))
    };
    Some(Palette {
        background: colour("bg.base")?,
        border: colour("border.unfocused")?,
        text: colour("text.primary")?,
        focus: colour("accent.primary")?,
        on_focus: colour("bg.base")?,
        selection_fg: colour("bg.base")?,
        selection_bg: colour("accent.secondary")?,
        bg_alt: colour("bg.highlight")?,
        draft: colour("success")?,
        multi_select: colour("bg.elevated")?,
        info: colour("info")?,
        error: colour("error")?,
        success: colour("success")?,
        warning: colour("warning")?,
        link: colour("accent.tertiary")?,
    })
}

#[cfg(test)]
mod tests {
    use opaline::builtins::builtin_names;
    use ratatui::style::Color;

    use super::*;

    #[test]
    fn rose_pine_uses_its_own_colours() {
        let palette = palette("rose-pine").unwrap();
        assert_eq!(palette.background, Color::Rgb(0x19, 0x17, 0x24));
        assert_eq!(palette.text, Color::Rgb(0xe0, 0xde, 0xf4));
        assert_eq!(palette.error, Color::Rgb(0xeb, 0x6f, 0x92));
    }

    #[test]
    fn names_are_case_insensitive() {
        assert_eq!(palette("Rose-Pine"), palette("rose-pine"));
    }

    #[test]
    fn github_means_github_light() {
        assert_eq!(palette("github"), palette("github-light"));
        assert_eq!(palette("GitHub"), palette("github-light"));
    }

    #[test]
    fn unknown_name_is_none() {
        assert_eq!(palette("nope"), None);
    }

    #[test]
    fn every_builtin_theme_defines_every_token() {
        for (name, _) in builtin_names() {
            assert!(palette(name).is_some(), "{name}");
        }
    }
}
