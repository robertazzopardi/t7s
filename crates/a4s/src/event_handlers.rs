use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
};
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
    chords::{AppAction, Route},
    keys::{encode, encode_paste},
    new_session::NewSessionModal,
    pty::{Pty, SessionId},
    sessions::Pending,
    status::Status,
};

impl App {
    pub fn on_key_event(&mut self, key: KeyEvent) {
        if self.confirm_dialog.is_some() {
            self.on_key_confirm(key);
        } else if self.new_modal.is_some() {
            self.on_key_new_modal(key);
        } else {
            match self.view {
                View::Sessions => self.on_key_sessions(key),
                View::Agent => self.on_key_agent(key),
            }
        }
    }

    /// Bracketed paste from the terminal, forwarded to the focused agent.
    pub fn on_paste(&mut self, text: &str) {
        if self.view != View::Agent
            || self.new_modal.is_some()
            || self.confirm_dialog.is_some()
        {
            return;
        }
        self.send_focused(|p| {
            encode_paste(text, p.screen().screen().bracketed_paste())
        });
    }

    /// Wheel scrolls the focused agent's scrollback by 3 lines.
    #[allow(clippy::wildcard_enum_match_arm)]
    pub fn on_mouse(&mut self, m: MouseEvent) {
        if self.view != View::Agent {
            return;
        }
        match m.kind {
            MouseEventKind::ScrollUp => {
                self.scroll_focused(|n| n.saturating_add(3));
            }
            MouseEventKind::ScrollDown => {
                self.scroll_focused(|n| n.saturating_sub(3));
            }
            _ => {}
        }
    }

    fn scroll_focused(&mut self, f: impl FnOnce(usize) -> usize) {
        let Some(id) = self.focused else {
            return;
        };
        if let Some(p) = self.session_mut(id).and_then(|s| s.pty.as_ref()) {
            let mut parser = p.screen();
            let now = parser.screen().scrollback();
            parser.screen_mut().set_scrollback(f(now));
        }
    }

    /// Encode with the focused PTY's modes and write it; any input snaps
    /// the view back to the live screen.
    fn send_focused(&mut self, encode_with: impl FnOnce(&Pty) -> Vec<u8>) {
        let Some(id) = self.focused else {
            return;
        };
        self.scroll_focused(|_| 0);
        let bytes = self
            .session_mut(id)
            .and_then(|s| s.pty.as_ref())
            .map(encode_with);
        if let Some(bytes) = bytes {
            self.send_to_session(id, &bytes);
        }
    }

    fn on_key_agent(&mut self, key: KeyEvent) {
        match self.router.route(key) {
            Route::Agent(k) => self.send_focused(|p| {
                encode(
                    k,
                    p.kitty_keys(),
                    p.screen().screen().application_cursor(),
                )
            }),
            Route::App(action) => self.run_action(action),
            Route::Pending | Route::Ignore => {}
        }
    }

    fn run_action(&mut self, action: AppAction) {
        let count = self.sessions_by_id.len();
        let current = self
            .focused
            .and_then(|f| self.sessions_by_id.iter().position(|s| s.id == f));
        let focus_index = |app: &mut Self, i: usize| {
            if let Some(id) = app.sessions_by_id.get(i).map(|s| s.id) {
                app.focus_session(id);
            }
        };
        match action {
            AppAction::Quit => self.request_quit(),
            AppAction::Focus(n) => focus_index(self, n - 1),
            AppAction::Next if count > 0 => {
                focus_index(self, current.map_or(0, |i| (i + 1) % count));
            }
            AppAction::Prev if count > 0 => {
                focus_index(self, current.map_or(0, |i| (i + count - 1) % count));
            }
            AppAction::Next | AppAction::Prev => {}
            AppAction::Sessions => self.view = View::Sessions,
            AppAction::New => self.open_new_modal(),
            AppAction::Restart => {
                if let Some(id) = self.focused {
                    self.restart_session(id);
                }
            }
            AppAction::Help => self.status_line.set_message(
                "Ctrl+a: n next, p prev, s sessions, c new, r restart, 1-9 focus; Ctrl+q quit",
            ),
        }
    }

    fn open_new_modal(&mut self) {
        self.new_modal =
            Some(NewSessionModal::new(&self.launch_dir.display().to_string()));
    }

