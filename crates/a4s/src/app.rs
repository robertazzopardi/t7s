use core::time::Duration;
use std::{
    collections::HashSet,
    env,
    io::stdout,
    panic::{set_hook, take_hook},
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};

use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags, read,
    },
    execute,
    terminal::supports_keyboard_enhancement,
};
use k9tui::{
    TuiApp,
    config::{AppSpec, Config, defaults},
    error::{Result, err},
    event::poll_input,
    widgets::{
        modal::ConfirmDialog, status_line::StatusLine, table::TableDataState,
    },
};
use ratatui::DefaultTerminal;
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    time::interval,
};

#[cfg(unix)]
use crate::hooks::HookSocket;
use crate::{
    agent::profile,
    chords::Router,
    keymap::SPEC,
    manifest::{self, Entry},
    new_session::NewSessionModal,
    pty::SessionId,
    session::{Session, SessionRow},
    sessions::Pending,
    status::{Status, StatusTracker},
};

pub const APP_NAME: &str = r"        _  _
  __ _ | || |  ___
 / _` || || |_/ __|
| (_| ||__   _\__ \
 \__,_|   |_| |___/
";

pub const PKG_NAME: &str = env!("CARGO_PKG_NAME");
pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Update pushed from a background task into the app's event loop. Later
/// tasks add variants.
pub enum BackgroundEvent {
    /// Status reported by an agent hook (`a4s notify`).
    Hook { id: SessionId, status: Status },
    /// New output arrived on this session's PTY.
    Output(SessionId),
    /// Once a second: idle detection and exit polling.
    Tick,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Sessions,
    Agent,
}

pub struct App {
    pub(crate) running: bool,
    pub(crate) config: Config,
    pub(crate) view: View,
    pub(crate) sessions: TableDataState<SessionRow>,
    pub(crate) status_line: StatusLine,
    pub(crate) confirm_dialog: Option<ConfirmDialog>,
    pub(crate) build_info: String,
    /// Sessions in table order; position + 1 is the `#` and `Ctrl+N` number.
    pub(crate) sessions_by_id: Vec<Session>,
    /// Most recently focused first.
    pub(crate) recent: Vec<SessionId>,
    pub(crate) focused: Option<SessionId>,
    pub(crate) next_id: SessionId,
    pub(crate) new_modal: Option<NewSessionModal>,
    pub(crate) pending: Option<Pending>,
    pub(crate) router: Router,
    /// Default directory for new sessions.
    pub(crate) launch_dir: PathBuf,
    /// BELs sent for newly blocked background sessions.
    pub(crate) bells: u32,
    /// Where manifests are saved; `None` turns persistence off (tests).
    pub(crate) state_home: Option<PathBuf>,
    /// Error from the latest save, reported on exit if quitting with it.
    pub(crate) save_error: Option<String>,
    /// Loaded entries with an unknown agent, re-saved untouched.
    pub(crate) orphans: Vec<Entry>,
    /// Repos whose manifest is already reflected in memory.
    pub(crate) loaded_repos: HashSet<PathBuf>,
    pub(crate) bg_tx: UnboundedSender<BackgroundEvent>,
    pub(crate) bg_rx: UnboundedReceiver<BackgroundEvent>,
    /// Keeps the hook socket alive (and removes it on drop).
    #[cfg(unix)]
    pub(crate) hook_socket: Option<HookSocket>,
}

impl App {
    #[must_use]
    pub fn new() -> Self {
        let (bg_tx, bg_rx) = unbounded_channel();
        Self {
            running: false,
            config: defaults(&SPEC),
            view: View::default(),
            sessions: TableDataState::new(Vec::new()),
            status_line: StatusLine::new(),
            confirm_dialog: None,
            build_info: format!("Name: {PKG_NAME}\nVersion: {PKG_VERSION}"),
            sessions_by_id: Vec::new(),
            recent: Vec::new(),
            focused: None,
            next_id: 1,
            new_modal: None,
            pending: None,
            router: Router::default(),
            launch_dir: env::current_dir().unwrap_or_default(),
            bells: 0,
            state_home: None,
            save_error: None,
            orphans: Vec::new(),
            loaded_repos: HashSet::new(),
            bg_tx,
            bg_rx,
            #[cfg(unix)]
            hook_socket: None,
        }
    }

