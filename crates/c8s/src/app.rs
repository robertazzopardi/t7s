use core::{future::Future, mem, time::Duration};
use std::{
    collections::HashMap,
    io::stdout,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

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
        client::{DaemonHealth, DockerClient},
    },
    error::{Result, err},
    in_flight::{DAEMON_KEY, InFlight},
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

/// Current UNIX time in seconds, as Docker's log `since` filter takes it.
fn unix_now() -> i32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i32::try_from(d.as_secs()).unwrap_or(i32::MAX))
}

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
    /// Container `id` restarted and its log stream reattached from UNIX
    /// time `since`.
    LogRestarted {
        id: String,
        since: i32,
    },
    /// Process list refreshed by the active process-view poller.
    Processes(Vec<ProcessRow>),
    /// `docker top` failed (e.g. the container is not running).
    ProcessError(String),
    /// Message for the status line from a background task.
    Status(String),
    /// Lifecycle action finished (start/stop/restart/remove, etc.).
    ActionDone {
        key: String,
        message: String,
    },
    ActionFailed {
        key: String,
        message: String,
    },
    /// Inspect finished; switch to the describe view.
    DescribeReady {
        key: String,
        name: String,
        text: String,
    },
    /// Daemon connect succeeded.
    Connected(DockerClient),
    ConnectFailed(String),
    DaemonHealthOk(DaemonHealth),
}

