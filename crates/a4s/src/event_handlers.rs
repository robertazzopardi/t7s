use crossterm::event::KeyEvent;
use k9tui::{
    event::is_ctrl_c,
    keymap::plain,
    theme,
    widgets::{
        modal::{ConfirmDialog, DialogAction},
        navigation::TableNavigationHandler,
    },
};

use crate::{
    app::{App, View},
    new_session::NewSessionModal,
    pty::SessionId,
    sessions::Pending,
};

impl App {
    pub fn on_key_event(&mut self, key: KeyEvent) {
        match self.view {
            View::Sessions => self.on_key_sessions(key),
            // Agent keys are forwarded to the PTY in a later task.
            View::Agent => {}
        }
    }

    fn on_key_sessions(&mut self, key: KeyEvent) {
        if self.confirm_dialog.is_some() {
            self.on_key_confirm(key);
            return;
        }
        if self.new_modal.is_some() {
            self.on_key_new_modal(key);
            return;
        }
        if is_ctrl_c(key) {
            self.quit();
            return;
        }
        let screen = "sessions";
        let selected = self.selected_session().map(|(id, ..)| id);
        match self.config.keymap.action(screen, plain(key)) {
            "quit" => self.quit(),
            "new" => {
                self.new_modal = Some(NewSessionModal::new(
                    &self.launch_dir.display().to_string(),
                ));
            }
            "open" => {
                if let Some(id) = selected {
                    self.focus_session(id);
                }
            }
            "kill" => self.prompt_kill(),
            "forget" => self.prompt_forget(),
            "restart" => {
                if let Some(id) = selected {
                    self.restart_session(id);
                }
            }
            _ => {
                if let Some(nav) = self.config.keymap.nav(screen, plain(key)) {
                    TableNavigationHandler::navigate_table(
                        &self.sessions.model,
                        &mut self.sessions.view,
                        nav,
                    );
                }
            }
        }
    }

    fn selected_session(&self) -> Option<(SessionId, String, bool)> {
        let s = self.sessions_by_id.get(self.selected_index()?)?;
        Some((s.id, s.name.clone(), s.is_exited()))
    }

    fn prompt_kill(&mut self) {
        let Some(i) = self.selected_index() else {
            return;
        };
        let Some(s) = self.sessions_by_id.get(i) else {
            return;
        };
        if s.is_exited() {
            self.status_line
                .set_message(format!("{} has already exited", s.name));
            return;
        }
        self.pending = Some(Pending::Kill(s.id));
        self.confirm_dialog = Some(ConfirmDialog::new(
            " Kill session ",
            format!(
                "Kill {} ({})?\nThe directory is not touched.",
                s.name, s.agent.name
            ),
            theme::modal_danger_border(),
            1, // default to No
        ));
    }

