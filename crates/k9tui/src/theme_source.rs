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
    let (base, text) = (colour("bg.base")?, colour("text.primary")?);
    // Light themes' accents can be too pale for the background colour on top.
    let on = |accent| {
        if contrast(base, accent) >= contrast(text, accent) {
            base
        } else {
            text
        }
    };
    let (focus, selection_bg) =
        (colour("accent.primary")?, colour("accent.secondary")?);
    Some(Palette {
        background: base,
        border: colour("border.unfocused")?,
        text,
        focus,
        on_focus: on(focus),
        selection_fg: on(selection_bg),
        selection_bg,
        bg_alt: colour("bg.highlight")?,
        draft: colour("success")?,
        multi_select: colour("bg.active")?,
        info: colour("info")?,
        error: colour("error")?,
        success: colour("success")?,
        warning: colour("warning")?,
        link: colour("accent.tertiary")?,
        themed_text: true,
    })
}

/// WCAG contrast ratio, 1 to 21.
fn contrast(one: Color, other: Color) -> f64 {
    let luminance = |colour| {
        let Color::Rgb(red, green, blue) = colour else {
            panic!("not rgb: {colour:?}")
        };
        let channel = |value: u8| {
            let scaled = f64::from(value) / 255.0;
            if scaled <= 0.039_28 {
                scaled / 12.92
            } else {
                ((scaled + 0.055) / 1.055).powf(2.4)
            }
        };
        0.0722f64.mul_add(
            channel(blue),
            0.2126f64.mul_add(channel(red), 0.7152 * channel(green)),
        )
    };
    let (one, other) = (luminance(one), luminance(other));
    (one.max(other) + 0.05) / (one.min(other) + 0.05)
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
    fn every_builtin_theme_keeps_selections_readable() {
        for (name, _) in builtin_names() {
            let p = palette(name).unwrap();
            // Some themes' surfaces sit close together; 1.1 still shows.
            let marked = contrast(p.multi_select, p.background);
            assert!(marked >= 1.1, "{name}: multi-select {marked:.2}");
            for (pair, fg, bg) in [
                ("multi-select text", p.text, p.multi_select),
                ("selection", p.selection_fg, p.selection_bg),
                ("on-focus", p.on_focus, p.focus),
            ] {
                let ratio = contrast(fg, bg);
                // Under 3 only where a light theme's accent is mid-tone and
                // neither its background nor its text does better.
                assert!(ratio >= 2.5, "{name}: {pair} {ratio:.2}");
            }
        }
    }

    #[test]
    fn every_builtin_theme_defines_every_token() {
        for (name, _) in builtin_names() {
            assert!(palette(name).is_some(), "{name}");
        }
    }
}
