//! Session lifecycle on `App`: create, focus, kill, forget, restart, and
//! reacting to PTY output, hooks and the once-a-second tick.

use std::{
    io::{self, Write, stdout},
    path::Path,
    time::{Instant, SystemTime},
};

use crossterm::terminal;
use k9tui::widgets::{
    status_line::FOOTER_HEIGHT, table::TableDataState, top_bar::TOPBAR_HEIGHT,
};

use crate::{
    agent::{AgentProfile, on_path, resolve_command},
    app::{App, BackgroundEvent, View},
    defaults::KILLED_CODE,
    pty::{Pty, SessionId},
    session::Session,
    status::{Status, StatusTracker},
};

/// What a pending confirm dialog will act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    Kill(SessionId),
    Forget(SessionId),
}

/// PTY size used before the agent view resizes it to the real pane.
fn initial_size() -> (u16, u16) {
    terminal::size().map_or((24, 80), |(cols, rows)| {
        (
            rows.saturating_sub(TOPBAR_HEIGHT + FOOTER_HEIGHT + 2)
                .max(1),
            cols.saturating_sub(2).max(1),
        )
    })
}

/// Resolved launch command, or the status-line text for a missing binary.
fn launch_command(
    profile: &AgentProfile,
    resume: bool,
) -> Result<Vec<String>, String> {
    let cmd = resolve_command(
        profile.resume.filter(|_| resume).unwrap_or(profile.command),
    );
    match cmd.first() {
        Some(prog) if on_path(prog) => Ok(cmd),
        Some(prog) => Err(format!("`{prog}` not found on PATH")),
        None => Err("empty agent command".into()),
    }
}

impl App {
    pub(crate) fn session_mut(
        &mut self,
        id: SessionId,
    ) -> Option<&mut Session> {
        self.sessions_by_id.iter_mut().find(|s| s.id == id)
    }