    /// Quit, asking first when agents are still busy.
    fn request_quit(&mut self) {
        let busy = self
            .sessions_by_id
            .iter()
            .filter(|s| {
                matches!(s.status.status(), Status::Working | Status::Blocked)
            })
            .count();
        if busy == 0 {
            self.quit();
            return;
        }
        self.pending = Some(Pending::Quit);
        self.confirm_dialog = Some(ConfirmDialog::new(
            " Quit ",
            format!("{busy} agents are still running. Quit and stop them?"),
            theme::modal_danger_border(),
            1,
        ));
    }

    fn on_key_sessions(&mut self, key: KeyEvent) {
        if is_ctrl_c(key)
            || (key.code == KeyCode::Char('q')
                && key.modifiers == KeyModifiers::CONTROL)
        {
            self.request_quit();
            return;
        }
        if let KeyCode::Char(c @ '1'..='9') = key.code
            && key.modifiers == KeyModifiers::NONE
        {
            let n = c.to_digit(10).map_or(0, |d| d as usize);
            if let Some(id) = self.sessions_by_id.get(n - 1).map(|s| s.id) {
                self.focus_session(id);
            }
            return;
        }
        let screen = "sessions";
        let selected = self.selected_session().map(|(id, ..)| id);
        match self.config.keymap.action(screen, plain(key)) {
            "quit" => self.request_quit(),
            "new" => self.open_new_modal(),
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
                    (DialogAction::Submit, Some(Pending::Quit)) => self.quit(),
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

    fn ctrl(app: &mut App, c: char) {
        app.on_key_event(KeyEvent::new(
            KeyCode::Char(c),
            KeyModifiers::CONTROL,
        ));
    }

    fn type_str(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn wait_screen(app: &mut App, id: SessionId, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let text = app
                .session_mut(id)
                .and_then(|s| {
                    s.pty.as_ref().map(|p| p.screen().screen().contents())
                })
                .unwrap_or_default();
            if text.contains(needle) {
                return;
            }
            assert!(Instant::now() < deadline, "no {needle:?} in {text:?}");
            sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn typed_keys_reach_focused_agent() {
        let mut app = App::new();
        app.create_session("one", &SH, &tmp());
        let id = app.sessions_by_id[0].id;
        type_str(&mut app, "echo $((20+22))");
        press(&mut app, KeyCode::Enter);
        wait_screen(&mut app, id, "\n42");
    }

    #[test]
    fn ctrl_c_reaches_agent_not_quit() {
        let mut app = App::new();
        app.running = true;
        app.create_session("one", &SH, &tmp());
        ctrl(&mut app, 'c');
        assert!(app.running);
        assert_eq!(app.view, View::Agent);
        assert!(app.confirm_dialog.is_none());
    }

    #[test]
    fn ctrl_number_switches_focus() {
        let mut app = App::new();
        app.create_session("one", &SH, &tmp());
        app.create_session("two", &SH, &tmp());
        let (a, b) = (app.sessions_by_id[0].id, app.sessions_by_id[1].id);
        assert_eq!(app.focused, Some(b));
        ctrl(&mut app, '1');
        assert_eq!(app.focused, Some(a));
        assert_eq!(app.recent, vec![a, b]);
        ctrl(&mut app, '9'); // no such session
        assert_eq!(app.focused, Some(a));
    }

    #[test]
    fn prefix_s_leaves_agent_view_and_digit_reenters() {
        let mut app = App::new();
        app.create_session("one", &SH, &tmp());
        ctrl(&mut app, 'a');
        press(&mut app, KeyCode::Char('s'));
        assert_eq!(app.view, View::Sessions);
        press(&mut app, KeyCode::Char('1'));
        assert_eq!(app.view, View::Agent);
    }

    #[test]
    fn paste_is_forwarded() {
        let mut app = App::new();
        app.create_session("one", &SH, &tmp());
        let id = app.sessions_by_id[0].id;
        app.on_paste("echo $((20+24))\n");
        wait_screen(&mut app, id, "\n44");
    }

    #[test]
    fn quit_with_working_agent_asks_first() {
        let mut app = App::new();
        app.running = true;
        app.create_session("one", &SH, &tmp());
        let id = app.sessions_by_id[0].id;
        ctrl(&mut app, 'q');
        assert!(!app.running, "idle agents quit without asking");
        app.running = true;
        app.on_hook(id, Status::Working);
        ctrl(&mut app, 'q');
        assert!(app.running);
        assert!(app.confirm_dialog.is_some());
        press(&mut app, KeyCode::Enter); // default No
        assert!(app.running);
        ctrl(&mut app, 'q');
        press(&mut app, KeyCode::Left);
        press(&mut app, KeyCode::Enter);
        assert!(!app.running);
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
