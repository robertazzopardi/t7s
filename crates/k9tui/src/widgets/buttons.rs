use ratatui::{
    prelude::{Buffer, Rect, Widget},
    text::{Line, Span},
};

use crate::theme;

pub struct Buttons<'a> {
    pub buttons: Vec<&'a str>,
    pub selected: usize,
}

impl Widget for Buttons<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut button_spans = vec![];
        for (i, button) in self.buttons.iter().enumerate() {
            if i == self.selected {
                button_spans.push(Span::styled(
                    format!(" {button} "),
                    theme::accent().bg(theme::multi_select_bg()),
                ));
            } else {
                button_spans.push(Span::styled(
                    format!(" {button} "),
                    theme::button_idle(),
                ));
            }
            button_spans.push(Span::raw(" "));
        }
        Line::from(button_spans).centered().render(area, buf);
    }
}