    /// Index into `sessions_by_id` of the selected table row.
    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.sessions
            .view
            .state
            .selected()
            .filter(|i| *i < self.sessions_by_id.len())
    }

    pub(crate) fn refresh_table(&mut self) {
        self.refresh_table_at(SystemTime::now());
    }

    /// Rebuild table rows, keeping the selection in range.
    pub(crate) fn refresh_table_at(&mut self, now: SystemTime) {
        let rows: Vec<_> = self
            .sessions_by_id
            .iter()
            .enumerate()
            .map(|(i, s)| s.row(i + 1, now))
            .collect();
        let selected = self.sessions.view.state.selected();
        let last = rows.len().checked_sub(1);
        self.sessions = TableDataState::new(rows);
        self.sessions
            .view
            .state
            .select(last.map(|l| selected.unwrap_or(0).min(l)));
    }

    fn spawn_pty(
        &self,
        id: SessionId,
        cmd: &[String],
        dir: &Path,
    ) -> io::Result<Pty> {
        let mut env = vec![("A4S_SESSION_ID".to_string(), id.to_string())];
        if let Some(sock) = self.socket_path() {
            env.push(("A4S_SOCKET".into(), sock.display().to_string()));
        }
        let (rows, cols) = initial_size();
        let tx = self.bg_tx.clone();
        Pty::spawn(cmd, dir, &env, rows, cols, move || {
            let _ = tx.send(BackgroundEvent::Output(id));
        })
    }

    /// Validate, spawn and focus a new session. On failure the reason goes
    /// to the status line, nothing is added, and `false` is returned.
    pub fn create_session(
        &mut self,
        name: &str,
        profile: &'static AgentProfile,
        dir: &Path,
    ) -> bool {
        let name = name.trim();
        let error = if name.is_empty() {
            Some("name must not be empty".to_string())
        } else if self.sessions_by_id.iter().any(|s| s.name == name) {
            Some(format!("name `{name}` is already in use"))
        } else if !dir.is_dir() {
            Some(format!("directory `{}` does not exist", dir.display()))
        } else {
            None
        };
        if let Some(e) = error {
            self.status_line.set_message(e);
            return false;
        }
        let cmd = match launch_command(profile, false) {
            Ok(cmd) => cmd,
            Err(e) => {
                self.status_line.set_message(e);
                return false;
            }
        };
        let id = self.next_id;
        let pty = match self.spawn_pty(id, &cmd, dir) {
            Ok(pty) => pty,
            Err(e) => {
                self.status_line.set_message(format!(
                    "failed to start {}: {e}",
                    profile.name
                ));
                return false;
            }
        };
        self.next_id += 1;
        self.sessions_by_id.push(Session {
            id,
            name: name.to_string(),
            agent: profile,
            dir: dir.to_path_buf(),
            pty: Some(pty),
            status: StatusTracker::new(Instant::now()),
            created: SystemTime::now(),
        });
        self.refresh_table();
        self.sessions
            .view
            .state
            .select(self.sessions_by_id.len().checked_sub(1));
        self.focus_session(id);
        true
    }

    pub(crate) fn focus_session(&mut self, id: SessionId) {
        self.focused = Some(id);
        self.recent.retain(|r| *r != id);
        self.recent.insert(0, id);
        self.view = View::Agent;
    }

    /// Kill the child by dropping its PTY. The row stays as `Exited` with
    /// the sentinel code `KILLED_CODE` until forgotten or restarted.
    pub(crate) fn kill_session(&mut self, id: SessionId) {
        if let Some(s) = self.session_mut(id)
            && !s.is_exited()
        {
            s.pty = None;
            s.status.exited(KILLED_CODE);
        }
        self.refresh_table();
    }

    pub(crate) fn forget_session(&mut self, id: SessionId) {
        self.sessions_by_id.retain(|s| s.id != id || !s.is_exited());
        let gone = !self.sessions_by_id.iter().any(|s| s.id == id);
        if gone {
            self.recent.retain(|r| *r != id);
            if self.focused == Some(id) {
                self.focused = None;
            }
        }
        self.refresh_table();
    }

    /// Respawn an exited session in its directory, resuming when the
    /// profile supports it. Keeps the id and resets status tracking.
    pub fn restart_session(&mut self, id: SessionId) {
        let Some((agent, dir, exited)) = self
            .session_mut(id)
            .map(|s| (s.agent, s.dir.clone(), s.is_exited()))
        else {
            return;
        };
        if !exited {
            self.status_line
                .set_message("Only exited sessions can be restarted");
            return;
        }
        let spawned = launch_command(agent, true).and_then(|cmd| {
            self.spawn_pty(id, &cmd, &dir)
                .map_err(|e| format!("failed to restart {}: {e}", agent.name))
        });
        match spawned {
            Ok(pty) => {
                if let Some(s) = self.session_mut(id) {
                    s.pty = Some(pty);
                    s.status = StatusTracker::new(Instant::now());
                }
            }
            Err(e) => self.status_line.set_message(e),
        }
        self.refresh_table();
    }

    /// Write bytes to a session's PTY. Returns `false` (writing nothing)
    /// when the session is missing, has no PTY, or has exited.
    pub fn send_to_session(&mut self, id: SessionId, bytes: &[u8]) -> bool {
        self.session_mut(id).is_some_and(|s| {
            !s.is_exited()
                && s.pty.as_mut().is_some_and(|p| p.write(bytes).is_ok())
        })
    }

    /// Apply `f` to a session, ringing the bell if it just became Blocked
    /// while not on screen.
    fn update_status(&mut self, id: SessionId, f: impl FnOnce(&mut Session)) {
        let visible = self.view == View::Agent && self.focused == Some(id);
        let Some(s) = self.session_mut(id) else {
            return;
        };
        let before = s.status.status();
        f(s);
        let now_blocked =
            before != Status::Blocked && s.status.status() == Status::Blocked;
        if now_blocked && !visible {
            self.ring_bell();
        }
    }

    pub(crate) fn on_output(&mut self, id: SessionId) {
        self.update_status(id, |s| {
            if let Some(pty) = &s.pty {
                let text = pty.screen().screen().contents();
                s.status.observe(&text, s.agent, Instant::now());
            }
        });
    }

    pub(crate) fn on_hook(&mut self, id: SessionId, status: Status) {
        self.update_status(id, |s| s.status.hook(status));
    }

    /// Idle detection and exit polling. An exited child keeps its `Pty` so
    /// the last screen stays visible.
    pub(crate) fn on_tick(&mut self) {
        let ids: Vec<_> = self.sessions_by_id.iter().map(|s| s.id).collect();
        for id in ids {
            self.update_status(id, |s| {
                s.status.tick(Instant::now());
                if !s.is_exited()
                    && let Some(code) = s.pty.as_mut().and_then(Pty::try_exit)
                {
                    s.status.exited(code);
                }
            });
        }
        self.refresh_table();
    }

    fn ring_bell(&mut self) {
        self.bells += 1;
        // Tests count bells instead of writing to the real terminal.
        if !cfg!(test) {
            let mut out = stdout();
            let _ = out.write_all(b"\x07");
            let _ = out.flush();
        }
    }
}