    /// Path of the hook socket, to pass to agents as `A4S_SOCKET`.
    #[must_use]
    pub fn socket_path(&self) -> Option<&Path> {
        #[cfg(unix)]
        {
            self.hook_socket.as_ref().map(HookSocket::path)
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    pub(crate) fn quit(&mut self) {
        self.persist();
        self.running = false;
    }

    /// Save every session's manifest entry; errors go to the status line.
    pub(crate) fn persist(&mut self) {
        let Some(home) = &self.state_home else { return };
        let entries: Vec<Entry> = self
            .sessions_by_id
            .iter()
            .map(|s| Entry {
                name: s.name.clone(),
                agent: s.agent.name.to_string(),
                dir: s.dir.clone(),
                repo: s.repo.clone(),
            })
            .chain(self.orphans.iter().cloned())
            .collect();
        self.save_error = manifest::save(&entries, home)
            .err()
            .map(|e| format!("could not save sessions: {e}"));
        if let Some(e) = &self.save_error {
            self.status_line.set_message(e.clone());
        }
    }

    /// Turn saved entries into exited sessions; unknown agents become
    /// orphans. Returns the names of the orphaned ones.
    fn add_entries(&mut self, entries: Vec<Entry>) -> Vec<String> {
        let mut unknown = Vec::new();
        for e in entries {
            let Some(agent) = profile(&e.agent) else {
                unknown.push(e.name.clone());
                self.orphans.push(e);
                continue;
            };
            let mut status = StatusTracker::new(Instant::now());
            status.exited(0);
            self.sessions_by_id.push(Session {
                id: self.next_id,
                name: e.name,
                agent,
                dir: e.dir,
                pty: None,
                status,
                created: SystemTime::now(),
                repo: e.repo,
            });
            self.next_id += 1;
        }
        unknown
    }

    fn report_load(&mut self, mut notes: Vec<String>, unknown: &[String]) {
        if !unknown.is_empty() {
            notes.push(format!(
                "unknown agent for: {} (kept in manifest)",
                unknown.join(", ")
            ));
        }
        if !notes.is_empty() {
            self.status_line.set_message(notes.join("; "));
        }
    }

    pub(crate) fn load_sessions(&mut self, home: &Path) {
        let loaded = manifest::load(home);
        self.loaded_repos.extend(loaded.repos);
        let unknown = self.add_entries(loaded.entries);
        self.report_load(loaded.warnings, &unknown);
        self.refresh_table();
    }

    /// Before a repo's manifest is first written, pull in its existing
    /// entries (the global file may have been bad, so it was never loaded)
    /// so saving cannot drop them.
    pub(crate) fn adopt_repo(&mut self, repo: &Path) {
        let Some(home) = self.state_home.clone() else {
            return;
        };
        if self.loaded_repos.contains(repo) {
            return;
        }
        let mut warnings = Vec::new();
        if let Some(entries) = manifest::load_repo(repo, &home, &mut warnings) {
            let fresh = entries
                .into_iter()
                .filter(|e| {
                    !self.sessions_by_id.iter().any(|s| s.name == e.name)
                        && !self.orphans.iter().any(|o| o.name == e.name)
                })
                .collect();
            let unknown = self.add_entries(fresh);
            self.report_load(warnings, &unknown);
            self.loaded_repos.insert(repo.to_path_buf());
        } else {
            self.report_load(warnings, &[]);
        }
    }

    /// Run the application's main loop.
    ///
    /// # Errors
    /// If drawing or reading terminal events fails, or the final save of
    /// the sessions failed (printed once the terminal is restored).
    pub fn run(&mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.running = true;

        while self.running {
            self.drain_background_events();
            terminal.draw(|frame| self.render(frame))?;
            self.poll_terminal_event()?;
        }
        self.save_error.take().map_or(Ok(()), |e| Err(err(e)))
    }

    pub(crate) fn drain_background_events(&mut self) {
        let mut changed = false;
        let mut output = Vec::new();
        let mut tick = false;
        while let Ok(event) = self.bg_rx.try_recv() {
            changed = true;
            match event {
                BackgroundEvent::Hook { id, status } => {
                    self.on_hook(id, status);
                }
                BackgroundEvent::Output(id) => {
                    if !output.contains(&id) {
                        output.push(id);
                    }
                }
                BackgroundEvent::Tick => tick = true,
            }
        }
        // Once per session per drain, not once per output chunk.
        for id in output {
            self.on_output(id);
        }
        // Outputs first so the tick sees fresh `last_output`.
        if tick {
            self.on_tick();
        }
        if changed {
            self.refresh_table();
        }
    }

    fn poll_terminal_event(&mut self) -> Result<()> {
        let bg_rx = &self.bg_rx;
        if !poll_input(Duration::from_millis(100), || !bg_rx.is_empty())? {
            return Ok(());
        }
        match read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.on_key_event(key);
            }
            Event::Mouse(m) => self.on_mouse(m),
            Event::Paste(text) => self.on_paste(&text),
            // Resizes are picked up by the next frame.
            Event::Key(_)
            | Event::FocusGained
            | Event::FocusLost
            | Event::Resize(_, _) => {}
        }
        Ok(())
    }
}

