use core::time::Duration;
use std::sync::Arc;

use crossterm::event::{Event, KeyEventKind, poll, read};
use k9tui::{
    theme,
    widgets::{
        constraint_len_calculator,
        hotkey::Hotkey,
        modal::ConfirmDialog,
        status_line::StatusLine,
        table::{TableDataState, filter_rows},
    },
};
use ratatui::DefaultTerminal;
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    task::JoinHandle,
    time::sleep,
};

use crate::{
    app_state::AppState,
    command::{Command, resolve},
    error::Result,
    proc::{
        ProcessRow, SortKey, SystemInfo,
        monitor::{ProcessSource, Snapshot},
        signals, sort_rows,
    },
    ui::widgets::hotkeys::list_hotkeys,
};

/// ASCII logo shown in the top bar.
pub const APP_NAME: &str = r"_________________
\______ \______  \______
 |     __/  /    /  ___/
 |    |    /    /\___  \
 |____|   /____//____  /
                     \/
";

pub const PKG_NAME: &str = env!("CARGO_PKG_NAME");
pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

const POLL_INTERVAL: Duration = Duration::from_millis(1500);

/// Update pushed from the background poller into the app's event loop.
pub enum BackgroundEvent {
    /// A fresh process list + system totals.
    Processes(Box<Snapshot>),
}

#[allow(clippy::struct_excessive_bools)]
pub struct App {
    pub(crate) running: bool,
    pub(crate) state: AppState,
    pub(crate) source: Arc<dyn ProcessSource>,
    pub(crate) hotkeys: Vec<Hotkey>,
    pub(crate) processes: TableDataState<ProcessRow>,
    /// Unfiltered rows from the last poll.
    pub(crate) all: Vec<ProcessRow>,
    pub(crate) info: SystemInfo,
    /// `(pid, start_time)` the highlight is pinned to, while following.
    pub(crate) following: Option<(i32, u64)>,
    /// Case-insensitive substring filter applied to the process list.
    pub(crate) filter: String,
    /// True while the `/` search bar is open for editing.
    pub(crate) search_open: bool,
    /// With `search_open`: the bar is the `:` command line, not the filter.
    pub(crate) command_mode: bool,
    pub(crate) command_text: String,
    pub(crate) sort_key: SortKey,
    pub(crate) sort_desc: bool,
    pub(crate) status_line: StatusLine,
    pub(crate) confirm_dialog: Option<ConfirmDialog>,
    /// `(pid, start_time, signal)` pending once the confirm dialog resolves.
    pub(crate) pending_signal: Option<(i32, u64, sysinfo::Signal)>,
    /// Absolute index of the first visible line in the describe view.
    pub(crate) describe_scroll: usize,
    /// Height of the last-rendered describe viewport, used to clamp scrolling.
    pub(crate) describe_viewport_height: usize,
    /// Top-bar stats show sizes and core counts instead of percentages.
    pub(crate) absolute_stats: bool,
    pub(crate) bg_tx: UnboundedSender<BackgroundEvent>,
    pub(crate) bg_rx: UnboundedReceiver<BackgroundEvent>,
    poll_task: Option<JoinHandle<()>>,
}

impl App {
    #[must_use]
    pub fn new(source: Arc<dyn ProcessSource>) -> Self {
        let (bg_tx, bg_rx) = unbounded_channel();
        let sort_key = SortKey::default();
        Self {
            running: false,
            state: AppState::default(),
            source,
            hotkeys: list_hotkeys(),
            processes: TableDataState::new(Vec::new()),
            all: Vec::new(),
            info: SystemInfo::default(),
            following: None,
            filter: String::new(),
            search_open: false,
            command_mode: false,
            command_text: String::new(),
            sort_key,
            sort_desc: sort_key.starts_descending(),
            status_line: StatusLine::new(),
            confirm_dialog: None,
            pending_signal: None,
            describe_scroll: 0,
            describe_viewport_height: 0,
            absolute_stats: false,
            bg_tx,
            bg_rx,
            poll_task: None,
        }
    }

