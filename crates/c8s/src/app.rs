use core::time::Duration;
use std::{io::stdout, process::Command};

use ansi_to_tui::IntoText;
use crossterm::{
    ExecutableCommand,
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind, poll,
        read,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use k9tui::widgets::{
    constraint_len_calculator,
    hotkey::Hotkey,
    modal::ConfirmDialog,
    status_line::StatusLine,
    table::{ColumnWidthMode, TableData, TableDataState, filter_rows},
};
use ratatui::{
    DefaultTerminal, Terminal,
    backend::Backend,
    text::{Line, Text},
};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    task::JoinHandle,
    time::sleep,
};

use crate::{
    app_state::{AppState, ResourceKind},
    docker::{
        ContainerRow, ImageRow, NetworkRow, ProcessRow, VolumeRow,
        client::DockerClient,
    },
    error::{Result, err},
    log_dedup::DedupMode,
    ui::widgets::hotkeys::LIST_HOTKEYS,
};

pub const APP_NAME: &str = r"         ______
  ____  /  __  \  ______
_/ ___\ >      < /  ___/
\  \___/   --   \\___ \
 \___  >______  /____  >
     \/       \/     \/
";

pub const PKG_NAME: &str = env!("CARGO_PKG_NAME");
pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Update pushed from a background task into the app's event loop.
pub enum BackgroundEvent {
    /// Container list refreshed by the poller.
    Containers(Vec<ContainerRow>),
    /// Image list refreshed by the poller.
    Images(Vec<ImageRow>),
    /// Volume list refreshed by the poller.
    Volumes(Vec<VolumeRow>),
    /// Network list refreshed by the poller.
    Networks(Vec<NetworkRow>),
    /// The poller failed to reach the daemon (e.g. it was stopped mid-session).
    PollError(String),
    /// One log line from the active log-tail task.
    LogLine(String),
    /// Process list refreshed by the active process-view poller.
    Processes(Vec<ProcessRow>),
    /// `docker top` failed (e.g. the container is not running).
    ProcessError(String),
    /// Message for the status line from a background task.
    Status(String),
}

#[allow(clippy::struct_excessive_bools)]
pub struct App {
    pub(crate) running: bool,
    pub(crate) state: AppState,
    /// Which resource table the list view currently shows.
    pub(crate) view: ResourceKind,
    pub(crate) docker: Option<DockerClient>,
    pub(crate) hotkeys: Vec<Hotkey>,
    pub(crate) containers: TableDataState<ContainerRow>,
    pub(crate) images: TableDataState<ImageRow>,
    pub(crate) volumes: TableDataState<VolumeRow>,
    pub(crate) networks: TableDataState<NetworkRow>,
    pub(crate) processes: TableDataState<ProcessRow>,
    /// Session-wide Fit/Content column sizing (`W` toggles).
    pub(crate) column_width_mode: ColumnWidthMode,
    /// Aborts the running `docker top` poller, if any, when leaving the view.
    pub(crate) process_task: Option<JoinHandle<()>>,
    /// Where `q` goes from the log view when it was not opened from the list.
    pub(crate) logs_back: Option<AppState>,
    /// Unfiltered rows from the last poll, kept so the `/` filter can be
    /// re-applied (or cleared) without waiting for the next poll tick.
    pub(crate) containers_all: Vec<ContainerRow>,
    pub(crate) images_all: Vec<ImageRow>,
    pub(crate) volumes_all: Vec<VolumeRow>,
    pub(crate) networks_all: Vec<NetworkRow>,
    /// Case-insensitive substring filter applied to the active list view(s).
    /// Empty = no filter.
    pub(crate) list_filter: String,
    /// Hide non-running containers (`a` toggles).
    pub(crate) running_only: bool,
    /// True while the `/` search bar is open for editing in the list view.
    pub(crate) list_search_open: bool,
    /// With `list_search_open`: the bar is the `:` command line (text in
    /// `command_text`), not the row filter.
    pub(crate) command_mode: bool,
    pub(crate) command_text: String,
    pub(crate) status_line: StatusLine,
    pub(crate) confirm_dialog: Option<ConfirmDialog>,
    /// Resource kind + id pending removal once the confirm dialog resolves.
    pub(crate) pending_remove: Option<(ResourceKind, String)>,
    pub(crate) log_lines: Vec<Line<'static>>,
    /// Absolute index of the first visible log line, synced each render.
    pub(crate) log_scroll: usize,
    /// True while the log view tracks new output; false once the user scrolls up.
    pub(crate) log_follow: bool,
    /// Height of the last-rendered log viewport, used to clamp scrolling.
    pub(crate) log_viewport_height: usize,
    /// Row count of the last-rendered (deduped + filtered) log view.
    pub(crate) log_view_len: usize,
    pub(crate) log_dedup: DedupMode,
    /// Case-insensitive substring filter applied to log lines. Empty = no filter.
    pub(crate) log_filter: String,
    /// True while the `/` search bar is open for editing.
    pub(crate) log_search_open: bool,
    /// Absolute index of the first visible line in the describe view.
    pub(crate) describe_scroll: usize,
    /// Height of the last-rendered describe viewport, used to clamp scrolling.
    pub(crate) describe_viewport_height: usize,
    pub(crate) build_info: String,

