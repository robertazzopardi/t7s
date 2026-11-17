use crossterm::event::KeyCode;
use k9tui::{
    theme,
    widgets::{
        hotkey::Hotkey,
        status_line::FOOTER_HEIGHT,
        table::DataTable,
        top_bar::{TOPBAR_HEIGHT, TopBarView},
    },
};
use ratatui::{
    Frame,
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};
use tui_term::widget::{Cursor, PseudoTerminal};

use crate::{
    app::{APP_NAME, App, View},
    pty::Pty,
    session::glyph,
};

/// Resize the PTY and parser to `inner` when it differs. Zero rows or
/// columns never reach `resize`. Returns whether a resize happened.
pub(crate) fn fit_pty(pty: &mut Pty, inner: Rect) -> bool {
    let (rows, cols) = (inner.height, inner.width);
    if rows < 1 || cols < 1 || pty.screen().screen().size() == (rows, cols) {
        return false;
    }
    pty.resize(rows, cols).is_ok()
}

impl App {
    /// Top bar entries for the most recently focused sessions.
    fn recent_hotkeys(&self) -> Vec<Hotkey> {
        self.recent
            .iter()
            .filter_map(|id| {
                let i = self.sessions_by_id.iter().position(|s| s.id == *id)?;
                let s = self.sessions_by_id.get(i)?;
                let glyph = glyph(s.status.status());
                Some(Hotkey::labeled(
                    &format!("Ctrl+{}", i + 1),
                    KeyCode::Char(char::from_digit(
                        u32::try_from(i + 1).ok()?,
                        10,
                    )?),
                    &format!("{glyph} {}", s.name),
                ))
            })
            .take(usize::from(TOPBAR_HEIGHT - 2))
            .collect()
    }