    /// Run the application's main loop.
    pub fn run(&mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.running = true;
        // First sample synchronously so the opening frame is not empty.
        self.apply_snapshot(self.source.snapshot());
        self.start_polling();

        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            self.drain_background_events();
            self.poll_terminal_event()?;
        }
        Ok(())
    }

    fn start_polling(&mut self) {
        if let Some(handle) = self.poll_task.take() {
            handle.abort();
        }
        let tx = self.bg_tx.clone();
        let source = Arc::clone(&self.source);
        self.poll_task = Some(tokio::spawn(async move {
            loop {
                // Stop once the app has quit and dropped its receiver.
                if tx.is_closed() {
                    return;
                }
                let event =
                    BackgroundEvent::Processes(Box::new(source.snapshot()));
                if tx.send(event).is_err() {
                    return;
                }
                sleep(POLL_INTERVAL).await;
            }
        }));
    }

    fn drain_background_events(&mut self) {
        while let Ok(event) = self.bg_rx.try_recv() {
            match event {
                BackgroundEvent::Processes(snapshot) => {
                    self.apply_snapshot(*snapshot);
                }
            }
        }
    }

    /// Poll crossterm for input with a short timeout so the loop keeps spinning
    /// to pick up background updates.
    fn poll_terminal_event(&mut self) -> Result<()> {
        if !poll(Duration::from_millis(100))? {
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

    pub(crate) fn selected_row(&self) -> Option<&ProcessRow> {
        let index = self.processes.view.state.selected()?;
        self.processes.model.items.get(index)
    }

    /// Replace the master rows and rebuild the visible table.
    pub(crate) fn apply_snapshot(&mut self, snapshot: Snapshot) {
        self.all = snapshot.rows;
        self.info = snapshot.info;
        self.rebuild_view();
    }

    /// Re-filter and re-sort. The highlight keeps its row number, moving only
    /// when the list shrinks past it, unless a process is being followed.
    pub(crate) fn rebuild_view(&mut self) {
        let mut rows = filter_rows(&self.all, &self.filter);
        sort_rows(&mut rows, self.sort_key, self.sort_desc);
        // Widths only grow, so columns do not shift between polls.
        let lens = constraint_len_calculator(&rows);
        let widest = &mut self.processes.model.longest_item_lens;
        widest.resize(lens.len().max(widest.len()), 0);
        for (width, len) in widest.iter_mut().zip(lens) {
            *width = (*width).max(len);
        }
        self.processes.model.items = rows;
        let followed = self.followed_row();
        if let (Some((pid, _)), None) = (self.following, followed) {
            self.following = None;
            self.set_status(format!("Stopped following PID {pid}"));
        }
        let row = followed
            .or_else(|| self.processes.view.state.selected())
            .unwrap_or(0);
        let last = self.processes.model.items.len().checked_sub(1);
        self.processes
            .view
            .state
            .select(last.map(|last| row.min(last)));
    }

    /// Visible row of the followed process, if it is still listed.
    fn followed_row(&self) -> Option<usize> {
        let (pid, start_time) = self.following?;
        self.processes
            .model
            .items
            .iter()
            .position(|r| r.pid == pid && r.start_time == start_time)
    }

    /// `f`: pin the highlight to the selected process, or release it.
    pub(crate) fn toggle_follow(&mut self) {
        self.following = match self.following {
            Some(_) => None,
            None => self.selected_row().map(|r| (r.pid, r.start_time)),
        };
    }

    pub(crate) const fn quit(&mut self) {
        self.running = false;
    }

    pub(crate) fn set_status<S: Into<String>>(&mut self, message: S) {
        self.status_line.set_message(message);
    }

    /// Select a sort key; pressing the active key again flips the direction.
    pub(crate) fn set_sort(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.sort_desc = !self.sort_desc;
        } else {
            self.sort_key = key;
            self.sort_desc = key.starts_descending();
        }
        self.rebuild_view();
        self.set_status(format!(
            "Sort: {} {}",
            key.label(),
            if self.sort_desc { "desc" } else { "asc" }
        ));
    }

    /// `u`: switch the top-bar stats between percentages and actual values.
    pub(crate) fn toggle_stat_units(&mut self) {
        self.absolute_stats = !self.absolute_stats;
        self.set_status(if self.absolute_stats {
            "Stats: actual values"
        } else {
            "Stats: percentages"
        });
    }

    /// `d`/Enter: open the describe view for the selection.
    pub(crate) fn open_describe(&mut self) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        match self.source.detail(row.pid) {
            Some(text) => {
                self.describe_scroll = 0;
                self.state = AppState::Describe {
                    pid: row.pid,
                    name: row.name,
                    text,
                };
            }
            None => self.set_status(format!("No details for PID {}", row.pid)),
        }
    }

    /// `i`: show the system summary panel.
    pub(crate) fn open_info(&mut self) {
        self.state = AppState::Info;
    }

    /// `K`/Delete: open the signal picker for the selection.
    pub(crate) fn open_signal_picker(&mut self) {
        if signals::menu().is_empty() {
            self.set_status("Signals are not supported on this platform");
            return;
        }
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        self.state = AppState::Signal {
            pid: row.pid,
            name: row.name,
            start_time: row.start_time,
            index: 0,
        };
    }

    /// Open the confirmation for sending `sig` to the selected process.
    pub(crate) fn prompt_signal(
        &mut self,
        pid: i32,
        name: &str,
        start_time: u64,
        sig: sysinfo::Signal,
    ) {
        self.pending_signal = Some((pid, start_time, sig));
        self.confirm_dialog = Some(ConfirmDialog::new(
            " Send signal ",
            format!("Send {} to '{name}' (PID {pid})?", signals::label(sig)),
            theme::modal_danger_border(),
            1, // default to No (k9s-style)
        ));
    }

    /// Actually send a signal and report the outcome.
    pub(crate) fn send_signal(
        &mut self,
        pid: i32,
        start_time: u64,
        sig: sysinfo::Signal,
    ) {
        let report = self.source.signal(pid, start_time, sig);
        self.set_status(report.message(pid, sig));
    }

    pub(crate) fn run_command(&mut self, text: &str) {
        match resolve(text) {
            Command::Empty => {}
            Command::Sort(key) => self.set_sort(key),
            Command::Info => self.open_info(),
            Command::Quit => self.quit(),
            Command::Unknown => {
                self.set_status(format!("Unknown command: {}", text.trim()));
            }
        }
    }

    pub(crate) fn close_command_bar(&mut self) {
        self.command_mode = false;
        self.search_open = false;
        self.command_text.clear();
    }

    /// Move the highlighted entry in the signal picker.
    pub(crate) const fn set_signal_index(&mut self, index: usize) {
        if let AppState::Signal { index: current, .. } = &mut self.state {
            *current = index;
        }
    }
}