/// `tokio::spawn` for work holding an in-flight `key`: if the task panics, an
/// `ActionFailed` still releases the key so the resource is not left busy.
pub fn spawn_keyed<F>(
    tx: UnboundedSender<BackgroundEvent>,
    key: String,
    task: F,
) where
    F: Future<Output = ()> + Send + 'static,
{
    let handle = tokio::spawn(task);
    tokio::spawn(async move {
        if handle.await.is_err() {
            let _ = tx.send(BackgroundEvent::ActionFailed {
                key,
                message: "Background task failed unexpectedly".into(),
            });
        }
    });
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
    /// Files the open log view was asked to tail, kept for `H` reloads.
    pub(crate) log_files: Option<Vec<String>>,
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
    /// Per container id: when its current run's logs start (UNIX time) and
    /// the lines of its run before that restart. Kept for the app lifetime.
    pub(crate) log_prev_runs: HashMap<String, (i32, Vec<Line<'static>>)>,
    /// Container id whose previous run `p` shows; `None` for file tails.
    pub(crate) log_prev_key: Option<String>,
    /// True while the log view shows the previous run.
    pub(crate) log_show_prev: bool,
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
    pub(crate) in_flight: InFlight,

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
            log_files: None,
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
            log_prev_runs: HashMap::new(),
            log_prev_key: None,
            log_show_prev: false,
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
            in_flight: InFlight::default(),
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
    pub fn connect(&mut self) {
        if !self.in_flight.try_begin(DAEMON_KEY) {
            self.set_status("Already connecting…");
            return;
        }
        self.state = AppState::Connecting;
        let tx = self.bg_tx.clone();
        spawn_keyed(tx.clone(), DAEMON_KEY.into(), async move {
            let result = match DockerClient::connect() {
                Ok(client) => match client.ping().await {
                    Ok(()) => BackgroundEvent::Connected(client),
                    Err(e) => BackgroundEvent::ConnectFailed(format!(
                        "Docker unreachable: {e}"
                    )),
                },
                Err(e) => BackgroundEvent::ConnectFailed(format!(
                    "Docker unreachable: {e}"
                )),
            };
            let _ = tx.send(result);
        });
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
    pub fn run(&mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.running = true;
        self.connect();

        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            self.drain_background_events();
            self.poll_terminal_event(&mut terminal)?;
        }
        Ok(())
    }

    pub(crate) fn drain_background_events(&mut self) {
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
                BackgroundEvent::LogRestarted { id, since } => {
                    if self.log_prev_key.as_deref() != Some(id.as_str()) {
                        continue;
                    }
                    let lines = mem::take(&mut self.log_lines);
                    self.log_prev_runs.insert(id, (since, lines));
                    self.log_show_prev = false;
                    self.log_scroll = 0;
                    self.log_follow = true;
                    self.set_status(
                        "Container restarted; p shows the previous run",
                    );
                }
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
                        if !self.log_show_prev {
                            self.log_scroll =
                                self.log_scroll.saturating_sub(drop);
                        }
                    }
                }
                BackgroundEvent::ActionDone { key, message }
                | BackgroundEvent::ActionFailed { key, message } => {
                    self.in_flight.end(&key);
                    self.set_status(message);
                }
                BackgroundEvent::DescribeReady { key, name, text } => {
                    self.in_flight.end(&key);
                    // Requested from the list; the user may have opened logs etc. since.
                    if self.state == AppState::List {
                        self.describe_scroll = 0;
                        self.state = AppState::Describe { name, text };
                    }
                }
                BackgroundEvent::Connected(client) => {
                    self.in_flight.end(DAEMON_KEY);
                    self.docker = Some(client);
                    self.state = AppState::List;
                    self.start_polling();
                    self.set_status("Connected to Docker");
                }
                BackgroundEvent::ConnectFailed(msg) => {
                    self.in_flight.end(DAEMON_KEY);
                    self.state = AppState::ConnectError(msg);
                }
                BackgroundEvent::DaemonHealthOk(health) => {
                    self.in_flight.end(DAEMON_KEY);
                    if self.state == AppState::List {
                        self.state = AppState::Info(health);
                    }
                }
            }
        }
    }

    /// Poll crossterm for an input event with a short timeout so the loop
    /// keeps spinning to pick up background updates (poller, log tail).
    fn poll_terminal_event(
        &mut self,
        terminal: &mut DefaultTerminal,
    ) -> Result<()> {
        if !poll(Duration::from_millis(100))? {
            return Ok(());
        }
        match read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                self.on_key_event(key, terminal)?;
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
        self.start_logs(id, name, None, None, false);
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
            self.start_logs(&id, &label, Some(row.files.clone()), back, false);
        } else if row.stdout.starts_with("pipe:")
            && rows.iter().any(|r| r.pid == "1" && r.stdout == row.stdout)
        {
            self.set_status("Shares container output (all processes)");
            self.start_logs(&id, &name, None, back, false);
        } else {
            self.set_status("No readable output for this process");
        }
    }

    /// Enter the log view tailing the container's logs, or the first
    /// readable of `files` inside it (container logs if none is readable).
    /// `back` is the state `q` returns to (the list when `None`). `full`
    /// loads the whole history instead of the last 200 lines.
    fn start_logs(
        &mut self,
        id: &str,
        name: &str,
        files: Option<Vec<String>>,
        back: Option<AppState>,
        full: bool,
    ) {
        let Some(docker) = self.docker.clone() else {
            return;
        };
        self.logs_back = back;
        self.log_files.clone_from(&files);
        self.log_lines.clear();
        self.log_prev_key = files.is_none().then(|| id.to_string());
        self.log_show_prev = false;
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
        // After a seen restart, open on the current run only (`p` has the rest).
        let since = match self.log_prev_runs.get(&id) {
            Some((since, _)) if !full && files.is_none() => *since,
            _ => 0,
        };
        let (tail, file_lines) = if full || since > 0 {
            ("all", "+1")
        } else {
            ("200", "200")
        };
        self.log_task = Some(tokio::spawn(async move {
            let (line_tx, mut line_rx) = unbounded_channel::<String>();
            let status_tx = tx.clone();
            let follower = tokio::spawn(async move {
                let mut errors = Vec::new();
                for path in files.iter().flatten() {
                    match docker.probe_file(&id, path).await {
                        Ok(()) => {
                            docker
                                .tail_file(&id, path, file_lines, line_tx)
                                .await;
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
                // The stream ends when the container stops; reattach once it
                // runs again (e.g. restarted outside c8s), skipping old lines.
                let mut since = since;
                let mut tail = tail;
                loop {
                    docker.tail_logs(&id, since, tail, &line_tx).await;
                    since = unix_now();
                    tail = "all";
                    let _ = status_tx.send(BackgroundEvent::Status(
                        "Container stopped; showing saved logs, waiting for \
                         restart…"
                            .into(),
                    ));
                    loop {
                        if line_tx.is_closed() {
                            return;
                        }
                        sleep(POLL_INTERVAL).await;
                        match docker.is_running(&id).await {
                            Ok(true) => break,
                            Ok(false) => {}
                            Err(e) => {
                                let _ =
                                    status_tx.send(BackgroundEvent::Status(
                                        format!("Log stream ended: {e}"),
                                    ));
                                return;
                            }
                        }
                    }
                    let _ = status_tx.send(BackgroundEvent::LogRestarted {
                        id: id.clone(),
                        since,
                    });
                }
            });
            while let Some(line) = line_rx.recv().await {
                if tx.send(BackgroundEvent::LogLine(line)).is_err() {
                    break;
                }
            }
            follower.abort();
        }));
    }

    /// Lines of the open container's run before its last seen restart.
    pub(crate) fn log_prev(&self) -> Option<&Vec<Line<'static>>> {
        let key = self.log_prev_key.as_ref()?;
        self.log_prev_runs.get(key).map(|(_, lines)| lines)
    }

    /// Switch the log view between the current and previous run (`p`).
    pub(crate) fn toggle_previous_logs(&mut self) {
        if self.log_prev().is_none() {
            self.set_status("No previous run since the logs were opened");
            return;
        }
        self.log_show_prev = !self.log_show_prev;
        self.log_scroll = 0;
        self.log_follow = true;
    }

    /// Reload the open log view with its whole history (`H`).
    pub(crate) fn load_full_logs(&mut self) {
        let AppState::Logs { id, name } = self.state.clone() else {
            return;
        };
        let back = self.logs_back.clone();
        let files = self.log_files.clone();
        self.start_logs(&id, &name, files, back, true);
        self.set_status("Loading full log history…");
    }

    /// Fetch a cheap daemon-level summary and switch to the info panel.
    pub(crate) fn open_daemon_health(&mut self) {
        use DAEMON_KEY;

        let key = DAEMON_KEY;
        if !self.in_flight.try_begin(key) {
            self.set_status("Daemon busy…");
            return;
        }
        let Some(docker) = self.docker.clone() else {
            self.in_flight.end(key);
            return;
        };
        self.set_status("Loading daemon info…");
        let tx = self.bg_tx.clone();
        spawn_keyed(tx.clone(), key.into(), async move {
            let event = match docker.daemon_health().await {
                Ok(h) => BackgroundEvent::DaemonHealthOk(h),
                Err(e) => BackgroundEvent::ActionFailed {
                    key: key.to_string(),
                    message: format!("Daemon info failed: {e}"),
                },
            };
            let _ = tx.send(event);
        });
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
    pub(crate) fn open_describe(
        &mut self,
        kind: ResourceKind,
        id: &str,
        name: &str,
    ) {
        use crate::{
            app::BackgroundEvent,
            docker::{
                describe_image_text, describe_network_text, describe_text,
                describe_volume_text,
            },
            in_flight::resource_key,
        };

        let key = resource_key(kind, id);
        if !self.in_flight.try_begin(key.clone()) {
            self.set_status("Already busy with this resource");
            return;
        }
        let Some(docker) = self.docker.clone() else {
            self.in_flight.end(&key);
            return;
        };
        let tx = self.bg_tx.clone();
        let id = id.to_string();
        let name = name.to_string();
        spawn_keyed(tx.clone(), key.clone(), async move {
            let text = match kind {
                ResourceKind::Containers => {
                    docker.inspect(&id).await.map(|i| describe_text(&i))
                }
                ResourceKind::Images => docker
                    .inspect_image(&id)
                    .await
                    .map(|i| describe_image_text(&i)),
                ResourceKind::Volumes => docker
                    .inspect_volume(&id)
                    .await
                    .map(|v| describe_volume_text(&v)),
                ResourceKind::Networks => docker
                    .inspect_network(&id)
                    .await
                    .map(|n| describe_network_text(&n)),
            };
            let event = match text {
                Ok(text) => BackgroundEvent::DescribeReady { key, name, text },
                Err(e) => BackgroundEvent::ActionFailed {
                    key,
                    message: format!("Inspect failed: {e}"),
                },
            };
            let _ = tx.send(event);
        });
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

#[cfg(test)]
mod tests {
    use tokio::time::sleep;

    use super::*;
    use crate::{app_state::AppState, in_flight::DAEMON_KEY};

    #[tokio::test]
    async fn connect_is_non_blocking_and_locks_daemon_key() {
        let mut app = App::new();
        app.connect();
        assert!(app.in_flight.contains(DAEMON_KEY));
        assert_eq!(app.state, AppState::Connecting);
    }

    #[test]
    fn late_describe_does_not_replace_another_view() {
        let mut app = App::new();
        app.state = AppState::Connecting;
        assert!(app.in_flight.try_begin("d"));
        app.bg_tx
            .send(BackgroundEvent::DescribeReady {
                key: "d".into(),
                name: "web".into(),
                text: "{}".into(),
            })
            .unwrap();
        app.drain_background_events();
        assert!(!app.in_flight.contains("d"));
        assert_eq!(app.state, AppState::Connecting);

        app.state = AppState::List;
        assert!(app.in_flight.try_begin("d"));
        app.bg_tx
            .send(BackgroundEvent::DescribeReady {
                key: "d".into(),
                name: "web".into(),
                text: "{}".into(),
            })
            .unwrap();
        app.drain_background_events();
        assert!(matches!(app.state, AppState::Describe { .. }));
    }

    #[tokio::test]
    async fn spawn_keyed_releases_key_when_task_panics() {
        let mut app = App::new();
        assert!(app.in_flight.try_begin("k"));
        spawn_keyed(app.bg_tx.clone(), "k".into(), async { panic!("boom") });
        for _ in 0..1000 {
            app.drain_background_events();
            if !app.in_flight.contains("k") {
                return;
            }
            sleep(Duration::from_millis(1)).await;
        }
        panic!("key still in flight");
    }
}
