use core::time::Duration;
use std::{
    env,
    io::stdout,
    panic::{set_hook, take_hook},
    path::{Path, PathBuf},
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
    error::Result,
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
    keymap::SPEC,
    new_session::NewSessionModal,
    pty::SessionId,
    session::{Session, SessionRow},
    sessions::Pending,
    status::Status,
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
    /// Message for the status line from a background task.
    Status(String),
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
    /// Default directory for new sessions.
    pub(crate) launch_dir: PathBuf,
    /// BELs sent for newly blocked background sessions.
    pub(crate) bells: u32,
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
            launch_dir: env::current_dir().unwrap_or_default(),
            bells: 0,
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

    pub(crate) const fn quit(&mut self) {
        self.running = false;
    }

    /// Run the application's main loop.
    ///
    /// # Errors
    /// If drawing or reading terminal events fails.
    pub fn run(&mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.running = true;

        while self.running {
            self.drain_background_events();
            terminal.draw(|frame| self.render(frame))?;
            self.poll_terminal_event()?;
        }
        Ok(())
    }

    pub(crate) fn drain_background_events(&mut self) {
        let mut changed = false;
        while let Ok(event) = self.bg_rx.try_recv() {
            changed = true;
            match event {
                BackgroundEvent::Status(message) => {
                    self.status_line.set_message(message);
                }
                BackgroundEvent::Hook { id, status } => {
                    self.on_hook(id, status);
                }
                BackgroundEvent::Output(id) => self.on_output(id),
                BackgroundEvent::Tick => self.on_tick(),
            }
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
            Event::Key(_)
            | Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Paste(_)
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