    pub(crate) bg_tx: UnboundedSender<BackgroundEvent>,
    pub(crate) bg_rx: UnboundedReceiver<BackgroundEvent>,
    /// Aborts the running log-tail task, if any, when leaving the log view.
    pub(crate) log_task: Option<JoinHandle<()>>,
    poll_task: Option<JoinHandle<()>>,
}

impl App {
    #[must_use]
    pub fn new() -> Self {
        let (bg_tx, bg_rx) = unbounded_channel();
        Self {
            running: false,
            state: AppState::default(),
            view: ResourceKind::default(),
            docker: None,
            hotkeys: LIST_HOTKEYS.to_vec(),
            containers: TableDataState::new(Vec::new()),
            images: TableDataState::new(Vec::new()),
            volumes: TableDataState::new(Vec::new()),
            networks: TableDataState::new(Vec::new()),
            processes: TableDataState::new(Vec::new()),
            column_width_mode: ColumnWidthMode::Fit,
            process_task: None,
            logs_back: None,
            containers_all: Vec::new(),
            images_all: Vec::new(),
            volumes_all: Vec::new(),
            networks_all: Vec::new(),
            list_filter: String::new(),
            running_only: false,
            list_search_open: false,
            command_mode: false,
            command_text: String::new(),
            status_line: StatusLine::new(),
            confirm_dialog: None,
            pending_remove: None,
            log_lines: Vec::new(),
            log_scroll: 0,
            log_follow: true,
            log_viewport_height: 0,
            log_view_len: 0,
            log_dedup: DedupMode::default(),
            log_filter: String::new(),
            log_search_open: false,
            describe_scroll: 0,
            describe_viewport_height: 0,
            build_info: format!("Name: {PKG_NAME}\nVersion: {PKG_VERSION}"),
            bg_tx,
            bg_rx,
            log_task: None,
            poll_task: None,
        }
    }

    pub(crate) fn toggle_column_width_mode(&mut self) {
        self.column_width_mode = self.column_width_mode.toggle();
        self.sync_column_width_mode();
        self.set_status(format!("Column width: {}", self.column_width_mode));
    }

    /// Push the session width mode onto every table, including `processes`,
    /// which is recreated on each open.
    pub(crate) const fn sync_column_width_mode(&mut self) {
        let m = self.column_width_mode;
        self.containers.view.column_width_mode = m;
        self.images.view.column_width_mode = m;
        self.volumes.view.column_width_mode = m;
        self.networks.view.column_width_mode = m;
        self.processes.view.column_width_mode = m;
    }

    /// Attempt to connect to the daemon and, on success, start polling.
    pub async fn connect(&mut self) {
        match DockerClient::connect() {
            Ok(client) => match client.ping().await {
                Ok(()) => {
                    self.docker = Some(client);
                    self.state = AppState::List;
                    self.start_polling();
                }
                Err(e) => {
                    self.state = AppState::ConnectError(format!(
                        "Docker unreachable: {e}"
                    ));
                }
            },
            Err(e) => {
                self.state =
                    AppState::ConnectError(format!("Docker unreachable: {e}"));
            }
        }
    }

