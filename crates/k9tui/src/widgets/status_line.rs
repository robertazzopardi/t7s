use ratatui::{
    buffer::Buffer,
    prelude::*,
    widgets::{Paragraph, Widget, Wrap},
};

use crate::theme;

/// Footer status line (ratatui `Paragraph`).
#[derive(Clone, Debug, Default)]
pub struct StatusLine {
    message: String,
    idle_hint: String,
}

impl StatusLine {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            message: String::new(),
            idle_hint: String::new(),
        }
    }

    pub fn set_message<S: Into<String>>(&mut self, message: S) {
        self.message = message.into();
    }

    pub fn set_idle_hint<S: Into<String>>(&mut self, hint: S) {
        self.idle_hint = hint.into();
    }

    pub fn clear(&mut self) {
        self.message.clear();
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Widget for StatusLine {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let (text, style) = if self.message.is_empty() {
            (self.idle_hint.as_str(), theme::status_idle())
        } else {
            (self.message.as_str(), theme::status_message())
        };

        Paragraph::new(text)
            .style(style)
            .wrap(Wrap { trim: true })
            .render(area, buf);
    }
}
