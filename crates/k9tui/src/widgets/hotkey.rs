use alloc::borrow::Cow;
use core::fmt::{self, Display};

use crossterm::event::KeyCode;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone)]
pub enum HotkeyDescription {
    Static(String),
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
            Self::Static(s) => Cow::Borrowed(s),
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
    pub key_display: Option<String>,
    /// Keymap action this hotkey triggers, so the bar can show remapped keys.
    pub action: Option<String>,
}

impl Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.key_label())
    }
}

impl Hotkey {
    #[must_use]
    pub fn new(c: char, description: &str) -> Self {
        Self {
            keycode: KeyCode::Char(c),
            description: HotkeyDescription::Static(description.to_owned()),
            key_display: None,
            action: None,
        }
    }

    /// Tag with the keymap action this hotkey triggers.
    #[must_use]
    pub fn action(mut self, action: &str) -> Self {
        self.action = Some(action.to_owned());
        self
    }

    #[must_use]
    pub fn code(keycode: KeyCode, description: &str) -> Self {
        Self {
            keycode,
            description: HotkeyDescription::Static(description.to_owned()),
            key_display: None,
            action: None,
        }
    }

    /// Custom key label for chords the `KeyCode` alone cannot show (e.g. `Ctrl+e`).
    #[must_use]
    pub fn labeled(display: &str, keycode: KeyCode, description: &str) -> Self {
        Self {
            keycode,
            description: HotkeyDescription::Static(description.to_owned()),
            key_display: Some(display.to_owned()),
            action: None,
        }
    }

    #[must_use]
    #[allow(clippy::wildcard_enum_match_arm)] // KeyCode is crossterm's; only label keys we show
    pub fn key_label(&self) -> String {
        if let Some(label) = &self.key_display {
            return label.clone();
        }
        match self.keycode {
            KeyCode::Char(' ') => "Space".to_string(),
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
            KeyCode::F(n) => format!("F{n}"),
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