    pub fn render(&mut self, frame: &mut Frame) {
        theme::paint_background(frame);
        let layout = Layout::vertical([
            Constraint::Length(TOPBAR_HEIGHT),
            Constraint::Min(0),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(frame.area());
        let top_area = layout.first().copied().unwrap_or_default();
        let content_area = layout.get(1).copied().unwrap_or_default();
        let footer_area = layout.get(2).copied().unwrap_or_default();

        let n = self.sessions.model.items.len();
        let hotkeys = self.config.bar(if self.view == View::Agent {
            "agent"
        } else {
            "sessions"
        });
        // In the agent view q goes to the agent; its bar lists the chords.
        let global = if self.view == View::Agent {
            Vec::new()
        } else {
            self.config.bar("global")
        };
        let recent = self.recent_hotkeys();
        frame.render_widget(
            TopBarView {
                summary: &format!("Sessions: {n}"),
                recent_hotkeys: &recent,
                hotkeys: &hotkeys,
                global_hotkeys: &global,
                app_name: APP_NAME,
                build_info: Some(self.build_info.clone()),
            },
            top_area,
        );

        let agent = match (self.view, self.focused) {
            (View::Agent, Some(id)) => {
                self.sessions_by_id.iter_mut().find(|s| s.id == id)
            }
            _ => None,
        };
        if let Some(s) = agent {
            let block = Block::bordered()
                .border_style(theme::border())
                .title(format!(" {} · {} ", s.name, s.agent.name))
                .title_alignment(Alignment::Center);
            let inner = block.inner(content_area);
            if let Some(pty) = s.pty.as_mut() {
                fit_pty(pty, inner);
                let parser = pty.screen();
                // The focused session is the only one on screen.
                let cursor = Cursor::default().visibility(true);
                frame.render_widget(
                    PseudoTerminal::new(parser.screen())
                        .block(block)
                        .cursor(cursor),
                    content_area,
                );
            } else {
                frame.render_widget(block, content_area);
                Paragraph::new("session exited")
                    .style(theme::muted())
                    .alignment(Alignment::Center)
                    .render(inner, frame.buffer_mut());
            }
        } else {
            let block = Block::new()
                .borders(Borders::ALL)
                .border_style(theme::border())
                .title(format!(" Sessions [{n}] "))
                .title_alignment(Alignment::Center);
            let inner = block.inner(content_area);
            frame.render_widget(block, content_area);
            if n == 0 {
                Paragraph::new("No sessions — press c to create one")
                    .style(theme::muted())
                    .alignment(Alignment::Center)
                    .render(inner, frame.buffer_mut());
            } else {
                frame.render_stateful_widget(
                    DataTable::default(),
                    inner,
                    &mut self.sessions,
                );
            }
        }

        frame.render_widget(self.status_line.clone(), footer_area);

        if let Some(modal) = &self.new_modal {
            frame.render_widget(modal, frame.area());
        }
        if let Some(dialog) = self.confirm_dialog.clone() {
            frame.render_widget(dialog, frame.area());
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing)]

    use core::fmt::Write as _;
    use std::{
        env::temp_dir,
        time::{Instant, SystemTime},
    };

    use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

    use super::*;
    use crate::{agent::AgentProfile, session::Session, status::StatusTracker};

    #[test]
    fn empty_sessions_view_shows_hint() {
        let mut app = App::new();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let out: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(Cell::symbol)
            .collect();
        assert!(out.contains("No sessions"));
    }

    static SLEEP: AgentProfile = AgentProfile {
        name: "test-sleep",
        command: &["/bin/sh", "-c", "sleep 30"],
        resume: None,
        blocked: &[],
        working: &[],
    };

    fn dump(terminal: &Terminal<TestBackend>) -> String {
        let w = usize::from(terminal.backend().buffer().area.width);
        let mut text = String::new();
        for row in terminal.backend().buffer().content().chunks(w) {
            let line: String = row.iter().map(Cell::symbol).collect();
            writeln!(text, "{}", line.trim_end()).unwrap();
        }
        text
    }

    #[test]
    fn zero_area_does_not_resize() {
        let mut app = App::new();
        app.create_session("one", &SLEEP, &temp_dir());
        let pty = app.sessions_by_id[0].pty.as_mut().unwrap();
        let before = pty.screen().screen().size();
        assert!(!fit_pty(pty, Rect::new(0, 0, 0, 5)));
        assert!(!fit_pty(pty, Rect::new(0, 0, 5, 0)));
        assert_eq!(pty.screen().screen().size(), before);
        assert!(fit_pty(pty, Rect::new(0, 0, 30, 7)));
        assert_eq!(pty.screen().screen().size(), (7, 30));
        assert!(
            !fit_pty(pty, Rect::new(0, 0, 30, 7)),
            "same size is a no-op"
        );
        // A frame too short for any content must not panic or resize.
        let mut terminal = Terminal::new(TestBackend::new(100, 3)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let pty = app.sessions_by_id[0].pty.as_ref().unwrap();
        assert_eq!(pty.screen().screen().size(), (7, 30));
    }

    #[test]
    fn agent_view_renders() {
        let mut app = App::new();
        app.build_info = String::new();
        app.create_session("one", &SLEEP, &temp_dir());
        let mut terminal = Terminal::new(TestBackend::new(60, 16)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        app.sessions_by_id[0]
            .pty
            .as_ref()
            .unwrap()
            .screen()
            .process(b"hello \x1b[1;31mred\x1b[0m\r\nsecond line");
        terminal.draw(|f| app.render(f)).unwrap();
        insta::assert_snapshot!(dump(&terminal));
    }

    #[test]
    fn top_bar_shows_recent_sessions() {
        let mut app = App::new();
        app.build_info = String::new();
        for (i, name) in ["alpha", "beta", "gamma"].into_iter().enumerate() {
            app.sessions_by_id.push(Session {
                id: i as u64 + 1,
                name: name.into(),
                agent: &SLEEP,
                dir: "/w".into(),
                pty: None,
                status: StatusTracker::new(Instant::now()),
                created: SystemTime::UNIX_EPOCH,
                repo: None,
            });
        }
        app.sessions_by_id[2].status.exited(1);
        app.focus_session(1);
        app.focus_session(3);
        app.view = View::Sessions;
        app.refresh_table_at(SystemTime::UNIX_EPOCH);
        let mut terminal = Terminal::new(TestBackend::new(110, 12)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        insta::assert_snapshot!(dump(&terminal));
    }
}
