use core::time::Duration;

use crossterm::event::{Event, KeyEventKind, read};
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
use tokio::sync::mpsc::{
    UnboundedReceiver, UnboundedSender, unbounded_channel,
};

use crate::{keymap::SPEC, session::SessionRow};

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
#[allow(dead_code, reason = "later tasks send events")]
pub enum BackgroundEvent {
    /// Message for the status line from a background task.
    Status(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(dead_code, reason = "agent view arrives in a later task")]
pub enum View {
    #[default]
    Sessions,
    Agent,
}

#[allow(dead_code, reason = "fields are used by later tasks")]
pub struct App {
    pub(crate) running: bool,
    pub(crate) config: Config,
    pub(crate) view: View,
    pub(crate) sessions: TableDataState<SessionRow>,
    pub(crate) status_line: StatusLine,
    pub(crate) confirm_dialog: Option<ConfirmDialog>,
    pub(crate) build_info: String,
    pub(crate) bg_tx: UnboundedSender<BackgroundEvent>,
    pub(crate) bg_rx: UnboundedReceiver<BackgroundEvent>,
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
            bg_tx,
            bg_rx,
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
        while let Ok(event) = self.bg_rx.try_recv() {
            match event {
                BackgroundEvent::Status(message) => {
                    self.status_line.set_message(message);
                }
            }
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
        self.run(terminal)
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
