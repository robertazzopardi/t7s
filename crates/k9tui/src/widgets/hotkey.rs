use alloc::borrow::Cow;
use core::fmt::{self, Display};

use crossterm::event::KeyCode;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone)]
pub enum HotkeyDescription {
    Static(&'static str),
    /// Digit hotkey to reopen a recently viewed table (`schema.table` in the bar).
    RecentTable {
        schema: String,
        table: String,
    },
}

impl HotkeyDescription {
    #[must_use]
    pub fn display_suffix(&self) -> Cow<'_, str> {
        match self {
            Self::Static(s) => Cow::Borrowed(*s),
            Self::RecentTable { schema, table } => {
                Cow::Owned(format!("{schema}.{table}"))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Hotkey {
    pub keycode: KeyCode,
    pub description: HotkeyDescription,
    /// When set, shown instead of the `KeyCode` label (e.g. `Ctrl+e`).
    pub key_display: Option<&'static str>,
}

impl Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.key_label())
    }
}

impl Hotkey {
    #[must_use]
    pub const fn new(c: char, description: &'static str) -> Self {
        Self {
            keycode: KeyCode::Char(c),
            description: HotkeyDescription::Static(description),
            key_display: None,
        }
    }

    #[must_use]
    pub const fn code(keycode: KeyCode, description: &'static str) -> Self {
        Self {
            keycode,
            description: HotkeyDescription::Static(description),
            key_display: None,
        }
    }

    /// Custom key label for chords the `KeyCode` alone cannot show (e.g. `Ctrl+e`).
    #[must_use]
    pub const fn labeled(
        display: &'static str,
        keycode: KeyCode,
        description: &'static str,
    ) -> Self {
        Self {
            keycode,
            description: HotkeyDescription::Static(description),
            key_display: Some(display),
        }
    }

    #[must_use]
    #[allow(clippy::wildcard_enum_match_arm)] // KeyCode is crossterm's; only label keys we show
    pub fn key_label(&self) -> String {
        if let Some(label) = self.key_display {
            return label.to_string();
        }
        match self.keycode {
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::Delete => "Del".to_string(),
            KeyCode::Up => "↑".to_string(),
            KeyCode::Down => "↓".to_string(),
            KeyCode::Left => "←".to_string(),
            KeyCode::Right => "→".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::PageUp => "PgUp".to_string(),
            KeyCode::PageDown => "PgDn".to_string(),
            other => format!("{other:?}"),
        }
    }

    #[must_use]
    pub fn length(&self) -> u16 {
        let key_len = u16::try_from(self.key_label().len()).unwrap_or(1);
        let desc_len = u16::try_from(UnicodeWidthStr::width(
            self.description.display_suffix().as_ref(),
        ))
        .unwrap_or(1);

        key_len + desc_len + 3
    }
}
