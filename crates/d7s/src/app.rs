use core::time::Duration;
use std::{
    env::{current_dir, temp_dir, var},
    fs::{read_to_string, write},
    io::stdout,
    path::Path,
    process::{Command, id},
    sync::Arc,
    time::Instant,
};

use crossterm::{
    ExecutableCommand, clipboard,
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use k9tui::widgets::{
    hotkey::Hotkey,
    status_line::StatusLine,
    table::{ColumnWidthMode, TableData, TableDataState},
};
use ratatui::DefaultTerminal;
use ratatui_textarea::TextArea;
use tokio::sync::mpsc::{
    UnboundedReceiver, UnboundedSender, unbounded_channel,
};

use crate::{
    app,
    app_state::{AppState, DatabaseExplorerState},
    bg::{BgEvent, DescribeTarget, SqlExecuteOk},
    cell_edit::CellEditSession,
    database_explorer::STATUS_CONNECT_FAILED,
    database_explorer_state::DatabaseExplorer,
    db::{
        Column, Database, RowDeleteSpec, Table,
        connection::{Connection, Environment, shorten_home_path},
        query_log::{QueryLog, QueryLogEntry},
        sqlite::init_db,
    },
    error::Result,
    filtered_data::FilteredData,
    in_flight::{InFlight, describe_key, key_connection, sql_key},
    services::{ConnectionService, PasswordService, PreferencesService},
    sql::safety::{StatementSafety, classify_statement, split_statements},
    ui::widgets::{
        connection_modal::ModalManager, describe_content::DescribeRow,
        help_content::HelpRow, hotkeys::CONNECTION_HOTKEYS,
    },
    virtual_table::VIRTUAL_TABLE_PAGE_SIZE,
};

pub const APP_NAME: &str = r"_________________
\______ \______  \______
 |    |  \  /    /  ___/
 |    `   \/    /\___  \
/_______  /____//____  /
        \/           \/
";

// Build metadata
pub const PKG_NAME: &str = env!("CARGO_PKG_NAME");
pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Re-run interval for SQL "watch" mode (`w` on SQL results), matching
/// c8s's container-list poll cadence.
const WATCH_INTERVAL: Duration = Duration::from_secs(2);

/// The main application which holds the state and logic of the application.
#[allow(clippy::struct_excessive_bools)] // session flags; not worth a state machine
pub struct App<'a> {
    /// Is the application running?
    pub(crate) running: bool,
    pub(crate) modal_manager: ModalManager,
    pub(crate) hotkeys: Vec<Hotkey>,
    /// Current application state
    pub(crate) state: AppState,
    /// Database explorer state (when connected to a database)
    pub(crate) database_explorer: DatabaseExplorer,
    /// Search filter widget
    pub(crate) search_filter: Option<TextArea<'a>>,
    /// When true, `search_filter` is the `:` command bar, not a row filter.
    pub(crate) command_mode: bool,
    /// Status line widget
    pub(crate) status_line: StatusLine,
    /// Password management service
    pub(crate) password_service: PasswordService,
    /// Build info
    pub(crate) build_info: String,
    /// Signal to the run loop to open the external editor
    pub(crate) open_editor_requested: bool,
    /// Table data: after `d`, row locators awaiting delete confirmation.
    pub(crate) pending_row_deletes: Option<Vec<RowDeleteSpec>>,
    /// k9s-style help panel in main content area (`?` toggles).
    pub(crate) show_help: bool,
    pub(crate) help_table: TableDataState<HelpRow>,
    /// k9s-style describe panel for the currently selected object (`i` toggles).
    pub(crate) show_describe: bool,
    pub(crate) describe_table: TableDataState<DescribeRow>,
    /// Rows per virtual table page (from prefs / `D7S_PAGE_SIZE`).
    pub(crate) page_size: u32,
    /// First Esc warns before dropping draft rows; second Esc discards.
    pub(crate) draft_discard_pending: bool,
    /// Shift+N without cached column meta: create defaults draft when meta loads.
    pub(crate) pending_defaults_draft: bool,
    /// One-shot hint after first connect in a session.
    pub(crate) showed_help_hint: bool,
    /// SQL results: when true, the current statement re-runs on
    /// [`WATCH_INTERVAL`] until toggled off or the view changes.
    pub(crate) watch_active: bool,
    pub(crate) watch_last_tick: Instant,
    /// Every query d7s ran this session (bounded ring buffer).
    pub(crate) query_log: QueryLog,
    /// `L`: query log view in the main content area.
    pub(crate) show_query_log: bool,
    pub(crate) query_log_table: TableDataState<QueryLogEntry>,
    pub(crate) cell_edit: Option<CellEditSession>,
    pub(crate) in_flight: InFlight,
    pub(crate) bg_tx: UnboundedSender<BgEvent>,
    pub(crate) bg_rx: UnboundedReceiver<BgEvent>,
    /// `W`: session-wide column layout (Fit / Content) for every data table.
    pub(crate) column_width_mode: ColumnWidthMode,
}

impl Default for App<'_> {
    fn default() -> Self {
        let (bg_tx, bg_rx) = unbounded_channel();
        Self {
            running: false,
            modal_manager: ModalManager::new(),
            hotkeys: CONNECTION_HOTKEYS.to_vec(),
            state: AppState::ConnectionList,
            database_explorer: DatabaseExplorer::default(),
            search_filter: None,
            command_mode: false,
            status_line: StatusLine::new(),
            password_service: PasswordService::new(),
            build_info: String::new(),
            open_editor_requested: false,
            pending_row_deletes: None,
            show_help: false,
            help_table: TableDataState::new(Vec::new()),
            show_describe: false,
            describe_table: TableDataState::new(Vec::new()),
            page_size: VIRTUAL_TABLE_PAGE_SIZE,
            draft_discard_pending: false,
            pending_defaults_draft: false,
            showed_help_hint: false,
            watch_active: false,
            watch_last_tick: Instant::now(),
            query_log: QueryLog::default(),
            show_query_log: false,
            query_log_table: TableDataState::new(Vec::new()),
            cell_edit: None,
            in_flight: InFlight::default(),
            bg_tx,
            bg_rx,
            column_width_mode: ColumnWidthMode::Fit,
        }
    }
}

impl App<'_> {
    pub(crate) fn toggle_column_width_mode(&mut self) {
        self.column_width_mode = self.column_width_mode.toggle();
        self.sync_column_width_mode();
        self.set_status(format!("Column width: {}", self.column_width_mode));
    }

    /// Push the session width mode onto every table the app owns, so tables
    /// created later (new query, new drill-down) inherit it.
    pub(crate) const fn sync_column_width_mode(&mut self) {
        let m = self.column_width_mode;
        self.help_table.view.column_width_mode = m;
        self.describe_table.view.column_width_mode = m;
        self.query_log_table.view.column_width_mode = m;
        let e = &mut self.database_explorer;
        e.connections.table.view.column_width_mode = m;
        if let Some(t) = &mut e.databases {
            t.table.view.column_width_mode = m;
        }
        if let Some(t) = &mut e.schemas {
            t.table.view.column_width_mode = m;
        }
        if let Some(t) = &mut e.tables {
            t.table.view.column_width_mode = m;
        }
        if let Some(t) = &mut e.columns {
            t.table.view.column_width_mode = m;
        }
        if let Some(t) = &mut e.table_data {
            t.table.view.column_width_mode = m;
        }
        e.sql_executor.table_state.view.column_width_mode = m;
    }

    /// Post initilisation for the App
    pub fn init(mut self) -> Result<Self> {
        init_db()?;

        let items = ConnectionService::get_all().unwrap_or_default();
        self.database_explorer.connections = FilteredData::new(items);
        self.database_explorer.recent_tables =
            PreferencesService::load_recent_tables();
        self.page_size = PreferencesService::effective_page_size();

        self.build_info = build_info()?;

        Ok(self)
    }

    /// Run the application's main loop.
    #[allow(clippy::future_not_send)]
    pub fn run(&mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.running = true;
        self.ping_connections_if_any();
        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            self.drain_bg_events();
            self.tick_watch();
            self.handle_crossterm_events()?;

            self.handle_external_terminal(&mut terminal)?;
        }
        self.save_session_preferences();
        Ok(())
    }

    /// `false` for a result keyed to a connection other than the active one
    /// (the user switched away while it was loading); such results are dropped.
    fn is_active_key(&self, key: &str) -> bool {
        key_connection(key)
            .is_none_or(|c| c == self.database_explorer.connection.name)
    }

    #[allow(clippy::too_many_lines)] // flat background-event dispatch match
    pub(crate) fn drain_bg_events(&mut self) {
        while let Ok(event) = self.bg_rx.try_recv() {
            match event {
                BgEvent::ActionDone {
                    key,
                    message,
                    reload_table,
                } => {
                    self.in_flight.end(&key);
                    self.set_status(message);
                    if let Some(r) = reload_table {
                        let on_table = matches!(
                            &self.database_explorer.state,
                            DatabaseExplorerState::TableData(s, t)
                                if *s == r.schema && *t == r.table
                        );
                        if on_table && self.is_active_key(&key) {
                            self.reload_current_table_data(r.success_status);
                        }
                    }
                }
                BgEvent::ActionFailed { key, message } => {
                    self.in_flight.end(&key);
                    self.set_status(message);
                }
                BgEvent::SqlFinished {
                    key,
                    sql,
                    record_history,
                    result,
                } => {
                    self.in_flight.end(&key);
                    let still_sql_view =
                        sql_key(&self.database_explorer.connection.name) == key
                            && matches!(
                                self.database_explorer.state,
                                DatabaseExplorerState::SqlResults(_)
                            );
                    match result {
                        Ok(SqlExecuteOk { cols, rows }) => {
                            if still_sql_view {
                                self.database_explorer
                                    .sql_executor
                                    .set_results(rows, &cols);
                            }
                            if record_history {
                                let _ =
                                    PreferencesService::push_sql_history(&sql);
                            }
                        }
                        Err(message) => {
                            if message
                                == "Query executed successfully but returned no data"
                            {
                                self.set_status(message);
                            } else {
                                self.set_status(format!(
                                    "SQL error: {message}"
                                ));
                            }
                        }
                    }
                }
                BgEvent::DatabasesLoaded { key, databases } => {
                    self.in_flight.end(&key);
                    if self.is_active_key(&key) {
                        self.apply_databases_loaded(databases);
                    }
                }
                BgEvent::SchemasLoaded { key, schemas } => {
                    self.in_flight.end(&key);
                    if self.is_active_key(&key) {
                        self.apply_schemas_loaded(schemas);
                    }
                }
                BgEvent::TablesLoaded {
                    key,
                    schema,
                    tables,
                } => {
                    self.in_flight.end(&key);
                    if self.is_active_key(&key) {
                        self.apply_tables_loaded(&schema, tables);
                    }
                }
                BgEvent::ColumnsLoaded {
                    key,
                    schema,
                    table,
                    columns,
                } => {
                    self.in_flight.end(&key);
                    if self.is_active_key(&key) {
                        self.apply_columns_loaded(&schema, &table, columns);
                    }
                }
                BgEvent::TableColumnMetaLoaded {
                    key,
                    schema,
                    table,
                    result,
                } => {
                    self.in_flight.end(&key);
                    if self.is_active_key(&key) {
                        self.apply_table_column_meta_loaded(
                            &schema, &table, result,
                        );
                    }
                }
                BgEvent::TablePageLoaded {
                    key,
                    schema,
                    table,
                    record_recent,
                    result,
                } => {
                    self.in_flight.end(&key);
                    if self.is_active_key(&key) {
                        self.apply_table_page_loaded(
                            &schema,
                            &table,
                            record_recent,
                            result,
                        );
                    }
                }
                BgEvent::ExplorerLoadFailed {
                    key,
                    resource,
                    schema,
                    table,
                    message,
                } => {
                    self.in_flight.end(&key);
                    if self.is_active_key(&key) {
                        self.apply_explorer_load_failed(
                            &resource,
                            schema.as_deref(),
                            table.as_deref(),
                            &message,
                        );
                    }
                }
                BgEvent::Connected {
                    key,
                    connection,
                    database,
                } => {
                    self.in_flight.end(&key);
                    let from_password_modal = self
                        .modal_manager
                        .password_connection()
                        .is_some_and(|c| c.name == connection.name);
                    let modal_password = from_password_modal
                        .then(|| {
                            self.modal_manager
                                .get_password_modal_mut()
                                .map(|m| m.input_value())
                        })
                        .flatten();
                    let conn_for_session = connection.clone();
                    self.apply_connected(connection, database);
                    if let Some(password) = modal_password {
                        self.password_service.store_session_password(
                            &conn_for_session,
                            password,
                        );
                        if let Some(password_modal) =
                            self.modal_manager.get_password_modal_mut()
                        {
                            password_modal.close();
                        }
                    }
                }
                BgEvent::ConnectFailed { key, message } => {
                    self.in_flight.end(&key);
                    self.set_status(message);
                    if let Some(modal_conn) =
                        self.modal_manager.password_connection().cloned()
                    {
                        self.password_service
                            .remove_session_password(&modal_conn);
                        if let Some(password_modal) =
                            self.modal_manager.get_password_modal_mut()
                        {
                            password_modal.clear_input();
                        }
                    }
                }
                BgEvent::ConnectPasswordRequired {
                    key,
                    connection,
                    prompt,
                } => {
                    self.in_flight.end(&key);
                    self.modal_manager.open_password_modal(connection, prompt);
                }
                BgEvent::ConnectionStatus { key, name, status } => {
                    self.in_flight.end(&key);
                    self.apply_connection_status(&name, status);
                }
                BgEvent::DatabaseSelected {
                    key,
                    database_name,
                    database,
                } => {
                    self.in_flight.end(&key);
                    if self.is_active_key(&key) {
                        self.apply_database_selected(&database_name, database);
                    }
                }
                BgEvent::DatabaseSelectFailed { key } => {
                    self.in_flight.end(&key);
                    self.set_status(STATUS_CONNECT_FAILED);
                }
                BgEvent::DescribeRowsLoaded { key, target, rows } => {
                    self.in_flight.end(&key);
                    self.apply_describe_rows_loaded(&target, rows);
                }
                BgEvent::ConnectionTestResult {
                    key,
                    success,
                    message,
                } => {
                    self.in_flight.end(&key);
                    self.apply_connection_test_result(success, message);
                }
            }
        }
    }

    /// Re-run the SQL results query if watch mode is on and the interval
    /// has elapsed. Turns itself off if the view has moved away from
    /// SQL results (e.g. the user pressed Esc back to the connection tree).
    fn tick_watch(&mut self) {
        if !self.watch_active {
            return;
        }
        if !matches!(
            self.database_explorer.state,
            DatabaseExplorerState::SqlResults(_)
        ) {
            self.watch_active = false;
            return;
        }
        if self.watch_last_tick.elapsed() < WATCH_INTERVAL {
            return;
        }
        self.watch_last_tick = Instant::now();
        self.execute_sql_query_watch_tick();
    }

    /// Toggle SQL-results watch mode (`w`).
    pub(crate) fn toggle_watch(&mut self) {
        self.watch_active = !self.watch_active;
        if self.watch_active {
            self.watch_last_tick = Instant::now();
            self.set_status(format!(
                "Watching (every {}s) — press w to stop",
                WATCH_INTERVAL.as_secs()
            ));
        } else {
            self.set_status("Watch stopped");
        }
    }

    fn save_session_preferences(&self) {
        let _ = PreferencesService::save_recent_tables(
            &self.database_explorer.recent_tables,
        );
        let sql = self.database_explorer.sql_executor.sql_input();
        if !sql.trim().is_empty() {
            let _ = PreferencesService::push_sql_history(&sql);
        }
        if self.state == AppState::DatabaseConnected {
            let _ = PreferencesService::set_last_connection(
                &self.database_explorer.connection.name,
            );
        }
        let _ = PreferencesService::set_page_size(self.page_size);
    }

    fn handle_external_terminal(
        &mut self,
        terminal: &mut DefaultTerminal,
    ) -> Result<()> {
        if self.open_editor_requested {
            self.open_editor_requested = false;
            let new_sql = if var("D7S_DEMO").is_ok() {
                var("D7S_DEMO_SQL")
                    .ok()
                    .and_then(|path| read_to_string(path).ok())
                    .unwrap_or_default()
            } else {
                let temp_path =
                    temp_dir().join(format!("d7s_sql_{}.sql", id()));
                let current_sql =
                    self.database_explorer.sql_executor.sql_input();
                write(&temp_path, &current_sql)?;
                Self::run_editor(terminal, &temp_path)?;
                read_to_string(&temp_path).unwrap_or_default()
            };
            self.apply_editor_sql(&new_sql);
        }

        Ok(())
    }

    fn apply_editor_sql(&mut self, new_sql: &str) {
        let new_sql = new_sql.trim_end_matches('\n');
        if new_sql.is_empty() {
            return;
        }
        self.database_explorer.sql_executor.set_sql(new_sql);
        let statements = split_statements(new_sql);
        if statements.is_empty() {
            self.set_status("No SQL statements found in editor file.");
            return;
        }

        if statements.len() == 1 {
            if let Some(statement) = statements.first() {
                self.prepare_sql_statement_execution(statement.text.clone());
            }
        } else {
            let options =
                statements.into_iter().map(|s| s.text).collect::<Vec<_>>();
            self.modal_manager.open_sql_query_selection_modal(options);
        }
    }

    /// Reload saved connections from disk and re-probe health.
    /// Status is runtime-only, so a reload without a ping would blank the column.
    pub(crate) fn refresh_connections(&mut self) {
        if let Ok(connections) = ConnectionService::get_all() {
            self.database_explorer.connections = FilteredData::new(connections);
            // Reapply filter if one is active
            if let Some(search_filter) = &self.search_filter
                && let Some(line) = search_filter.lines().first()
                && !line.is_empty()
            {
                self.apply_filter();
            }
        }
        self.ping_connections_if_any();
    }

    /// Copy the value under the cursor to the clipboard
    pub(crate) fn copy(&mut self) {
        let explorer = &self.database_explorer;
        let value: Option<String> = (|| -> Option<String> {
            let v = match &explorer.state {
                DatabaseExplorerState::Connections => {
                    let view = &explorer.connections.table.view;
                    let selected = view.state.selected()?;
                    let col = view.state.selected_column().unwrap_or(0);
                    explorer
                        .connections
                        .table
                        .model
                        .items
                        .get(selected)?
                        .col(col)
                }
                DatabaseExplorerState::Databases => {
                    let dbs = explorer.databases.as_ref()?;
                    let selected = dbs.table.view.state.selected()?;
                    let col =
                        dbs.table.view.state.selected_column().unwrap_or(0);
                    dbs.table.model.items.get(selected)?.col(col)
                }
                DatabaseExplorerState::Schemas => {
                    let schemas = explorer.schemas.as_ref()?;
                    let selected = schemas.table.view.state.selected()?;
                    let col =
                        schemas.table.view.state.selected_column().unwrap_or(0);
                    schemas.table.model.items.get(selected)?.col(col)
                }
                DatabaseExplorerState::Tables(_) => {
                    let tables = explorer.tables.as_ref()?;
                    let selected = tables.table.view.state.selected()?;
                    let col =
                        tables.table.view.state.selected_column().unwrap_or(0);
                    tables.table.model.items.get(selected)?.col(col)
                }
                DatabaseExplorerState::Columns(_, _) => {
                    let columns = explorer.columns.as_ref()?;
                    let selected = columns.table.view.state.selected()?;
                    let col =
                        columns.table.view.state.selected_column().unwrap_or(0);
                    columns.table.model.items.get(selected)?.col(col)
                }
                DatabaseExplorerState::TableData(_, _) => {
                    let table_data = explorer.table_data.as_ref()?;
                    let selected_row =
                        table_data.table.view.state.selected()?;
                    let selected_col = table_data
                        .table
                        .view
                        .state
                        .selected_column()
                        .unwrap_or(0);
                    let row = table_data.table.model.items.get(selected_row)?;
                    row.values.get(selected_col)?.clone()
                }
                DatabaseExplorerState::SqlResults(_) => {
                    let table = &explorer.sql_executor.table_state;
                    let selected_row = table.view.state.selected()?;
                    let selected_col =
                        table.view.state.selected_column().unwrap_or(0);
                    let row = table.model.items.get(selected_row)?;
                    row.values.get(selected_col)?.clone()
                }
            };
            Some(v)
        })();
        if let Some(value) = value
            && execute!(
                stdout(),
                clipboard::CopyToClipboard {
                    content: value.clone(),
                    destination: clipboard::ClipboardSelection(vec![
                        clipboard::ClipboardType::Clipboard,
                    ]),
                }
            )
            .is_ok()
        {
            self.set_status(format!("Copied: {value}"));
        }
    }

    /// Queue describe for the current selection (off the UI thread).
    pub(crate) fn spawn_describe_rows(&mut self) -> bool {
        let Some((target, work)) = self.describe_work_for_selection() else {
            return false;
        };
        let key = describe_key();
        if !self.in_flight.try_begin(key.clone()) {
            self.set_status("Already loading describe…");
            return true;
        }
        let database = self.database_explorer.database.clone();
        let tx = self.bg_tx.clone();
        self.set_status("Loading describe…");
        tokio::spawn(async move {
            let rows = build_describe_rows_async(work, database).await;
            let _ = tx.send(BgEvent::DescribeRowsLoaded { key, target, rows });
        });
        true
    }

    pub(crate) fn apply_describe_rows_loaded(
        &mut self,
        target: &DescribeTarget,
        rows: Vec<DescribeRow>,
    ) {
        if !self.describe_target_matches(target) || rows.is_empty() {
            return;
        }
        self.describe_table = TableDataState::new(rows);
        self.show_describe = true;
    }

    pub(crate) fn apply_connection_test_result(
        &mut self,
        success: bool,
        message: String,
    ) {
        use crate::ui::widgets::connection_modal::TestResult;
        let Some(modal) = self.modal_manager.get_connection_modal_mut() else {
            return;
        };
        if !matches!(modal.test_result, TestResult::Testing) {
            return;
        }
        modal.test_result = if success {
            TestResult::Success
        } else {
            TestResult::Failed(message)
        };
    }

    fn describe_target_matches(&self, target: &DescribeTarget) -> bool {
        self.describe_target_for_selection().as_ref() == Some(target)
    }

    fn describe_target_for_selection(&self) -> Option<DescribeTarget> {
        let explorer = &self.database_explorer;
        match &explorer.state {
            DatabaseExplorerState::Connections => explorer
                .connections
                .table
                .view
                .state
                .selected()
                .and_then(|i| explorer.connections.table.model.items.get(i))
                .map(|c| DescribeTarget::Connection {
                    name: c.name.clone(),
                }),
            DatabaseExplorerState::Tables(_) => {
                explorer.tables.as_ref().and_then(|t| {
                    let i = t.table.view.state.selected()?;
                    let table = t.table.model.items.get(i)?;
                    Some(DescribeTarget::Table {
                        schema: table.schema.clone(),
                        name: table.name.clone(),
                    })
                })
            }
            DatabaseExplorerState::Columns(schema, table_name) => {
                explorer.columns.as_ref().and_then(|c| {
                    let i = c.table.view.state.selected()?;
                    let col = c.table.model.items.get(i)?;
                    Some(DescribeTarget::Column {
                        schema: schema.clone(),
                        table: table_name.clone(),
                        column: col.name.clone(),
                    })
                })
            }
            DatabaseExplorerState::TableData(schema, table_name) => {
                explorer.table_data.as_ref().and_then(|t| {
                    let i = t.table.view.state.selected()?;
                    Some(DescribeTarget::TableDataRow {
                        schema: schema.clone(),
                        table: table_name.clone(),
                        row_index: i,
                    })
                })
            }
            DatabaseExplorerState::Databases
            | DatabaseExplorerState::Schemas
            | DatabaseExplorerState::SqlResults(_) => None,
        }
    }

    fn describe_work_for_selection(
        &self,
    ) -> Option<(DescribeTarget, DescribeWork)> {
        let target = self.describe_target_for_selection()?;
        let explorer = &self.database_explorer;
        let work = match &target {
            DescribeTarget::Connection { name } => {
                let conn = explorer
                    .connections
                    .table
                    .model
                    .items
                    .iter()
                    .find(|c| c.name == *name)?
                    .clone();
                DescribeWork::Connection(conn)
            }
            DescribeTarget::Table { schema, name } => {
                let table =
                    explorer.tables.as_ref().and_then(|t| {
                        t.table.model.items.iter().find(|tbl| {
                            tbl.schema == *schema && tbl.name == *name
                        })
                    })?;
                DescribeWork::Table(table.clone())
            }
            DescribeTarget::Column {
                schema,
                table,
                column,
            } => {
                let col = explorer.columns.as_ref().and_then(|c| {
                    c.table.model.items.iter().find(|col| col.name == *column)
                })?;
                DescribeWork::Column {
                    schema: schema.clone(),
                    table: table.clone(),
                    column: col.clone(),
                }
            }
            DescribeTarget::TableDataRow {
                schema,
                table,
                row_index,
            } => {
                let table_data = explorer.table_data.as_ref()?;
                let row = table_data.table.model.items.get(*row_index)?;
                let names = table_data
                    .table
                    .model
                    .dynamic_column_names
                    .as_ref()
                    .map(|n| n.to_vec())
                    .unwrap_or_default();
                DescribeWork::TableDataRow {
                    schema: schema.clone(),
                    table: table.clone(),
                    values: row.values.clone(),
                    field_names: names,
                }
            }
        };
        Some((target, work))
    }

    /// Copy the full selected row as tab-separated values.
    pub(crate) fn copy_row_tsv(&mut self) {
        let explorer = &self.database_explorer;
        let values: Option<Vec<String>> = (|| -> Option<Vec<String>> {
            match &explorer.state {
                DatabaseExplorerState::TableData(_, _) => {
                    let table_data = explorer.table_data.as_ref()?;
                    let selected_row =
                        table_data.table.view.state.selected()?;
                    Some(
                        table_data
                            .table
                            .model
                            .items
                            .get(selected_row)?
                            .values
                            .clone(),
                    )
                }
                DatabaseExplorerState::SqlResults(_) => {
                    let table = &explorer.sql_executor.table_state;
                    let selected_row = table.view.state.selected()?;
                    Some(table.model.items.get(selected_row)?.values.clone())
                }
                DatabaseExplorerState::Connections
                | DatabaseExplorerState::Databases
                | DatabaseExplorerState::Schemas
                | DatabaseExplorerState::Tables(_)
                | DatabaseExplorerState::Columns(_, _) => None,
            }
        })();
        if let Some(values) = values {
            let tsv = values.join("\t");
            if execute!(
                stdout(),
                clipboard::CopyToClipboard {
                    content: tsv,
                    destination: clipboard::ClipboardSelection(vec![
                        clipboard::ClipboardType::Clipboard,
                    ]),
                }
            )
            .is_ok()
            {
                self.set_status("Copied row (TSV)".to_string());
            }
        }
    }

    /// Write SQL result rows to a temp TSV file.
    pub(crate) fn export_sql_results_tsv(&mut self) {
        if !matches!(
            self.database_explorer.state,
            DatabaseExplorerState::SqlResults(_)
        ) {
            return;
        }
        let executor = &self.database_explorer.sql_executor;
        if executor.table_state.model.items.is_empty() {
            self.set_status("No results to export".to_string());
            return;
        }
        let mut out = executor.column_names.join("\t");
        out.push('\n');
        for row in &executor.table_state.model.items {
            out.push_str(&row.values.join("\t"));
            out.push('\n');
        }
        let path = temp_dir().join(format!("d7s_results_{}.tsv", id()));
        match write(&path, out) {
            Ok(()) => self.set_status(format!("Wrote {}", path.display())),
            Err(e) => self.set_status(format!("Export failed: {e}")),
        }
    }

    /// Set running to false to quit the application.
    pub(crate) const fn quit(&mut self) {
        self.running = false;
    }

    /// Set the status line message
    pub fn set_status<S: Into<String>>(&mut self, message: S) {
        self.status_line.set_message(message);
    }

    /// Clear the status line
    pub fn clear_status(&mut self) {
        self.status_line.clear();
    }

    pub(crate) fn connect_status_message(
        &mut self,
        name: &str,
        env: Environment,
    ) -> String {
        if self.showed_help_hint {
            format!("Connected to {name} ({env})")
        } else {
            self.showed_help_hint = true;
            format!("Connected to {name} ({env}) — press ? for help")
        }
    }

    fn run_editor(terminal: &mut DefaultTerminal, path: &Path) -> Result<()> {
        let editor = var("VISUAL")
            .or_else(|_| var("EDITOR"))
            .unwrap_or_else(|_| "vim".to_string());
        let (program, args) = Self::parse_editor_command(&editor);

        execute!(stdout(), DisableBracketedPaste)?;
        stdout().execute(LeaveAlternateScreen)?;
        disable_raw_mode()?;
        let mut cmd = Command::new(&program);
        cmd.args(args);
        cmd.arg(path).status()?;
        stdout().execute(EnterAlternateScreen)?;
        enable_raw_mode()?;
        execute!(stdout(), EnableBracketedPaste)?;
        terminal.clear()?;
        Ok(())
    }

    fn parse_editor_command(editor: &str) -> (String, Vec<String>) {
        let mut parts = editor.split_whitespace();
        let program = parts.next().unwrap_or("vim").to_string();
        let args = parts.map(ToString::to_string).collect();
        (program, args)
    }

    fn enter_sql_results_state(&mut self, statement: String) {
        if !matches!(
            self.database_explorer.state,
            DatabaseExplorerState::SqlResults(_)
        ) {
            let current_state = self.database_explorer.state.clone();
            self.database_explorer.previous_state = Some(current_state);
        }
        // A newly selected/edited statement is a view change: stop watching
        // the old one rather than silently watching the new query.
        self.watch_active = false;
        self.database_explorer.state =
            DatabaseExplorerState::SqlResults(statement);
    }

    pub(crate) fn prepare_sql_statement_execution(
        &mut self,
        statement: String,
    ) {
        if classify_statement(&statement)
            == StatementSafety::RequiresConfirmation
        {
            self.modal_manager
                .open_sql_execution_confirmation_modal(statement);
        } else {
            self.execute_sql_statement_now(statement);
        }
    }

    pub(crate) fn execute_sql_statement_now(&mut self, statement: String) {
        self.enter_sql_results_state(statement.clone());
        self.database_explorer
            .sql_executor
            .set_selected_statement(statement);
        self.execute_sql_query();
    }

    /// `L`: show every query d7s ran this session, newest first.
    pub(crate) fn open_query_log_view(&mut self) {
        self.query_log_table = TableDataState::new(self.query_log.snapshot());
        self.show_query_log = true;
    }

    /// `A`: show currently-running Postgres backends via `pg_stat_activity`.
    /// SQLite is an embedded/file engine with no server process to inspect,
    /// so there's nothing equivalent to show there.
    pub(crate) fn open_activity_view(&mut self) {
        if !self.database_explorer.connection.r#type.has_activity_view() {
            self.set_status(
                "Activity view is Postgres-only (pg_stat_activity has no equivalent here).",
            );
            return;
        }
        self.execute_sql_statement_now(ACTIVITY_QUERY.to_string());
    }
}

enum DescribeWork {
    Connection(Connection),
    Table(Table),
    Column {
        schema: String,
        table: String,
        column: Column,
    },
    TableDataRow {
        schema: String,
        table: String,
        values: Vec<String>,
        field_names: Vec<String>,
    },
}

#[allow(clippy::too_many_lines)]
async fn build_describe_rows_async(
    work: DescribeWork,
    database: Option<Arc<dyn Database>>,
) -> Vec<DescribeRow> {
    match work {
        DescribeWork::Connection(conn) => connection_describe_rows(&conn),
        DescribeWork::Table(table) => {
            let mut rows = vec![
                DescribeRow::section("Table"),
                DescribeRow::new("Name", &table.name),
                DescribeRow::new("Schema", &table.schema),
            ];
            if let Some(db) = database.as_ref() {
                if let Ok(count) =
                    db.get_table_row_count(&table.schema, &table.name).await
                {
                    rows.push(DescribeRow::new("Row count", count.to_string()));
                }
                if let Ok(cols) =
                    db.get_columns(&table.schema, &table.name).await
                {
                    rows.push(DescribeRow::new(
                        "Columns",
                        cols.len().to_string(),
                    ));
                }
                if let Ok(pk) =
                    db.get_primary_key_columns(&table.schema, &table.name).await
                    && !pk.is_empty()
                {
                    rows.push(DescribeRow::new("Primary key", pk.join(", ")));
                }
                if let Ok(indexes) =
                    db.get_table_index_names(&table.schema, &table.name).await
                    && !indexes.is_empty()
                {
                    rows.push(DescribeRow::new("Indexes", indexes.join(", ")));
                }
                let size = db
                    .get_table_size(&table.schema, &table.name)
                    .await
                    .ok()
                    .flatten()
                    .or_else(|| table.size.clone());
                if let Some(size) = size {
                    rows.push(DescribeRow::new("Size", size));
                }
            } else if let Some(size) = table.size.clone() {
                rows.push(DescribeRow::new("Size", size));
            }
            rows
        }
        DescribeWork::Column {
            schema,
            table,
            column: col,
        } => {
            let mut rows = vec![
                DescribeRow::section("Column"),
                DescribeRow::new("Table", format!("{schema}.{table}")),
                DescribeRow::new("Name", &col.name),
                DescribeRow::new("Type", &col.data_type),
                DescribeRow::new(
                    "Nullable",
                    if col.is_nullable { "YES" } else { "NO" },
                ),
                DescribeRow::new(
                    "Default",
                    col.default_value.clone().unwrap_or_default(),
                ),
            ];
            if let Some(db) = database.as_ref()
                && let Ok(pk) =
                    db.get_primary_key_columns(&schema, &table).await
            {
                rows.push(DescribeRow::new(
                    "Part of key",
                    if pk.iter().any(|k| k == &col.name) {
                        "YES"
                    } else {
                        "NO"
                    },
                ));
            }
            rows.push(DescribeRow::new(
                "Description",
                col.description.clone().unwrap_or_default(),
            ));
            rows
        }
        DescribeWork::TableDataRow {
            schema,
            table,
            values,
            field_names,
        } => {
            let mut rows = vec![
                DescribeRow::section("Row"),
                DescribeRow::new("Table", format!("{schema}.{table}")),
            ];
            for (idx, value) in values.iter().enumerate() {
                let field = field_names
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| format!("col{idx}"));
                rows.push(DescribeRow::new(field, value.clone()));
            }
            rows
        }
    }
}

/// Turn a connection's existing `Display` output ("` Field: value`" per line)
/// into describe rows, reusing the formatting already defined for it.
fn connection_describe_rows(conn: &Connection) -> Vec<DescribeRow> {
    let mut rows = vec![DescribeRow::section("Connection")];
    for line in conn.to_string().lines() {
        if let Some((field, value)) = line.trim().split_once(':') {
            rows.push(DescribeRow::new(field.trim(), value.trim()));
        }
    }
    rows.push(DescribeRow::new(
        "Environment",
        conn.environment.to_string(),
    ));
    rows
}

/// `pg_stat_activity` columns worth showing at a glance. Permission-denied
/// (non-superusers without `pg_read_all_stats` may only see their own rows,
/// or be refused entirely) surfaces as a normal SQL error via the existing
/// error path rather than crashing.
pub const ACTIVITY_QUERY: &str = "SELECT pid, query, state, wait_event, query_start \
FROM pg_stat_activity ORDER BY query_start DESC NULLS LAST";

/// Info related to the program
fn build_info() -> Result<String> {
    let mut lines = vec![
        format!("Name: {}", app::PKG_NAME),
        format!("Version: {}", app::PKG_VERSION),
    ];
    if var("D7S_DEMO").is_err() {
        let path_buf = current_dir()?;
        let cwd = path_buf.as_path().to_str().unwrap_or(".");
        lines.push(format!("Path: {}", shorten_home_path(cwd)));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{remove_file, write},
        net::TcpListener,
        path,
        sync::Arc,
        time::SystemTime,
    };

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{
        Terminal, backend::TestBackend, buffer::Cell, style::Modifier,
    };
    use rusqlite::Connection as RusqliteConnection;
    use tokio::time::sleep;

    use super::*;
    use crate::{
        db::{
            Table,
            connection::{Connection, ConnectionStatus, ConnectionType},
            postgres::Postgres,
            query_log::{QueryLogEntry, QueryOrigin, tests::temp_sqlite},
            tls::TlsConfig,
        },
        in_flight::table_key,
        ui::widgets::hotkeys::DATABASE_HOTKEYS,
    };

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    /// App connected to a throwaway `SQLite` file, wired through the log.
    fn sqlite_app<'a>(tag: &str) -> App<'a> {
        let sqlite = temp_sqlite(tag);
        RusqliteConnection::open(&sqlite.path)
            .unwrap()
            .execute_batch("CREATE TABLE t(a); INSERT INTO t VALUES (7);")
            .unwrap();
        let mut app = App::default();
        let conn = Connection {
            name: tag.to_string(),
            r#type: ConnectionType::Sqlite,
            url: sqlite.path.clone(),
            ..Connection::default()
        };
        let db = Arc::from(app.query_log.wrap(Box::new(sqlite)));
        app.database_explorer = DatabaseExplorer::new(conn, Some(db));
        app.state = AppState::DatabaseConnected;
        app.database_explorer.state =
            DatabaseExplorerState::Tables("sqlite_schema".into());
        app
    }

    fn render_text(app: &mut App<'_>) -> String {
        render_text_w(app, 140)
    }

    fn render_text_w(app: &mut App<'_>, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let buf = terminal.backend().buffer();
        buf.content()
            .chunks(usize::from(buf.area.width))
            .map(|row| row.iter().map(Cell::symbol).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn entry(origin: QueryOrigin, sql: &str, ok: bool) -> QueryLogEntry {
        QueryLogEntry {
            at: SystemTime::UNIX_EPOCH,
            origin,
            duration: Duration::from_millis(12),
            sql: sql.to_string(),
            outcome: if ok { Ok(3) } else { Err("boom".into()) },
        }
    }

    #[tokio::test]
    async fn query_log_sql_column_fills_width_and_truncates() {
        let mut app = sqlite_app("qlwidth");
        let sql = format!("SELECT {} FROM t", ["col_a"; 14].join(", "));
        let mut e = entry(QueryOrigin::User, &sql, true);
        e.outcome = Ok(1);
        app.query_log.push(e);
        app.on_key_event(key('L')).unwrap();

        let wide = render_text_w(&mut app, 160);
        assert!(wide.contains(&sql), "full SQL visible at 160:\n{wide}");
        assert!(wide.contains("1 row "), "singular row:\n{wide}");

        let narrow = render_text_w(&mut app, 100);
        assert!(narrow.contains("SQL"), "SQL header at 100:\n{narrow}");
        assert!(
            narrow.contains("SELECT col_a") && narrow.contains('…'),
            "truncated SQL at 100:\n{narrow}"
        );
    }

    #[tokio::test]
    async fn l_opens_query_log_view_newest_first_and_esc_returns() {
        let mut app = sqlite_app("qlview");
        app.query_log
            .push(entry(QueryOrigin::User, "SELECT older", true));
        app.query_log
            .push(entry(QueryOrigin::Watch, "SELECT newer", false));

        app.on_key_event(key('L')).unwrap();
        assert!(app.show_query_log);
        let text = render_text(&mut app);
        for want in [
            "Query log",
            "Origin",
            "Duration",
            "watch",
            "user",
            "ERR boom",
            "3 rows",
        ] {
            assert!(text.contains(want), "missing {want:?} in:\n{text}");
        }
        let newer = text.find("SELECT newer").expect("newer row");
        let older = text.find("SELECT older").expect("older row");
        assert!(newer < older, "newest entry must be listed first");

        app.on_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(!app.show_query_log);
        assert!(!render_text(&mut app).contains("Origin"));

        // `q` also returns (and does not quit)
        app.running = true;
        app.on_key_event(key('L')).unwrap();
        app.on_key_event(key('q')).unwrap();
        assert!(!app.show_query_log && app.running);
    }

    #[tokio::test]
    async fn activity_key_is_ignored_on_sqlite() {
        use crate::{
            db::connection::ConnectionType,
            ui::widgets::hotkeys::database_hotkeys,
        };
        let mut app = sqlite_app("noact");
        app.on_key_event(key('A')).unwrap();
        assert!(app.query_log.snapshot().is_empty(), "nothing may run");
        assert!(matches!(
            app.database_explorer.state,
            DatabaseExplorerState::Tables(_)
        ));
        assert!(!render_text(&mut app).contains("Postgres-only"));

        let has_activity = |t| {
            database_hotkeys(t)
                .iter()
                .any(|h| h.keycode == KeyCode::Char('A'))
        };
        assert!(has_activity(ConnectionType::Postgres));
        assert!(!has_activity(ConnectionType::Sqlite));
        assert!(!has_activity(ConnectionType::Mysql));
    }

    #[test]
    fn activity_view_renders_pg_stat_activity_columns() {
        let mut app = sqlite_app("actview");
        app.enter_sql_results_state(ACTIVITY_QUERY.to_string());
        let cols: Vec<String> =
            ["pid", "query", "state", "wait_event", "query_start"]
                .map(String::from)
                .to_vec();
        app.database_explorer.sql_executor.set_results(
            vec![vec![
                "4242".into(),
                "select pg_sleep(9)".into(),
                "active".into(),
                "PgSleep".into(),
                "2026-01-01".into(),
            ]],
            &cols,
        );
        let text = render_text(&mut app);
        for want in ["pid", "wait_event", "query_start", "4242", "pg_sleep"] {
            assert!(text.contains(want), "missing {want:?} in:\n{text}");
        }
    }

    #[tokio::test]
    async fn w_toggles_watch_only_on_sql_results_and_shows_indicator() {
        let mut app = sqlite_app("watchkey");
        app.on_key_event(key('w')).unwrap();
        assert!(!app.watch_active, "w is inert outside SQL results");

        app.enter_sql_results_state("SELECT a FROM t".into());
        app.on_key_event(key('w')).unwrap();
        assert!(app.watch_active);
        // The toggle's own status message temporarily replaces the idle hint.
        assert!(render_text(&mut app).contains("Watching (every 2s)"));
        app.clear_status();
        assert!(render_text(&mut app).contains("WATCHING"));

        app.on_key_event(key('w')).unwrap();
        assert!(!app.watch_active);
        assert!(!render_text(&mut app).contains("WATCHING"));
    }

    #[tokio::test]
    async fn sql_execute_refuses_when_sql_key_busy() {
        let mut app = sqlite_app("conn");
        app.database_explorer
            .sql_executor
            .set_selected_statement("SELECT a FROM t");
        let key = sql_key(&app.database_explorer.connection.name);
        assert!(app.in_flight.try_begin(key.clone()));
        app.execute_sql_query();
        assert!(app.in_flight.contains(&key));
    }

    #[tokio::test]
    async fn watch_tick_is_logged_but_not_added_to_sql_history() {
        let mut app = sqlite_app("watchtick");
        app.enter_sql_results_state("SELECT a FROM t".into());
        app.database_explorer
            .sql_executor
            .set_selected_statement("SELECT a FROM t");
        let history_before = PreferencesService::load_sql_history();

        app.execute_sql_query_watch_tick();
        drain_bg_work(&mut app).await;

        assert_eq!(PreferencesService::load_sql_history(), history_before);
        let snap = app.query_log.snapshot();
        assert_eq!(snap.len(), 1);
        let e = snap.first().unwrap();
        assert_eq!(e.origin, QueryOrigin::Watch);
        assert_eq!(e.sql, "SELECT a FROM t");
        assert_eq!(e.outcome, Ok(1));
    }

    async fn tables_app<'a>(tag: &str) -> App<'a> {
        let mut app = sqlite_app(tag);
        RusqliteConnection::open(&app.database_explorer.connection.url)
            .unwrap()
            .execute_batch(
                "CREATE TABLE users(id, note TEXT); CREATE TABLE orders(id);
                 CREATE TABLE order_items(id);
                 INSERT INTO users(id) VALUES (1),(2),(3);",
            )
            .unwrap();
        app.load_tables("sqlite_schema");
        drain_bg_work(&mut app).await;
        app
    }

    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }

    #[allow(clippy::future_not_send)]
    fn type_str(app: &mut App<'_>, s: &str) {
        for c in s.chars() {
            app.on_key_event(key(c)).unwrap();
        }
    }

    fn sqlite_conn(name: &str, path: &str) -> Connection {
        Connection {
            name: name.into(),
            r#type: ConnectionType::Sqlite,
            url: path.into(),
            ..Connection::default()
        }
    }

    #[tokio::test]
    async fn p_pings_all_connections_and_statuses_survive_filter_clear() {
        let present = temp_sqlite("ping-present");
        write(&present.path, b"").unwrap();
        let missing = temp_sqlite("ping-missing");
        // A port that was just free, so nothing is listening on it.
        let closed_port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let pg = Connection {
            name: "pg-down".into(),
            r#type: ConnectionType::Postgres,
            url: format!("postgres://u@127.0.0.1:{closed_port}/db"),
            ..Connection::default()
        };

        let mut app = App::default();
        app.database_explorer.connections = FilteredData::new(vec![
            sqlite_conn("present", &present.path),
            sqlite_conn("missing", &missing.path),
            pg,
        ]);
        // Filtered view shows only one row; the ping must still cover all.
        app.database_explorer.connections.apply_filter("present");
        app.on_key_event(key('p')).unwrap();
        drain_bg_work(&mut app).await;

        let status = |c: &FilteredData<Connection>| {
            c.original.iter().map(|c| c.status).collect::<Vec<_>>()
        };
        assert_eq!(
            status(&app.database_explorer.connections),
            vec![
                ConnectionStatus::Up,
                ConnectionStatus::Down,
                ConnectionStatus::Down
            ]
        );
        assert!(
            !path::Path::new(&missing.path).exists(),
            "ping must not create a missing sqlite file"
        );

        app.database_explorer.connections.clear_filter();
        let text = render_text(&mut app);
        assert!(text.contains("Status"));
        assert!(text.contains("● up"));
        assert_eq!(text.matches("● down").count(), 2);
        let _ = remove_file(&present.path);
    }

    #[tokio::test]
    async fn ping_if_any_skips_when_no_connections() {
        let mut app = App::default();
        app.ping_connections_if_any();
        assert!(app.database_explorer.connections.original.is_empty());
    }

    #[tokio::test]
    async fn ping_if_any_sets_statuses() {
        let present = temp_sqlite("launch-ping-present");
        write(&present.path, b"").unwrap();
        let missing = temp_sqlite("launch-ping-missing");

        let mut app = App::default();
        app.database_explorer.connections = FilteredData::new(vec![
            sqlite_conn("present", &present.path),
            sqlite_conn("missing", &missing.path),
        ]);
        assert!(
            app.database_explorer
                .connections
                .original
                .iter()
                .all(|c| c.status == ConnectionStatus::Unknown)
        );

        app.ping_connections_if_any();
        drain_bg_work(&mut app).await;

        let statuses: Vec<_> = app
            .database_explorer
            .connections
            .original
            .iter()
            .map(|c| c.status)
            .collect();
        assert_eq!(
            statuses,
            vec![ConnectionStatus::Up, ConnectionStatus::Down]
        );
        let _ = remove_file(&present.path);
    }

    #[tokio::test]
    async fn refresh_wiping_status_is_restored_by_reprobe() {
        // refresh_connections replaces rows from disk (Unknown status). Simulate
        // that wipe, then the same reprobe refresh ends with.
        let present = temp_sqlite("refresh-ping-present");
        write(&present.path, b"").unwrap();
        let mut app = App::default();
        app.database_explorer.connections =
            FilteredData::new(vec![sqlite_conn("present", &present.path)]);
        app.ping_connections_if_any();
        drain_bg_work(&mut app).await;
        let status = |app: &App<'_>| {
            app.database_explorer
                .connections
                .original
                .first()
                .map(|c| c.status)
        };
        assert_eq!(status(&app), Some(ConnectionStatus::Up));

        app.database_explorer.connections =
            FilteredData::new(vec![sqlite_conn("present", &present.path)]);
        assert_eq!(status(&app), Some(ConnectionStatus::Unknown));
        app.ping_connections_if_any();
        drain_bg_work(&mut app).await;
        assert_eq!(status(&app), Some(ConnectionStatus::Up));
        let _ = remove_file(&present.path);
    }

    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }

    /// Wall-clock bound (not a yield count) so slow CI runners let
    /// `spawn_blocking` work finish.
    async fn drain_bg_work(app: &mut App<'_>) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            app.drain_bg_events();
            if app.in_flight.is_empty() {
                return;
            }
            sleep(Duration::from_millis(1)).await;
        }
    }

    #[allow(clippy::future_not_send)]
    async fn run_cmd(app: &mut App<'_>, cmd: &str) {
        app.on_key_event(key(':')).unwrap();
        type_str(app, cmd);
        app.on_key_event(enter()).unwrap();
        drain_bg_work(app).await;
    }

    #[tokio::test]
    async fn colon_bar_types_completes_and_executes() {
        let mut app = tables_app("cmd_flow").await;
        app.on_key_event(key(':')).unwrap();
        assert!(app.command_mode && app.search_filter.is_some());
        type_str(&mut app, "tab");
        assert_eq!(app.command_suggestion().as_deref(), Some("table "));
        app.on_key_event(tab()).unwrap();
        assert_eq!(app.command_text(), "table ");
        type_str(&mut app, "us");
        assert_eq!(app.command_suggestion().as_deref(), Some("table users"));
        let text = render_text(&mut app);
        assert!(text.contains("users"), "ghost completion rendered");
        app.on_key_event(tab()).unwrap();
        assert_eq!(app.command_text(), "table users");
        app.on_key_event(enter()).unwrap();
        drain_bg_work(&mut app).await;
        assert!(!app.command_mode && app.search_filter.is_none());
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::TableData(
                "sqlite_schema".into(),
                "users".into()
            )
        );
    }

    #[tokio::test]
    async fn load_table_data_refuses_when_table_key_busy() {
        let mut app = tables_app("tbl_busy").await;
        let key = table_key(
            &app.database_explorer.connection.name,
            "sqlite_schema",
            "users",
        );
        assert!(app.in_flight.try_begin(key.clone()));
        app.load_table_data("sqlite_schema", "users");
        assert!(app.in_flight.contains(&key));
        assert_eq!(app.status_line.message(), "Already loading this table");
    }

    #[tokio::test]
    async fn table_page_from_previous_connection_is_dropped() {
        let mut app = tables_app("tbl_stale_conn").await;
        let before = app.database_explorer.state.clone();
        app.load_table_data("sqlite_schema", "users");
        // User switched to another connection while the page was loading.
        app.database_explorer.connection.name = "other".into();
        drain_bg_work(&mut app).await;
        assert!(app.in_flight.is_empty());
        assert_eq!(app.database_explorer.state, before);
        assert!(app.database_explorer.table_data.is_none());
    }

    #[tokio::test]
    async fn stale_table_page_load_error_is_discarded() {
        let mut app = tables_app("tbl_stale_err").await;
        app.load_table_data("sqlite_schema", "users");
        app.database_explorer.state = DatabaseExplorerState::TableData(
            "sqlite_schema".into(),
            "orders".into(),
        );
        app.set_status("viewing orders");
        app.apply_table_page_loaded(
            "sqlite_schema",
            "users",
            true,
            Err("late error".into()),
        );
        assert_eq!(app.status_line.message(), "viewing orders");
        assert!(app.database_explorer.table_data.is_none());
    }

    #[tokio::test]
    async fn stale_describe_load_is_discarded() {
        let mut app = tables_app("desc_stale").await;
        app.on_key_event(key('d')).unwrap();
        app.database_explorer.state = DatabaseExplorerState::TableData(
            "sqlite_schema".into(),
            "orders".into(),
        );
        drain_bg_work(&mut app).await;
        assert!(!app.show_describe);
    }

    #[tokio::test]
    async fn describe_refuses_when_describe_key_busy() {
        let mut app = tables_app("desc_busy").await;
        let key = describe_key();
        assert!(app.in_flight.try_begin(key.clone()));
        assert!(app.spawn_describe_rows());
        assert!(app.in_flight.contains(&key));
        assert_eq!(app.status_line.message(), "Already loading describe…");
    }

    #[tokio::test]
    async fn stale_table_page_load_is_discarded() {
        let mut app = tables_app("tbl_stale").await;
        app.load_table_data("sqlite_schema", "users");
        app.database_explorer.state = DatabaseExplorerState::TableData(
            "sqlite_schema".into(),
            "orders".into(),
        );
        drain_bg_work(&mut app).await;
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::TableData(
                "sqlite_schema".into(),
                "orders".into()
            )
        );
        assert!(app.database_explorer.table_data.is_none());
    }

    #[tokio::test]
    async fn colon_bar_renders_prompt_and_backspace_edits() {
        let mut app = tables_app("cmd_render").await;
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "xq");
        app.on_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.command_text(), "x");
        let text = render_text(&mut app);
        assert!(text.contains("┤ Command ├") || text.contains(" Command "));
        assert!(text.contains(":x"), "prompt then typed text, no gap");
        assert!(!text.contains(": x"));
        assert!(!text.contains("Filter (current view)"));
    }

    #[tokio::test]
    async fn colon_bar_first_char_sits_right_after_prompt_with_ghost() {
        let mut app = tables_app("cmd_nogap").await;
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "table u");
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let buf = terminal.backend().buffer();
        let rows: Vec<&[Cell]> =
            buf.content().chunks(usize::from(buf.area.width)).collect();
        let title = rows
            .iter()
            .position(|r| {
                r.iter()
                    .map(Cell::symbol)
                    .collect::<String>()
                    .contains("┌ Command ")
            })
            .expect("title row");
        let cells = rows.get(title + 1).expect("bar row");
        let colon = cells
            .iter()
            .position(|c| c.symbol() == ":")
            .expect("prompt cell");
        let after = |n: usize| cells.get(colon + n).expect("cell");
        assert_eq!(after(1).symbol(), "t", "first char right after ':'");
        assert_eq!(after(7).symbol(), "u", "last typed char");
        assert_eq!(after(8).symbol(), "s", "ghost right after typed text");
        assert!(after(8).modifier.contains(Modifier::REVERSED));
    }

    #[tokio::test]
    async fn colon_bar_esc_cancels_and_ctrl_c_quits() {
        let mut app = tables_app("cmd_esc").await;
        app.running = true;
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "users");
        app.on_key_event(esc()).unwrap();
        assert!(!app.command_mode && app.search_filter.is_none());
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::Tables("sqlite_schema".into())
        );
        app.on_key_event(key(':')).unwrap();
        app.on_key_event(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ))
        .unwrap();
        assert!(!app.running);
    }

    #[tokio::test]
    async fn colon_is_ignored_in_inputs_modals_and_overlays() {
        let mut app = tables_app("cmd_ignored").await;
        // Inside the `/` filter bar it is just text.
        app.on_key_event(key('/')).unwrap();
        app.on_key_event(key(':')).unwrap();
        assert!(!app.command_mode);
        assert_eq!(app.command_text(), ":");
        app.on_key_event(esc()).unwrap();
        // Help overlay.
        app.on_key_event(key('?')).unwrap();
        app.on_key_event(key(':')).unwrap();
        assert!(app.search_filter.is_none() && app.show_help);
        app.on_key_event(esc()).unwrap();
        // Modal.
        app.modal_manager.open_new_connection_modal();
        app.on_key_event(key(':')).unwrap();
        assert!(app.search_filter.is_none() && !app.command_mode);
    }

    #[tokio::test]
    async fn view_commands_navigate() {
        let mut app = tables_app("cmd_nav").await;
        let schema = "sqlite_schema".to_string();
        run_cmd(&mut app, "table users").await;
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::TableData(schema.clone(), "users".into())
        );
        run_cmd(&mut app, "2").await;
        assert_eq!(
            app.database_explorer.table_data.as_ref().and_then(|d| d
                .table
                .view
                .state
                .selected()),
            Some(1)
        );
        run_cmd(&mut app, "columns").await;
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::Columns(schema.clone(), "users".into())
        );
        run_cmd(&mut app, "tab").await;
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::Tables(schema.clone())
        );
        run_cmd(&mut app, "table ord").await;
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::TableData(schema, "orders".into()),
            "first prefix match (list order) wins"
        );
        assert!(
            render_text(&mut app).contains("2 matches: orders, order_items")
        );
    }

    #[tokio::test]
    async fn utility_commands_and_messages() {
        let mut app = tables_app("cmd_misc").await;
        run_cmd(&mut app, "zzz").await;
        assert!(render_text(&mut app).contains("Unknown command: zzz"));
        run_cmd(&mut app, "schemas").await;
        assert!(render_text(&mut app).contains("SQLite has no schemas"));
        run_cmd(&mut app, "schema public").await;
        assert!(render_text(&mut app).contains("SQLite has no schemas"));
        run_cmd(&mut app, "activity").await;
        assert!(render_text(&mut app).contains("Postgres-only"));
        run_cmd(&mut app, "123").await;
        assert!(render_text(&mut app).contains("needs an open table"));
        run_cmd(&mut app, "sql").await;
        assert!(app.open_editor_requested);
        run_cmd(&mut app, "log").await;
        assert!(app.show_query_log);
        app.show_query_log = false;
        run_cmd(&mut app, "help").await;
        assert!(app.show_help);
        app.show_help = false;
        run_cmd(&mut app, "conn").await;
        assert_eq!(app.state, AppState::ConnectionList);
        run_cmd(&mut app, "tables").await;
        assert!(render_text(&mut app).contains("Not connected"));
        app.running = true;
        run_cmd(&mut app, "q").await;
        assert!(!app.running);
    }

    fn table_data_cell_value(app: &App<'_>, col: Option<usize>) -> String {
        let fd = app.database_explorer.table_data.as_ref().unwrap();
        let row = fd.table.view.state.selected().unwrap();
        let col = col.unwrap_or_else(|| {
            fd.table.view.state.selected_column().unwrap_or(0)
        });
        fd.table
            .model
            .items
            .get(row)
            .and_then(|r| r.values.get(col))
            .cloned()
            .unwrap_or_default()
    }

    fn select_table_column(app: &mut App<'_>, col: usize) {
        let fd = app.database_explorer.table_data.as_mut().unwrap();
        fd.table.view.state.select_column(Some(col));
    }

    fn selected_col(app: &App<'_>) -> usize {
        app.database_explorer
            .table_data
            .as_ref()
            .unwrap()
            .table
            .view
            .state
            .selected_column()
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn n_creates_draft_and_enters_edit() {
        let mut app = tables_app("key_n").await;
        run_cmd(&mut app, "table users").await;
        app.on_key_event(key('n')).unwrap();
        assert!(app.has_table_draft_rows());
        assert!(app.is_cell_editing());
    }

    #[tokio::test]
    async fn a_aliases_n() {
        let mut app = tables_app("key_a").await;
        run_cmd(&mut app, "table users").await;
        app.on_key_event(key('a')).unwrap();
        assert!(app.has_table_draft_rows());
    }

    async fn defaults_draft_table_app(tag: &str) -> App<'static> {
        let mut app = sqlite_app(tag);
        rusqlite::Connection::open(&app.database_explorer.connection.url)
            .unwrap()
            .execute_batch(
                "CREATE TABLE def_row(id INTEGER PRIMARY KEY, name TEXT DEFAULT 'x', note TEXT);",
            )
            .unwrap();
        app.load_tables("sqlite_schema");
        drain_bg_work(&mut app).await;
        app
    }

    #[tokio::test]
    async fn n_draft_spawns_column_meta_when_uncached() {
        let mut app = defaults_draft_table_app("draft_defaults_spawn").await;
        run_cmd(&mut app, "table def_row").await;
        app.database_explorer.table_columns = None;
        app.database_explorer.table_pk_names = None;
        app.on_key_event(KeyEvent::new(
            KeyCode::Char('N'),
            KeyModifiers::SHIFT,
        ))
        .unwrap();
        assert!(app.pending_defaults_draft);
        assert!(!app.has_table_draft_rows());
        drain_bg_work(&mut app).await;
        assert!(!app.pending_defaults_draft);
        assert!(app.has_table_draft_rows());
    }

    #[tokio::test]
    async fn n_draft_omits_default_columns_on_commit() {
        let mut app = defaults_draft_table_app("draft_defaults").await;
        run_cmd(&mut app, "table def_row").await;
        app.on_key_event(KeyEvent::new(
            KeyCode::Char('N'),
            KeyModifiers::SHIFT,
        ))
        .unwrap();
        assert!(app.has_table_draft_rows());
        assert_eq!(selected_col(&app), 1, "first editable column is name");
        assert_eq!(
            table_data_cell_value(&app, Some(1)),
            "x",
            "N prefills literal defaults in the draft row"
        );
        assert_eq!(table_data_cell_value(&app, Some(2)), "NULL");
        assert!(
            render_text(&mut app).contains("Draft with defaults ("),
            "status notes how many cols use defaults"
        );
        // First editable is `name` (id is rowid PK); tab to `note`.
        app.on_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(
            app.cell_edit.as_ref().unwrap().buffer,
            "",
            "NULL display starts empty so typing replaces"
        );
        assert_eq!(
            table_data_cell_value(&app, Some(2)),
            "NULL",
            "grid still shows NULL until first key"
        );
        type_str(&mut app, "hi");
        assert_eq!(app.cell_edit.as_ref().unwrap().buffer, "hi");
        app.on_key_event(enter()).unwrap();
        drain_bg_work(&mut app).await;
        app.on_key_event(key('s')).unwrap();
        drain_bg_work(&mut app).await;
        assert!(!app.has_table_draft_rows());
        let db_path = app.database_explorer.connection.url.clone();
        let name: String = rusqlite::Connection::open(&db_path)
            .unwrap()
            .query_row(
                "SELECT name FROM def_row ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, "x");
        let note: String = rusqlite::Connection::open(&db_path)
            .unwrap()
            .query_row(
                "SELECT note FROM def_row ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(note, "hi");
    }

    #[tokio::test]
    async fn tab_moves_to_next_column_on_draft() {
        let mut app = tables_app("inline_tab").await;
        run_cmd(&mut app, "table users").await;
        app.on_key_event(key('n')).unwrap();
        let c0 = selected_col(&app);
        app.on_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
        assert!(app.is_cell_editing());
        assert_ne!(selected_col(&app), c0);
    }

    #[tokio::test]
    async fn draft_enter_stashes_without_insert() {
        let mut app = tables_app("inline_stash").await;
        run_cmd(&mut app, "table users").await;
        app.on_key_event(key('n')).unwrap();
        app.on_key_event(key('z')).unwrap();
        app.on_key_event(enter()).unwrap();
        assert!(!app.is_cell_editing());
        assert!(app.has_table_draft_rows());
        assert_eq!(table_data_cell_value(&app, None), "z");
    }

    async fn cell_update_table_app(tag: &str) -> App<'static> {
        let mut app = sqlite_app(tag);
        rusqlite::Connection::open(&app.database_explorer.connection.url)
            .unwrap()
            .execute_batch(
                "CREATE TABLE cell_edit_row(id INTEGER PRIMARY KEY, name TEXT);
                 INSERT INTO cell_edit_row VALUES (1, 'old');",
            )
            .unwrap();
        app.load_tables("sqlite_schema");
        drain_bg_work(&mut app).await;
        app
    }

    fn backspace() -> KeyEvent {
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
    }

    #[tokio::test]
    async fn unbound_z_from_browse_starts_edit_with_z() {
        let mut app = cell_update_table_app("type_overwrite_z").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 1);
        assert!(!app.is_cell_editing());
        app.on_key_event(key('z')).unwrap();
        assert!(app.is_cell_editing());
        assert_eq!(
            app.cell_edit.as_ref().unwrap().buffer,
            "z",
            "type-to-overwrite seeds edit buffer"
        );
        assert_eq!(table_data_cell_value(&app, None), "z");
    }

    #[tokio::test]
    async fn type_to_overwrite_requires_column_selection() {
        let mut app = cell_update_table_app("no_col_overwrite").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        assert!(
            app.database_explorer
                .table_data
                .as_ref()
                .unwrap()
                .table
                .view
                .state
                .selected_column()
                .is_none()
        );
        app.on_key_event(key('z')).unwrap();
        assert!(!app.is_cell_editing());
        let id: i64 =
            rusqlite::Connection::open(&app.database_explorer.connection.url)
                .unwrap()
                .query_row(
                    "SELECT id FROM cell_edit_row WHERE name = 'old'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
        assert_eq!(id, 1);
    }

    #[tokio::test]
    async fn paste_while_editing_appends_to_buffer() {
        let mut app = cell_update_table_app("paste_edit").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 1);
        app.on_key_event(enter()).unwrap();
        app.handle_paste_text(" pasted");
        assert!(app.cell_edit.as_ref().unwrap().buffer.ends_with(" pasted"));
        assert!(table_data_cell_value(&app, None).ends_with(" pasted"));
    }

    #[tokio::test]
    async fn draft_discard_is_two_esc_from_browse() {
        let mut app = tables_app("draft_esc_count").await;
        run_cmd(&mut app, "table users").await;
        app.on_key_event(key('n')).unwrap();
        assert!(app.is_cell_editing());
        app.on_key_event(esc()).unwrap(); // cancel cell edit
        assert!(!app.is_cell_editing());
        assert!(app.has_table_draft_rows());
        app.on_key_event(esc()).unwrap(); // warn
        assert!(app.has_table_draft_rows());
        assert!(app.draft_discard_pending);
        app.on_key_event(esc()).unwrap(); // discard
        assert!(!app.has_table_draft_rows());
    }

    #[tokio::test]
    async fn table_data_browse_q_quits_without_cell_edit() {
        let mut app = cell_update_table_app("browse_q").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 1);
        app.running = true;
        app.on_key_event(key('q')).unwrap();
        assert!(!app.is_cell_editing());
        assert!(!app.running);
    }

    #[tokio::test]
    async fn table_data_browse_e_requests_editor_without_cell_edit() {
        let mut app = cell_update_table_app("browse_e").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 1);
        app.on_key_event(key('e')).unwrap();
        assert!(!app.is_cell_editing());
        assert!(app.open_editor_requested);
    }

    #[tokio::test]
    async fn table_data_browse_backspace_clears_cell_for_edit() {
        let mut app = cell_update_table_app("browse_backspace").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 1);
        assert_eq!(table_data_cell_value(&app, None), "old");
        app.on_key_event(backspace()).unwrap();
        assert!(app.is_cell_editing());
        assert_eq!(app.cell_edit.as_ref().unwrap().buffer, "");
        assert_eq!(table_data_cell_value(&app, None), "");
    }

    #[tokio::test]
    async fn d_ignored_while_editing() {
        let mut app = cell_update_table_app("edit_ignore_d").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 1);
        app.on_key_event(enter()).unwrap();
        assert!(app.is_cell_editing());
        app.on_key_event(KeyEvent::new(
            KeyCode::Char('D'),
            KeyModifiers::SHIFT,
        ))
        .unwrap();
        assert!(app.is_cell_editing());
        assert!(app.pending_row_deletes.is_none());
        let db_path = app.database_explorer.connection.url.clone();
        let n: i64 = rusqlite::Connection::open(&db_path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM cell_edit_row", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn ctrl_e_opens_cell_modal_while_editing() {
        let mut app = cell_update_table_app("edit_modal").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 1);
        app.on_key_event(enter()).unwrap();
        app.on_key_event(KeyEvent::new(
            KeyCode::Char('e'),
            KeyModifiers::CONTROL,
        ))
        .unwrap();
        assert!(app.modal_manager.get_cell_value_modal().is_some());
    }

    #[tokio::test]
    async fn saved_cell_finish_persists_update() {
        let mut app = cell_update_table_app("inline_save").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 1);
        app.on_key_event(enter()).unwrap();
        for _ in 0..3 {
            app.on_key_event(backspace()).unwrap();
        }
        type_str(&mut app, "nix");
        app.on_key_event(enter()).unwrap();
        drain_bg_work(&mut app).await;
        assert!(!app.is_cell_editing());
        assert!(!app.has_table_draft_rows());
        let db_path = app.database_explorer.connection.url.clone();
        let persisted: String = rusqlite::Connection::open(&db_path)
            .unwrap()
            .query_row("SELECT name FROM cell_edit_row WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(persisted, "nix");
        app.on_key_event(key('r')).unwrap();
        select_table_column(&mut app, 1);
        assert_eq!(table_data_cell_value(&app, Some(1)), "nix");
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn db_name(app: &App<'_>, id: i64) -> Option<String> {
        rusqlite::Connection::open(&app.database_explorer.connection.url)
            .unwrap()
            .query_row(
                "SELECT name FROM cell_edit_row WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn plain_e_types_into_cell_buffer() {
        let mut app = tables_app("edit_type_e").await;
        run_cmd(&mut app, "table users").await;
        app.on_key_event(key('n')).unwrap();
        type_str(&mut app, "hello");
        assert_eq!(app.cell_edit.as_ref().unwrap().buffer, "hello");
        assert!(app.modal_manager.get_cell_value_modal().is_none());
    }

    #[tokio::test]
    async fn unchanged_null_cell_is_not_written_back() {
        let mut app = cell_update_table_app("edit_null").await;
        rusqlite::Connection::open(&app.database_explorer.connection.url)
            .unwrap()
            .execute_batch("INSERT INTO cell_edit_row VALUES (2, NULL);")
            .unwrap();
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 1);
        app.on_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .unwrap();
        app.on_key_event(enter()).unwrap();
        assert_eq!(
            app.cell_edit.as_ref().unwrap().buffer,
            "",
            "NULL display edits start empty"
        );
        assert_eq!(table_data_cell_value(&app, Some(1)), "NULL");
        app.on_key_event(enter()).unwrap();
        assert!(!app.is_cell_editing());
        assert_eq!(db_name(&app, 2), None, "still SQL NULL, not 'NULL'");
        assert_eq!(table_data_cell_value(&app, Some(1)), "NULL");
    }

    #[tokio::test]
    async fn refused_create_does_not_enter_edit_on_saved_row() {
        let mut app = cell_update_table_app("edit_filtered_n").await;
        rusqlite::Connection::open(&app.database_explorer.connection.url)
            .unwrap()
            .execute_batch("INSERT INTO cell_edit_row VALUES (2, 'zed');")
            .unwrap();
        run_cmd(&mut app, "table cell_edit_row").await;
        app.on_key_event(key('/')).unwrap();
        type_str(&mut app, "zed");
        app.on_key_event(enter()).unwrap();
        for c in ['n', 'N', 'c'] {
            app.on_key_event(key(c)).unwrap();
            assert!(!app.is_cell_editing(), "{c}");
            assert!(!app.has_table_draft_rows(), "{c}");
        }
    }

    #[tokio::test]
    async fn ctrl_chords_do_not_type_and_ctrl_c_quits_while_editing() {
        let mut app = cell_update_table_app("edit_ctrl").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 1);
        app.on_key_event(enter()).unwrap();
        app.on_key_event(ctrl('x')).unwrap();
        app.on_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT))
            .unwrap();
        assert_eq!(app.cell_edit.as_ref().unwrap().buffer, "old");
        app.running = true;
        app.on_key_event(ctrl('c')).unwrap();
        assert!(!app.running);
        assert_eq!(db_name(&app, 1).as_deref(), Some("old"));
    }

    #[tokio::test]
    async fn rowid_pk_cell_is_not_editable_on_saved_rows() {
        let mut app = cell_update_table_app("edit_saved_pk").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 0);
        app.on_key_event(enter()).unwrap();
        assert!(!app.is_cell_editing());
        assert!(
            render_text(&mut app).contains("not editable"),
            "rowid PK refused like Tab skip"
        );
    }

    #[tokio::test]
    async fn table_data_discoverability_in_bar_status_and_cell_edit() {
        let mut app = tables_app("discover").await;
        run_cmd(&mut app, "table users").await;
        let idle = render_text(&mut app);
        assert!(idle.contains("n new row · Enter edit cell · ? help"));
        assert!(idle.contains("New row"));
        assert!(idle.contains("> Edit"));
        app.on_key_event(key('i')).unwrap();
        let editing = render_text(&mut app);
        assert!(editing.contains("> Cancel"));
        assert!(editing.contains("> Next col"));
        assert!(editing.contains("Ctrl+e"));
        assert!(editing.contains("> Done") || editing.contains("> Modal"));
        assert!(!editing.contains("> Duplicate"));
    }

    #[tokio::test]
    async fn clearing_saved_nullable_cell_persists_sql_null() {
        let mut app = cell_update_table_app("clear_to_null").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 1);
        app.on_key_event(backspace()).unwrap();
        assert!(app.is_cell_editing());
        assert_eq!(app.cell_edit.as_ref().unwrap().buffer, "");
        app.on_key_event(enter()).unwrap();
        drain_bg_work(&mut app).await;
        assert!(!app.is_cell_editing());
        assert_eq!(db_name(&app, 1), None, "cleared cell is SQL NULL");
    }

    #[tokio::test]
    async fn paste_goes_to_modal_when_ctrl_e_open() {
        let mut app = cell_update_table_app("paste_modal").await;
        run_cmd(&mut app, "table cell_edit_row").await;
        select_table_column(&mut app, 1);
        app.on_key_event(enter()).unwrap();
        app.on_key_event(KeyEvent::new(
            KeyCode::Char('e'),
            KeyModifiers::CONTROL,
        ))
        .unwrap();
        assert!(app.modal_manager.get_cell_value_modal().is_some());
        let before = app.cell_edit.as_ref().unwrap().buffer.clone();
        app.handle_paste_text(" via-modal");
        assert_eq!(
            app.cell_edit.as_ref().unwrap().buffer,
            before,
            "inline buffer unchanged while modal open"
        );
        assert!(
            render_text(&mut app).contains("via-modal"),
            "paste lands in the open modal"
        );
    }

    #[tokio::test]
    async fn enter_edits_draft_cell_inline_esc_reverts() {
        let mut app = tables_app("inline_esc").await;
        run_cmd(&mut app, "table users").await;
        app.on_key_event(key('n')).unwrap();
        assert!(app.has_table_draft_rows());
        assert!(app.is_cell_editing());
        app.on_key_event(key('x')).unwrap();
        assert_eq!(table_data_cell_value(&app, None), "x");
        app.on_key_event(esc()).unwrap();
        assert!(!app.is_cell_editing());
        assert_eq!(table_data_cell_value(&app, None), "");
        assert!(app.has_table_draft_rows());
    }

    #[tokio::test]
    async fn draft_row_blocks_navigation_commands() {
        let mut app = tables_app("cmd_draft").await;
        run_cmd(&mut app, "table users").await;
        app.on_key_event(key('n')).unwrap();
        app.on_key_event(esc()).unwrap();
        assert!(app.has_table_draft_rows());
        run_cmd(&mut app, "tables").await;
        assert!(matches!(
            app.database_explorer.state,
            DatabaseExplorerState::TableData(..)
        ));
        assert!(render_text(&mut app).contains("Draft row pending"));
    }

    fn render_sized(app: &mut App<'_>, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let buf = terminal.backend().buffer();
        buf.content()
            .chunks(usize::from(buf.area.width))
            .map(|row| row.iter().map(Cell::symbol).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn named_tables_app<'a>(tag: &str, tables: &[&str]) -> App<'a> {
        let mut app = sqlite_app(tag);
        app.database_explorer.tables = Some(FilteredData::new(
            tables
                .iter()
                .map(|n| Table {
                    name: (*n).to_string(),
                    schema: "sqlite_schema".into(),
                    size: None,
                })
                .collect::<Vec<_>>(),
        ));
        app
    }

    fn tab() -> KeyEvent {
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)
    }

    #[tokio::test]
    async fn table_command_args_errors_and_verb_named_tables() {
        let mut app = tables_app("cmd_table_arg").await;
        RusqliteConnection::open(&app.database_explorer.connection.url)
            .unwrap()
            .execute_batch(
                "CREATE TABLE log(id); CREATE TABLE act(id);
                 CREATE TABLE \"tables\"(id); CREATE TABLE \"my data\"(id);",
            )
            .unwrap();
        app.load_tables("sqlite_schema");
        drain_bg_work(&mut app).await;
        let open = |t: &str| {
            DatabaseExplorerState::TableData("sqlite_schema".into(), t.into())
        };
        for (cmd, table) in [
            ("table log", "log"),
            ("table act", "act"),
            ("table tables", "tables"),
            ("table my data", "my data"),
            ("table   MY DA  ", "my data"),
        ] {
            run_cmd(&mut app, "tables").await;
            run_cmd(&mut app, cmd).await;
            assert_eq!(app.database_explorer.state, open(table), "{cmd}");
        }
        // Bare verbs keep their meaning even if a table shares the name.
        run_cmd(&mut app, "log").await;
        assert!(app.show_query_log, "bare :log opens the query log");
        app.on_key_event(esc()).unwrap();
        run_cmd(&mut app, "tables").await;
        run_cmd(&mut app, "act").await;
        assert!(render_text(&mut app).contains("Postgres-only"));
        // A bare table name is no longer a command.
        run_cmd(&mut app, "users").await;
        assert!(render_text(&mut app).contains("Unknown command: users"));
        // `:table` alone == `:tables`; unknown table reports and stays put.
        run_cmd(&mut app, "table users").await;
        run_cmd(&mut app, "table").await;
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::Tables("sqlite_schema".into())
        );
        run_cmd(&mut app, "table nope").await;
        assert!(render_text(&mut app).contains("No table matching 'nope'"));
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::Tables("sqlite_schema".into())
        );
    }

    #[tokio::test]
    async fn table_argument_tab_completion_and_spaces() {
        let mut app = named_tables_app(
            "cmd_table_tab",
            &["orders", "Order Items", "log", "users"],
        );
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "table or");
        assert_eq!(
            app.command_suggestion().as_deref(),
            Some("table orders"),
            "best (first) prefix match"
        );
        app.on_key_event(tab()).unwrap();
        assert_eq!(app.command_text(), "table orders");
        assert_eq!(app.command_suggestion(), None);
        app.on_key_event(tab()).unwrap();
        assert_eq!(app.command_text(), "table orders", "Tab is idempotent");
        app.on_key_event(esc()).unwrap();
        // Spaces inside the argument.
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "table order i");
        app.on_key_event(tab()).unwrap();
        assert_eq!(app.command_text(), "table Order Items");
        app.on_key_event(esc()).unwrap();
        // Empty argument completes to the first table; non-table verbs
        // never get an argument completion.
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "table ");
        assert_eq!(app.command_suggestion().as_deref(), Some("table orders"));
        app.on_key_event(esc()).unwrap();
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "columns u");
        assert_eq!(app.command_suggestion(), None);
        app.on_key_event(esc()).unwrap();
        // No match: nothing to complete.
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "table zz");
        app.on_key_event(tab()).unwrap();
        assert_eq!(app.command_text(), "table zz");
    }

    #[tokio::test]
    async fn extra_whitespace_and_blank_commands() {
        let mut app = tables_app("cmd_ws").await;
        run_cmd(&mut app, "table users").await;
        run_cmd(&mut app, "  tables  ").await;
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::Tables("sqlite_schema".into())
        );
        // Blank / whitespace-only: bar closes, nothing else changes.
        for blank in ["", "   "] {
            app.set_status("");
            run_cmd(&mut app, blank).await;
            assert!(!app.command_mode && app.search_filter.is_none());
            assert_eq!(
                app.database_explorer.state,
                DatabaseExplorerState::Tables("sqlite_schema".into())
            );
            assert!(!render_text(&mut app).contains("Unknown command"));
        }
        // Uppercase + padding resolves; Tab cannot complete past whitespace.
        run_cmd(&mut app, "  table  USERS ").await;
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::TableData(
                "sqlite_schema".into(),
                "users".into()
            )
        );
    }

    #[tokio::test]
    async fn typing_in_command_bar_never_filters_and_esc_keeps_old_filter() {
        let mut app = tables_app("cmd_nofilter").await;
        let total = |a: &App<'_>| {
            a.database_explorer
                .tables
                .as_ref()
                .map(|t| t.table.model.items.len())
        };
        assert_eq!(total(&app), Some(4));
        // Commit a real filter first.
        app.on_key_event(key('/')).unwrap();
        type_str(&mut app, "users");
        app.on_key_event(enter()).unwrap();
        assert_eq!(total(&app), Some(1));
        assert!(app.has_active_filter());
        // Command typing must not touch it; Esc leaves it as it was.
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "zzz");
        assert_eq!(total(&app), Some(1), "no live filtering behind the bar");
        app.on_key_event(esc()).unwrap();
        assert!(app.has_active_filter());
        assert_eq!(total(&app), Some(1), "Esc must not clear/restore rows");
        // `/` after `:` is a normal filter bar again.
        app.on_key_event(key('/')).unwrap();
        assert!(!app.command_mode && app.search_filter.is_some());
        app.on_key_event(esc()).unwrap();
        assert_eq!(total(&app), Some(4), "/ Esc clears the filter as before");
        // Same on the connection list.
        let mut app = App::default();
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "zzz");
        assert!(app.command_mode);
        assert!(!app.has_active_filter());
        app.on_key_event(enter()).unwrap();
        assert!(!app.command_mode && app.search_filter.is_none());
        assert!(render_text(&mut app).contains("Unknown command: zzz"));
    }

    #[tokio::test]
    async fn command_flags_reset_on_every_exit_path() {
        let mut app = tables_app("cmd_reset").await;
        for exit in [esc(), enter()] {
            app.on_key_event(key(':')).unwrap();
            assert!(app.command_mode);
            app.on_key_event(exit).unwrap();
            assert!(!app.command_mode && app.search_filter.is_none());
        }
        // Enter on a failing command, on a view change, and on quit.
        for cmd in ["zzz", "users", "c", "q"] {
            app.running = true;
            run_cmd(&mut app, cmd).await;
            assert!(!app.command_mode && app.search_filter.is_none(), "{cmd}");
        }
        // After `:q` the next `/` is a filter, not a command bar.
        app.running = true;
        app.on_key_event(key('/')).unwrap();
        assert!(app.search_filter.is_some() && !app.command_mode);
    }

    #[tokio::test]
    async fn ctrl_j_does_not_create_a_second_line() {
        let mut app = tables_app("cmd_ctrlj").await;
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "us");
        for c in ['j', 'm'] {
            app.on_key_event(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::CONTROL,
            ))
            .unwrap();
        }
        type_str(&mut app, "ers");
        assert_eq!(app.command_text(), "users");
        assert_eq!(
            app.search_filter.as_ref().map(|t| t.lines().len()),
            Some(1)
        );
    }

    #[tokio::test]
    async fn unicode_commands_and_tables() {
        let mut app = named_tables_app(
            "cmd_unicode",
            &["表格", "📦boxes", "Order Items", "O'Brien"],
        );
        // Emoji / multibyte typing, Backspace and Tab never panic.
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "🦀é表");
        assert_eq!(app.command_text(), "🦀é表");
        assert_eq!(app.command_suggestion(), None);
        app.on_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.command_text(), "🦀é");
        app.on_key_event(tab()).unwrap();
        assert_eq!(app.command_text(), "🦀é", "nothing to complete");
        let _ = render_text(&mut app);
        app.on_key_event(enter()).unwrap();
        assert!(render_text(&mut app).contains("Unknown command: 🦀"));
        // Ghost for a wide-glyph table: typed is 2 cells wide per glyph.
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "表");
        assert_eq!(
            app.command_suggestion(),
            None,
            "verbs only before `table `"
        );
        type_str(&mut app, "x");
        app.on_key_event(esc()).unwrap();
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "table 表");
        assert_eq!(app.command_suggestion().as_deref(), Some("table 表格"));
        let text = render_text(&mut app);
        assert!(text.contains("格"), "{text}");
        app.on_key_event(tab()).unwrap();
        assert_eq!(app.command_text(), "table 表格");
        app.on_key_event(esc()).unwrap();
        // Spaces / quotes / case in table names.
        for (typed, shown) in [
            ("table order i", "table Order Items"),
            ("table O'B", "table O'Brien"),
            ("table 📦", "table 📦boxes"),
        ] {
            app.on_key_event(key(':')).unwrap();
            type_str(&mut app, typed);
            app.on_key_event(tab()).unwrap();
            assert_eq!(app.command_text(), shown);
            app.on_key_event(esc()).unwrap();
        }
    }

    #[tokio::test]
    async fn bar_renders_without_panic_at_tiny_and_long_input_sizes() {
        let long = "a_very_long_table_name_that_overflows_a_small_bar";
        let mut app = named_tables_app("cmd_narrow", &[long, "表格"]);
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "table a_v");
        for (w, h) in [(20, 5), (80, 24), (120, 30), (6, 3), (3, 3), (1, 1)] {
            let _ = render_sized(&mut app, w, h);
        }
        let text = render_sized(&mut app, 80, 24);
        assert!(
            text.contains(long),
            "long name fits at 80 cols (not Fit-soft-capped)"
        );
        // Ghost is clipped, not wrapped, in a 20x5 terminal.
        let text = render_sized(&mut app, 20, 5);
        assert!(!text.contains(&long[3..]));
        // Typed text wider than the bar: ghost vanishes, no panic.
        type_str(&mut app, &"x".repeat(300));
        for (w, h) in [(20, 5), (80, 24), (5, 3)] {
            let _ = render_sized(&mut app, w, h);
        }
        // Wide-glyph input at the narrow sizes.
        app.on_key_event(esc()).unwrap();
        app.on_key_event(key(':')).unwrap();
        type_str(&mut app, "table 表");
        for (w, h) in [(20, 5), (7, 3), (4, 3)] {
            let _ = render_sized(&mut app, w, h);
        }
    }

    #[tokio::test]
    async fn row_jump_edges_and_draft_guard() {
        let mut app = tables_app("cmd_rows").await;
        run_cmd(&mut app, "table users").await;
        for (cmd, want) in [
            ("0", "Row number must be >= 1"),
            ("99999999999999999999", "Unknown command"),
            ("-1", "Unknown command"),
            ("4", "beyond loaded page"),
        ] {
            run_cmd(&mut app, cmd).await;
            assert!(render_text(&mut app).contains(want), "{cmd}");
        }
        run_cmd(&mut app, "3").await;
        assert!(render_text(&mut app).contains("Jumped to row 3"));
        // Row jump would replace the page and drop the draft: refuse.
        app.on_key_event(key('n')).unwrap();
        app.on_key_event(esc()).unwrap();
        assert!(app.has_table_draft_rows());
        run_cmd(&mut app, "1").await;
        assert!(app.has_table_draft_rows(), "draft survives :1");
        assert!(render_text(&mut app).contains("Draft row pending"));
        // Non-navigating commands remain allowed with a draft.
        run_cmd(&mut app, "help").await;
        assert!(app.show_help);
    }

    #[tokio::test]
    async fn colon_is_inert_in_describe_query_log_and_ignored_while_disconnected()
     {
        let mut app = tables_app("cmd_overlays").await;
        for flag in 0..2 {
            if flag == 0 {
                app.show_describe = true;
            } else {
                app.show_describe = false;
                app.show_query_log = true;
            }
            app.on_key_event(key(':')).unwrap();
            assert!(app.search_filter.is_none() && !app.command_mode);
        }
        app.show_query_log = false;
        // Disconnected: bar opens, view commands are refused politely.
        let mut app = App::default();
        run_cmd(&mut app, "schemas").await;
        assert!(render_text(&mut app).contains("Not connected"));
        run_cmd(&mut app, "5").await;
        assert!(render_text(&mut app).contains("Not connected"));
        run_cmd(&mut app, "conn").await;
        assert!(render_text(&mut app).contains("Already on the connections"));
    }

    #[tokio::test]
    async fn conn_command_stops_watch_and_returns_to_list() {
        let mut app = tables_app("cmd_conn").await;
        app.database_explorer.state =
            DatabaseExplorerState::SqlResults("select 1".into());
        app.watch_active = true;
        run_cmd(&mut app, "conn").await;
        assert_eq!(app.state, AppState::ConnectionList);
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::Connections
        );
        // The watch loop turns itself off once the view left SQL results.
        app.tick_watch();
        assert!(!app.watch_active);
        assert!(!app.command_mode);
    }

    #[tokio::test]
    async fn hotkey_bar_shows_command_hint_at_120_cols() {
        let mut app = tables_app("cmd_hotkeys").await;
        app.hotkeys = DATABASE_HOTKEYS.to_vec();
        let text = render_sized(&mut app, 120, 30);
        assert!(text.contains("<:> Command"), "{text}");
        assert!(text.contains("Query log"), "{text}");
    }

    /// Needs `just docker-up` (Postgres on localhost:5432):
    /// `cargo test -p d7s -- --ignored pg_command`
    #[tokio::test]
    #[ignore = "requires docker compose Postgres"]
    async fn pg_command_schemas_and_activity() {
        let pg = Postgres {
            name: "it".into(),
            host: Some("localhost".into()),
            port: Some("5432".into()),
            user: "d7s_user".into(),
            database: "d7s_test".into(),
            password: "d7s_password".into(),
            tls: TlsConfig::default(),
        };
        let mut app = App::default();
        app.database_explorer = DatabaseExplorer::new(
            Connection {
                name: "it".into(),
                r#type: ConnectionType::Postgres,
                ..Connection::default()
            },
            Some(Arc::from(app.query_log.wrap(Box::new(pg)))),
        );
        app.state = AppState::DatabaseConnected;
        app.database_explorer.state = DatabaseExplorerState::Databases;
        run_cmd(&mut app, "schemas").await;
        assert_eq!(app.database_explorer.state, DatabaseExplorerState::Schemas);
        run_cmd(&mut app, "activity").await;
        assert!(matches!(
            app.database_explorer.state,
            DatabaseExplorerState::SqlResults(_)
        ));
        assert!(
            app.query_log
                .snapshot()
                .iter()
                .any(|e| e.origin == QueryOrigin::Activity)
        );
        run_cmd(&mut app, "conn").await;
        assert_eq!(app.state, AppState::ConnectionList);
    }

    #[test]
    fn toggle_column_width_mode_syncs_all_tables() {
        let mut app = App::default();
        assert_eq!(app.column_width_mode, ColumnWidthMode::Fit);
        app.toggle_column_width_mode();
        assert_eq!(app.column_width_mode, ColumnWidthMode::Content);
        assert_eq!(app.status_line.message(), "Column width: Content");
        assert_eq!(
            app.help_table.view.column_width_mode,
            ColumnWidthMode::Content
        );
        assert_eq!(
            app.database_explorer
                .sql_executor
                .table_state
                .view
                .column_width_mode,
            ColumnWidthMode::Content
        );
        app.toggle_column_width_mode();
        assert_eq!(
            app.database_explorer
                .connections
                .table
                .view
                .column_width_mode,
            ColumnWidthMode::Fit
        );
    }
}
