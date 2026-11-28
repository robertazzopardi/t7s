//! The `c` form: Name, Agent (cycled with left/right) and Dir. Built from
//! k9tui's `ModalField`; k9tui has no generic form widget.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use k9tui::{
    theme,
    widgets::modal::{DialogAction, ModalField},
};
use ratatui::{
    prelude::{Alignment, Buffer, Constraint, Layout, Rect, Widget},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::agent::{AgentProfile, PROFILES, profile};

const WIDTH: u16 = 64;
const HEIGHT: u16 = 8;
const NAME: usize = 0;
const AGENT: usize = 1;
const DIR: usize = 2;
const LABEL_WIDTH: u16 = 8;

#[derive(Debug, Clone)]
pub struct NewSessionModal {
    fields: [ModalField; 3],
    focus: usize,
}

impl NewSessionModal {
    #[must_use]
    pub fn new(dir: &str) -> Self {
        let mut name = ModalField::new("Name");
        let mut agent = ModalField::new("Agent");
        // Options make `input_key` a no-op on Agent, so typing cannot edit it.
        agent.set_options(PROFILES.iter().map(|p| p.name).collect());
        let mut d = ModalField::new("Dir");
        d.set_value(dir);
        name.set_focus(true);
        Self {
            fields: [name, agent, d],
            focus: NAME,
        }
    }

    fn move_focus(&mut self, forward: bool) {
        let len = self.fields.len();
        self.focus = if forward {
            (self.focus + 1) % len
        } else {
            (self.focus + len - 1) % len
        };
        for (i, f) in self.fields.iter_mut().enumerate() {
            f.set_focus(i == self.focus);
        }
    }

    fn cycle_agent(&mut self, forward: bool) {
        let n = PROFILES.len();
        let Some(f) = self.fields.get_mut(AGENT) else {
            return;
        };
        let cur = PROFILES.iter().position(|p| p.name == f.value());
        let next = match (cur, forward) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, _) => 0,
        };
        if let Some(p) = PROFILES.get(next) {
            f.set_value(p.name);
        }
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn handle_key_events(&mut self, key: KeyEvent) -> DialogAction {
        match key.code {
            KeyCode::Esc => return DialogAction::Cancel,
            KeyCode::Enter => return DialogAction::Submit,
            KeyCode::Tab | KeyCode::Down => self.move_focus(true),
            KeyCode::BackTab | KeyCode::Up => self.move_focus(false),
            KeyCode::Left | KeyCode::Right if self.focus == AGENT => {
                self.cycle_agent(key.code == KeyCode::Right);
            }
            _ => {
                if let Some(f) = self.fields.get_mut(self.focus) {
                    f.input_key(key);
                }
            }
        }
        DialogAction::None
    }

    /// Current form values as entered.
    #[must_use]
    pub fn submission(
        &self,
    ) -> (String, Option<&'static AgentProfile>, PathBuf) {
        let v = |i: usize| self.fields.get(i).map_or("", ModalField::value);
        (
            v(NAME).trim().to_string(),
            profile(v(AGENT)),
            PathBuf::from(v(DIR)),
        )
    }
}

impl Widget for &NewSessionModal {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let w = WIDTH.min(area.width);
        let h = HEIGHT.min(area.height);
        let modal = Rect::new(
            area.x + (area.width - w) / 2,
            area.y + (area.height - h) / 2,
            w,
            h,
        );
        Clear.render(modal, buf);
        let block = Block::new()
            .title(" New session ")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(theme::modal_default_border())
            .style(theme::modal_background());
        let inner = block.inner(modal);
        block.render(modal, buf);

        let rows = Layout::vertical([Constraint::Length(1); 5]).split(inner);
        for (i, f) in self.fields.iter().enumerate() {
            let Some(&row) = rows.get(i) else { continue };
            let style = if f.is_focused {
                theme::focus_field()
            } else {
                theme::info_value()
            };
            let [label, value] = Layout::horizontal([
                Constraint::Length(LABEL_WIDTH),
                Constraint::Min(1),
            ])
            .areas(row);
            Paragraph::new(format!("{}:", f.label))
                .style(style)
                .render(label, buf);
            if i == AGENT {
                Paragraph::new(format!("◀ {} ▶", f.value()))
                    .style(style)
                    .render(value, buf);
            } else {
                Widget::render(&f.input, value, buf);
            }
        }
        if let Some(&hint) = rows.get(4) {
            Paragraph::new("Tab next  ←/→ agent  Enter create  Esc cancel")
                .style(theme::muted())
                .alignment(Alignment::Center)
                .render(hint, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyModifiers;

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_cycling_and_submit() {
        let mut m = NewSessionModal::new("/tmp");
        assert_eq!(m.submission().1.map(|p| p.name), Some("claude"));
        for c in "ab".chars() {
            m.handle_key_events(key(KeyCode::Char(c)));
        }
        m.handle_key_events(key(KeyCode::Tab));
        m.handle_key_events(key(KeyCode::Right));
        m.handle_key_events(key(KeyCode::Left));
        m.handle_key_events(key(KeyCode::Left));
        let (name, agent, dir) = m.submission();
        assert_eq!(name, "ab");
        assert_eq!(agent.map(|p| p.name), Some("shell"), "left wraps");
        assert_eq!(dir, PathBuf::from("/tmp"));
        assert_eq!(
            m.handle_key_events(key(KeyCode::Enter)),
            DialogAction::Submit
        );
    }

    #[test]
    fn long_dir_stays_inside_border() {
        let dirs = [
            "/Users/rob/".to_string() + &"deeply/nested/".repeat(10),
            "/Users/rob/".to_string() + &"プロジェクト/🚀/".repeat(4),
        ];
        for dir in &dirs {
            for width in 20..=80 {
                let mut m = NewSessionModal::new(dir);
                m.move_focus(false);
                let area = Rect::new(0, 0, width, 12);
                let mut buf = Buffer::empty(area);
                (&m).render(area, &mut buf);
                let (w, h) = (WIDTH.min(width), HEIGHT.min(12));
                let right = (width - w) / 2 + w - 1;
                let top = (12 - h) / 2;
                for y in top + 1..top + h - 1 {
                    assert_eq!(
                        buf[(right, y)].symbol(),
                        "│",
                        "w={width} y={y}"
                    );
                    for x in right + 1..width {
                        assert_eq!(
                            buf[(x, y)].symbol(),
                            " ",
                            "w={width} x={x} y={y}"
                        );
                    }
                }
            }
        }
    }
}