    fn start_polling(&mut self) {
        let Some(docker) = self.docker.clone() else {
            return;
        };
        if let Some(handle) = self.poll_task.take() {
            handle.abort();
        }
        let tx = self.bg_tx.clone();
        self.poll_task = Some(tokio::spawn(async move {
            loop {
                // Stop polling once the app has quit and dropped its
                // receiver, instead of hammering the Docker daemon forever
                // in the background.
                if tx.is_closed() {
                    return;
                }

                // Run the four list calls concurrently instead of awaiting
                // them one at a time, so a poll cycle costs the slowest
                // single call rather than the sum of all four.
                let (containers, images, volumes, networks) = tokio::join!(
                    docker.list_containers(),
                    docker.list_images(),
                    docker.list_volumes(),
                    docker.list_networks(),
                );

                match containers {
                    Ok(rows) => {
                        let _ = tx.send(BackgroundEvent::Containers(rows));
                    }
                    Err(e) => {
                        if tx
                            .send(BackgroundEvent::PollError(e.to_string()))
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                match images {
                    Ok(rows) => {
                        let _ = tx.send(BackgroundEvent::Images(rows));
                    }
                    Err(e) => {
                        let _ =
                            tx.send(BackgroundEvent::PollError(e.to_string()));
                    }
                }
                match volumes {
                    Ok(rows) => {
                        let _ = tx.send(BackgroundEvent::Volumes(rows));
                    }
                    Err(e) => {
                        let _ =
                            tx.send(BackgroundEvent::PollError(e.to_string()));
                    }
                }
                match networks {
                    Ok(rows) => {
                        let _ = tx.send(BackgroundEvent::Networks(rows));
                    }
                    Err(e) => {
                        let _ =
                            tx.send(BackgroundEvent::PollError(e.to_string()));
                    }
                }
                sleep(POLL_INTERVAL).await;
            }
        }));
    }

    pub(crate) fn selected_container(&self) -> Option<&ContainerRow> {
        let idx = self.containers.view.state.selected()?;
        self.containers.model.items.get(idx)
    }

    pub(crate) fn selected_image(&self) -> Option<&ImageRow> {
        let idx = self.images.view.state.selected()?;
        self.images.model.items.get(idx)
    }

    pub(crate) fn selected_volume(&self) -> Option<&VolumeRow> {
        let idx = self.volumes.view.state.selected()?;
        self.volumes.model.items.get(idx)
    }

    pub(crate) fn selected_network(&self) -> Option<&NetworkRow> {
        let idx = self.networks.view.state.selected()?;
        self.networks.model.items.get(idx)
    }

    pub(crate) const fn quit(&mut self) {
        self.running = false;
    }

    pub(crate) fn set_status<S: Into<String>>(&mut self, message: S) {
        self.status_line.set_message(message);
    }

    /// Run the application's main loop.
    pub async fn run(&mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.running = true;
        self.connect().await;

        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            self.drain_background_events();
            self.poll_terminal_event(&mut terminal).await?;
        }
        Ok(())
    }

    fn drain_background_events(&mut self) {
        while let Ok(event) = self.bg_rx.try_recv() {
            match event {
                BackgroundEvent::Containers(mut rows) => {
                    rows.sort_by_key(|r| r.name.to_lowercase());
                    self.containers_all = rows;
                    let filtered = self.visible_containers();
                    apply_table_update(&mut self.containers, filtered);
                }
                BackgroundEvent::Images(mut rows) => {
                    rows.sort_by_key(|r| r.repo_tags.to_lowercase());
                    self.images_all = rows;
                    let filtered =
                        filter_rows(&self.images_all, &self.list_filter);
                    apply_table_update(&mut self.images, filtered);
                }
                BackgroundEvent::Volumes(mut rows) => {
                    rows.sort_by_key(|r| r.name.to_lowercase());
                    self.volumes_all = rows;
                    let filtered =
                        filter_rows(&self.volumes_all, &self.list_filter);
                    apply_table_update(&mut self.volumes, filtered);
                }
                BackgroundEvent::Networks(mut rows) => {
                    rows.sort_by_key(|r| r.name.to_lowercase());
                    self.networks_all = rows;
                    let filtered =
                        filter_rows(&self.networks_all, &self.list_filter);
                    apply_table_update(&mut self.networks, filtered);
                }
                BackgroundEvent::PollError(e) => {
                    self.set_status(format!("Refresh failed: {e}"));
                }
                BackgroundEvent::Processes(rows) => {
                    apply_table_update(&mut self.processes, rows);
                }
                BackgroundEvent::Status(msg) => self.set_status(msg),
                BackgroundEvent::ProcessError(e) => {
                    self.set_status(format!("Processes unavailable: {e}"));
                }
                BackgroundEvent::LogLine(line) => {
                    const MAX_LOG_LINES: usize = 5000;
                    let parsed = line
                        .as_bytes()
                        .into_text()
                        .unwrap_or_else(|_| Text::raw(line));
                    self.log_lines.extend(parsed.lines);
                    if self.log_lines.len() > MAX_LOG_LINES {
                        let drop = self.log_lines.len() - MAX_LOG_LINES;
                        self.log_lines.drain(..drop);
                        self.log_scroll = self.log_scroll.saturating_sub(drop);
                    }
                }
            }
        }
    }

    /// Poll crossterm for an input event with a short timeout so the loop
    /// keeps spinning to pick up background updates (poller, log tail).
    async fn poll_terminal_event(
        &mut self,
        terminal: &mut DefaultTerminal,
    ) -> Result<()> {
        if !poll(Duration::from_millis(100))? {
            return Ok(());
        }
        match read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.on_key_event(key, terminal).await?;
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

    /// Suspend the TUI, spawn an interactive `docker exec` shell, and resume.
    pub(crate) fn exec_shell<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        id: &str,
    ) -> Result<()> {
        execute!(stdout(), DisableBracketedPaste)?;
        stdout().execute(LeaveAlternateScreen)?;
        disable_raw_mode()?;

        let status = Command::new("docker")
            .args(["exec", "-it", id, "sh"])
            .status();

        stdout().execute(EnterAlternateScreen)?;
        enable_raw_mode()?;
        execute!(stdout(), EnableBracketedPaste)?;
        terminal.clear().map_err(|e| err(e.to_string()))?;

        match status {
            Ok(s) if s.success() => self.set_status("Exec session ended"),
            Ok(s) => self.set_status(format!("Exec exited: {s}")),
            Err(e) => self.set_status(format!("Failed to exec: {e}")),
        }
        Ok(())
    }

    /// Enter the log view and start tailing the container's logs.
    pub(crate) fn open_logs(&mut self, id: &str, name: &str) {
        self.start_logs(id, name, None, None);
    }

    /// Show the output of the process selected in the process view: its log
    /// file if it writes to one, else the container logs when it shares
    /// PID 1's output pipe.
    pub(crate) fn open_process_output(&mut self) {
        let AppState::Processes { id, name } = &self.state else {
            return;
        };
        let (id, name) = (id.clone(), name.clone());
        let back = Some(self.state.clone());
        let rows = &self.processes.model.items;
        let Some(row) = self
            .processes
            .view
            .state
            .selected()
            .and_then(|i| rows.get(i))
            .cloned()
        else {
            return;
        };
        if let Some(path) = row.files.first() {
            let label = format!("{name}: {} {path}", row.pid);
            self.start_logs(&id, &label, Some(row.files.clone()), back);
        } else if row.stdout.starts_with("pipe:")
            && rows.iter().any(|r| r.pid == "1" && r.stdout == row.stdout)
        {
            self.set_status("Shares container output (all processes)");
            self.start_logs(&id, &name, None, back);
        } else {
            self.set_status("No readable output for this process");
        }
    }

    /// Enter the log view tailing the container's logs, or the first
    /// readable of `files` inside it (container logs if none is readable).
    /// `back` is the state `q` returns to (the list when `None`).
    fn start_logs(
        &mut self,
        id: &str,
        name: &str,
        files: Option<Vec<String>>,
        back: Option<AppState>,
    ) {
        let Some(docker) = self.docker.clone() else {
            return;
        };
        self.logs_back = back;
        self.log_lines.clear();
        self.log_scroll = 0;
        self.log_follow = true;
        self.log_filter.clear();
        self.log_search_open = false;
        self.state = AppState::Logs {
            id: id.to_string(),
            name: name.to_string(),
        };
        if let Some(handle) = self.log_task.take() {
            handle.abort();
        }
        let tx = self.bg_tx.clone();
        let id = id.to_string();
        self.log_task = Some(tokio::spawn(async move {
            let (line_tx, mut line_rx) = unbounded_channel::<String>();
            let status_tx = tx.clone();
            let follower = tokio::spawn(async move {
                let mut errors = Vec::new();
                for path in files.iter().flatten() {
                    match docker.probe_file(&id, path).await {
                        Ok(()) => {
                            docker.tail_file(&id, path, line_tx).await;
                            return;
                        }
                        Err(e) => errors.push(e),
                    }
                }
                if !errors.is_empty() {
                    let _ = status_tx.send(BackgroundEvent::Status(format!(
                        "{}; showing container logs",
                        errors.join("; ")
                    )));
                }
                docker.tail_logs(&id, line_tx).await;
            });
            while let Some(line) = line_rx.recv().await {
                if tx.send(BackgroundEvent::LogLine(line)).is_err() {
                    break;
                }
            }
            follower.abort();
        }));
    }

    /// Fetch a cheap daemon-level summary and switch to the info panel.
    pub(crate) async fn open_daemon_health(&mut self) {
        let Some(docker) = self.docker.clone() else {
            return;
        };
        match docker.daemon_health().await {
            Ok(health) => self.state = AppState::Info(health),
            Err(e) => self.set_status(format!("Daemon info failed: {e}")),
        }
    }

    /// Enter the process view and poll `docker top` while it is open.
    pub(crate) fn open_processes(&mut self, id: &str, name: &str) {
        let Some(docker) = self.docker.clone() else {
            return;
        };
        self.processes = TableDataState::new(Vec::new());
        self.state = AppState::Processes {
            id: id.to_string(),
            name: name.to_string(),
        };
        if let Some(handle) = self.process_task.take() {
            handle.abort();
        }
        let tx = self.bg_tx.clone();
        let id = id.to_string();
        self.process_task = Some(tokio::spawn(async move {
            loop {
                let event = match docker.processes(&id).await {
                    Ok(rows) => BackgroundEvent::Processes(rows),
                    Err(e) => BackgroundEvent::ProcessError(e.to_string()),
                };
                if tx.send(event).is_err() {
                    break;
                }
                sleep(POLL_INTERVAL).await;
            }
        }));
    }

    /// Leave the process view, stopping the poller.
    pub(crate) fn close_processes(&mut self) {
        if let Some(handle) = self.process_task.take() {
            handle.abort();
        }
        self.state = AppState::List;
    }

    /// Leave the log view, stopping the tail task.
    pub(crate) fn close_logs(&mut self) {
        if let Some(handle) = self.log_task.take() {
            handle.abort();
        }
        self.state = self.logs_back.take().unwrap_or(AppState::List);
    }

    /// Containers matching the `/` filter and, when on, running-only.
    fn visible_containers(&self) -> Vec<ContainerRow> {
        let mut rows = filter_rows(&self.containers_all, &self.list_filter);
        if self.running_only {
            rows.retain(ContainerRow::is_running);
        }
        rows
    }

    /// Re-apply the `/` filter to all four resource lists from their
    /// unfiltered masters, e.g. after the filter text changes.
    pub(crate) fn reapply_list_filter(&mut self) {
        let filtered = self.visible_containers();
        apply_table_update(&mut self.containers, filtered);
        let filtered = filter_rows(&self.images_all, &self.list_filter);
        apply_table_update(&mut self.images, filtered);
        let filtered = filter_rows(&self.volumes_all, &self.list_filter);
        apply_table_update(&mut self.volumes, filtered);
        let filtered = filter_rows(&self.networks_all, &self.list_filter);
        apply_table_update(&mut self.networks, filtered);
    }

    /// Fetch and show full inspect details for a resource of any kind.
    pub(crate) async fn open_describe(
        &mut self,
        kind: ResourceKind,
        id: &str,
        name: &str,
    ) {
        use crate::docker::{
            describe_image_text, describe_network_text, describe_text,
            describe_volume_text,
        };
        let Some(docker) = self.docker.clone() else {
            return;
        };
        let text = match kind {
            ResourceKind::Containers => {
                docker.inspect(id).await.map(|i| describe_text(&i))
            }
            ResourceKind::Images => docker
                .inspect_image(id)
                .await
                .map(|i| describe_image_text(&i)),
            ResourceKind::Volumes => docker
                .inspect_volume(id)
                .await
                .map(|v| describe_volume_text(&v)),
            ResourceKind::Networks => docker
                .inspect_network(id)
                .await
                .map(|n| describe_network_text(&n)),
        };
        match text {
            Ok(text) => {
                self.describe_scroll = 0;
                self.state = AppState::Describe {
                    name: name.to_string(),
                    text,
                };
            }
            Err(e) => self.set_status(format!("Inspect failed: {e}")),
        }
    }

    /// Leave the describe view.
    pub(crate) fn close_describe(&mut self) {
        self.state = AppState::List;
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

/// Merge a freshly polled row list into a table, preserving the current
/// selection where possible instead of resetting it to row 0.
fn apply_table_update<T: TableData + Clone>(
    table: &mut TableDataState<T>,
    rows: Vec<T>,
) {
    let selected = table.view.state.selected();
    table.model.longest_item_lens = constraint_len_calculator(&rows);
    table.model.items = rows;

    let len = table.model.items.len();
    match selected {
        Some(sel) if sel >= len => {
            table.view.state.select(if len == 0 {
                None
            } else {
                Some(len - 1)
            });
        }
        None if len > 0 => table.view.state.select(Some(0)),
        _ => {}
    }
}