impl TuiApp for App {
    const SPEC: AppSpec = SPEC;
    type Args = ();

    fn build(config: Config, (): ()) -> Result<Self> {
        let mut app = Self::new();
        app.config = config;
        let home = manifest::state_home();
        app.load_sessions(&home);
        app.state_home = Some(home);
        Ok(app)
    }

    fn start(&mut self, terminal: DefaultTerminal) -> Result<()> {
        #[cfg(unix)]
        {
            // Hooks are best effort: run without them if binding fails.
            self.hook_socket = HookSocket::bind(self.bg_tx.clone()).ok();
        }
        let tx = self.bg_tx.clone();
        tokio::spawn(async move {
            let mut every = interval(Duration::from_secs(1));
            while tx.send(BackgroundEvent::Tick).is_ok() {
                every.tick().await;
            }
        });
        let _guard = InputModes::enter();
        self.run(terminal)
    }
}

/// Keyboard enhancement (for Ctrl+digit chords) and mouse capture, undone
/// on drop and on panic.
struct InputModes {
    enhanced: bool,
}

impl InputModes {
    fn enter() -> Self {
        let enhanced = supports_keyboard_enhancement().unwrap_or(false);
        if enhanced {
            let _ = execute!(
                stdout(),
                PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                )
            );
        }
        let _ = execute!(stdout(), EnableMouseCapture);
        let prev_hook = take_hook();
        set_hook(Box::new(move |info| {
            restore(enhanced);
            prev_hook(info);
        }));
        Self { enhanced }
    }
}

impl Drop for InputModes {
    fn drop(&mut self) {
        restore(self.enhanced);
    }
}

fn restore(enhanced: bool) {
    let _ = execute!(stdout(), DisableMouseCapture);
    if enhanced {
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use super::*;

    #[test]
    fn persist_then_load_keeps_sessions_and_orphans() {
        let home = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let entry = |name: &str, agent: &str| Entry {
            name: name.into(),
            agent: agent.into(),
            dir: dir.path().into(),
            repo: None,
        };
        let saved = [entry("a", "claude"), entry("b", "nope")];
        manifest::save(&saved, home.path()).unwrap();

        let mut app = App::new();
        app.load_sessions(home.path());
        assert_eq!(app.sessions_by_id.len(), 1);
        assert!(
            app.sessions_by_id
                .iter()
                .all(|s| s.is_exited() && s.pty.is_none())
        );
        assert_eq!(app.orphans, vec![entry("b", "nope")]);

        app.state_home = Some(home.path().into());
        app.persist();
        let mut got = manifest::load(home.path()).entries;
        got.sort_by(|x, y| x.name.cmp(&y.name));
        assert_eq!(got, saved);
    }

    #[test]
    fn new_session_cannot_reuse_orphan_name() {
        let mut app = App::new();
        app.orphans.push(Entry {
            name: "ghost".into(),
            agent: "nope".into(),
            dir: "/".into(),
            repo: None,
        });
        let agent = profile("claude").unwrap();
        assert!(!app.create_session("ghost", agent, Path::new("/")));
        assert!(app.sessions_by_id.is_empty());
    }

    #[test]
    fn bad_global_does_not_drop_repo_entries() {
        let (home, repo) =
            (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let ok = Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(repo.path())
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let r = manifest::repo_root(repo.path()).unwrap();
        let entry = |name: &str| Entry {
            name: name.into(),
            agent: "claude".into(),
            dir: r.clone(),
            repo: Some(r.clone()),
        };
        manifest::save(&[entry("A")], home.path()).unwrap();
        let global = manifest::manifest_path(None, home.path());
        fs::write(&global, "bad = [").unwrap();

        let mut app = App::new();
        app.load_sessions(home.path());
        assert!(app.sessions_by_id.is_empty());
        app.state_home = Some(home.path().into());
        app.adopt_repo(&r);
        let _ = app.add_entries(vec![entry("B")]);
        app.persist();
        let mut names: Vec<_> = manifest::load(home.path())
            .entries
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, ["A", "B"]);
    }
}