    fn prompt_forget(&mut self) {
        let Some((id, name, exited)) = self.selected_session() else {
            return;
        };
        if !exited {
            self.status_line
                .set_message(format!("Kill {name} before forgetting it"));
            return;
        }
        self.pending = Some(Pending::Forget(id));
        self.confirm_dialog = Some(ConfirmDialog::new(
            " Forget session ",
            format!("Forget session {name}?\nFiles are not touched."),
            theme::modal_danger_border(),
            1,
        ));
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    fn on_key_confirm(&mut self, key: KeyEvent) {
        let Some(dialog) = self.confirm_dialog.as_mut() else {
            return;
        };
        match dialog.handle_key_events(key) {
            DialogAction::None => {}
            action => {
                self.confirm_dialog = None;
                let pending = self.pending.take();
                match (action, pending) {
                    (DialogAction::Submit, Some(Pending::Kill(id))) => {
                        self.kill_session(id);
                    }
                    (DialogAction::Submit, Some(Pending::Forget(id))) => {
                        self.forget_session(id);
                    }
                    _ => {}
                }
            }
        }
    }

    fn on_key_new_modal(&mut self, key: KeyEvent) {
        let Some(modal) = self.new_modal.as_mut() else {
            return;
        };
        match modal.handle_key_events(key) {
            DialogAction::None => {}
            DialogAction::Cancel => self.new_modal = None,
            DialogAction::Submit => {
                let (name, agent, dir) = modal.submission();
                let Some(agent) = agent else {
                    return;
                };
                // The form stays open on failure so the input can be fixed.
                if self.create_session(&name, agent, &dir) {
                    self.new_modal = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing)]

    use core::{fmt::Write as _, time::Duration};
    use std::{
        env::temp_dir,
        path::PathBuf,
        thread::sleep,
        time::{Instant, SystemTime},
    };

    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

    use super::*;
    use crate::{
        agent::AgentProfile,
        defaults::KILLED_CODE,
        session::Session,
        status::{Status, StatusTracker},
    };

    // Test profiles avoid mutating SHELL (tests run in parallel).
    static SH: AgentProfile = AgentProfile {
        name: "test-sh",
        command: &["/bin/sh"],
        resume: None,
        blocked: &["Proceed?"],
        working: &[],
    };
    static EXIT3: AgentProfile = AgentProfile {
        name: "test-exit3",
        command: &["/bin/sh", "-c", "exit 3"],
        resume: None,
        blocked: &[],
        working: &[],
    };
    static NO_SUCH: AgentProfile = AgentProfile {
        name: "test-missing",
        command: &["a4s-no-such-cmd"],
        resume: None,
        blocked: &[],
        working: &[],
    };

    fn press(app: &mut App, code: KeyCode) {
        app.on_key_event(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn tmp() -> PathBuf {
        temp_dir()
    }

    fn wait_exited(app: &mut App, id: SessionId) {
        for _ in 0..100 {
            app.on_tick();
            if app.session_mut(id).is_some_and(|s| s.is_exited()) {
                return;
            }
            sleep(Duration::from_millis(50));
        }
        panic!("session did not exit");
    }

    #[test]
    fn q_quits_sessions_view() {
        let mut app = App::new();
        app.running = true;
        press(&mut app, KeyCode::Char('q'));
        assert!(!app.running);
    }

    #[test]
    fn create_adds_session_and_focuses_it() {
        let mut app = App::new();
        assert!(app.create_session("one", &SH, &tmp()));
        assert_eq!(app.sessions_by_id.len(), 1);
        assert_eq!(app.sessions.model.items.len(), 1);
        assert_eq!(app.view, View::Agent);
        let id = app.sessions_by_id[0].id;
        assert_eq!(app.focused, Some(id));
        assert_eq!(app.recent, vec![id]);
        assert!(app.sessions_by_id[0].pty.is_some());
    }

    #[test]
    fn create_with_missing_binary_shows_error_and_adds_nothing() {
        let mut app = App::new();
        assert!(!app.create_session("x", &NO_SUCH, &tmp()));
        assert_eq!(app.sessions_by_id.len(), 0);
        assert_eq!(app.view, View::Sessions);
        assert_eq!(
            app.status_line.message(),
            "`a4s-no-such-cmd` not found on PATH"
        );
    }

    #[test]
    fn duplicate_name_rejected() {
        let mut app = App::new();
        assert!(app.create_session("one", &SH, &tmp()));
        assert!(!app.create_session("one", &SH, &tmp()));
        assert_eq!(app.sessions_by_id.len(), 1);
        assert!(app.status_line.message().contains("already in use"));
        assert!(!app.create_session("  ", &SH, &tmp()));
        assert!(!app.create_session("two", &SH, &tmp().join("a4s-nope")));
        assert_eq!(app.sessions_by_id.len(), 1);
    }

    #[test]
    fn kill_requires_confirm() {
        let mut app = App::new();
        app.create_session("one", &SH, &tmp());
        app.view = View::Sessions;
        press(&mut app, KeyCode::Char('K'));
        assert!(app.confirm_dialog.is_some());
        assert!(!app.sessions_by_id[0].is_exited(), "not before confirm");
        press(&mut app, KeyCode::Left); // select Yes
        press(&mut app, KeyCode::Enter);
        assert!(app.confirm_dialog.is_none());
        assert_eq!(
            app.sessions_by_id[0].status.status(),
            Status::Exited(KILLED_CODE)
        );
        assert!(app.sessions_by_id[0].pty.is_none());
        assert!(app.sessions.model.items[0].state.starts_with('✗'));
    }

    #[test]
    fn kill_cancelled_keeps_session_running() {
        let mut app = App::new();
        app.create_session("one", &SH, &tmp());
        app.view = View::Sessions;
        press(&mut app, KeyCode::Char('K'));
        press(&mut app, KeyCode::Enter); // default is No
        assert!(!app.sessions_by_id[0].is_exited());
    }

    #[test]
    fn forget_only_when_exited() {
        let mut app = App::new();
        app.create_session("one", &SH, &tmp());
        app.view = View::Sessions;
        press(&mut app, KeyCode::Char('F'));
        assert!(app.confirm_dialog.is_none());
        assert!(app.status_line.message().contains("Kill one"));
        let id = app.sessions_by_id[0].id;
        app.kill_session(id);
        press(&mut app, KeyCode::Char('F'));
        assert!(app.confirm_dialog.is_some());
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.sessions_by_id.len(), 0);
        assert_eq!(app.sessions.model.items.len(), 0);
        assert_eq!(app.recent.len(), 0);
    }

    #[test]
    fn restart_only_when_exited_and_resets_status() {
        let mut app = App::new();
        app.create_session("one", &SH, &tmp());
        let id = app.sessions_by_id[0].id;
        app.restart_session(id);
        assert!(app.status_line.message().contains("exited"));
        app.kill_session(id);
        app.restart_session(id);
        let s = &app.sessions_by_id[0];
        assert!(s.pty.is_some());
        assert_eq!(s.status.status(), Status::Idle);
        assert_eq!(s.id, id);
    }

    #[test]
    fn keys_to_exited_session_are_dropped() {
        let mut app = App::new();
        assert!(app.create_session("one", &EXIT3, &tmp()));
        let id = app.sessions_by_id[0].id;
        wait_exited(&mut app, id);
        assert_eq!(app.sessions_by_id[0].status.status(), Status::Exited(3));
        assert!(app.sessions_by_id[0].pty.is_some(), "last screen kept");
        assert!(!app.send_to_session(id, b"hello\n"));
        app.kill_session(id);
        assert!(!app.send_to_session(id, b"hello\n"), "no pty");
        assert!(!app.send_to_session(9999, b"x"), "unknown id");
    }

    #[test]
    fn bell_only_for_background_session_turning_blocked() {
        let mut app = App::new();
        app.create_session("one", &SH, &tmp());
        let id = app.sessions_by_id[0].id;
        app.on_hook(id, Status::Blocked);
        assert_eq!(app.bells, 0, "focused session is on screen");
        app.on_hook(id, Status::Working);
        app.view = View::Sessions;
        app.on_hook(id, Status::Blocked);
        assert_eq!(app.bells, 1);
        app.on_hook(id, Status::Blocked);
        assert_eq!(app.bells, 1, "only on the transition");
    }

    #[test]
    fn output_event_runs_observe() {
        let mut app = App::new();
        app.create_session("one", &SH, &tmp());
        let id = app.sessions_by_id[0].id;
        app.view = View::Sessions;
        app.session_mut(id)
            .unwrap()
            .pty
            .as_mut()
            .unwrap()
            .write(b"echo Proceed?\n")
            .unwrap();
        for _ in 0..100 {
            app.on_output(id);
            if app.sessions_by_id[0].status.status() == Status::Blocked {
                break;
            }
            sleep(Duration::from_millis(50));
        }
        assert_eq!(app.sessions_by_id[0].status.status(), Status::Blocked);
        assert!(app.bells >= 1);
    }

    #[test]
    fn new_modal_opens_and_cancels() {
        let mut app = App::new();
        press(&mut app, KeyCode::Char('c'));
        assert!(app.new_modal.is_some());
        press(&mut app, KeyCode::Esc);
        assert!(app.new_modal.is_none());
    }

    #[test]
    fn sessions_table_renders() {
        let created = SystemTime::UNIX_EPOCH;
        let now = created + Duration::from_secs(125);
        let mut app = App::new();
        app.build_info = String::new();
        for (i, (name, dir)) in
            [("alpha", "/work/alpha"), ("beta", "/work/beta")]
                .into_iter()
                .enumerate()
        {
            app.sessions_by_id.push(Session {
                id: i as u64 + 1,
                name: name.into(),
                agent: &SH,
                dir: dir.into(),
                pty: None,
                status: StatusTracker::new(Instant::now()),
                created,
            });
        }
        app.sessions_by_id[1].status.exited(2);
        app.refresh_table_at(now);
        let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let cells = terminal.backend().buffer().content();
        let mut text = String::new();
        for row in cells.chunks(100) {
            let line: String = row.iter().map(Cell::symbol).collect();
            writeln!(text, "{}", line.trim_end()).unwrap();
        }
        insta::assert_snapshot!(text);
    }
}
