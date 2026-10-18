use std::{
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

use color_eyre::Result;
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
    table::{TableData, TableDataState},
};
use ratatui::DefaultTerminal;
use ratatui_textarea::TextArea;

use crate::{
    app_state::{AppState, DatabaseExplorerState},
    database_explorer_state::DatabaseExplorer,
    db::{
        RowDeleteSpec,
        query_log::{QueryLog, QueryLogEntry},
        sqlite::init_db,
    },
    filtered_data::FilteredData,
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
}

impl Default for App<'_> {
    fn default() -> Self {
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
            showed_help_hint: false,
            watch_active: false,
            watch_last_tick: Instant::now(),
            query_log: QueryLog::default(),
            show_query_log: false,
            query_log_table: TableDataState::new(Vec::new()),
        }
    }
}

impl App<'_> {
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
    pub async fn run(&mut self, mut terminal: DefaultTerminal) -> Result<()> {
        self.running = true;
        self.ping_connections_if_any().await;
        while self.running {
            terminal.draw(|frame| self.render(frame))?;
            self.tick_watch().await;
            self.handle_crossterm_events().await?;

            self.handle_external_terminal(&mut terminal).await?;
        }
        self.save_session_preferences();
        Ok(())
    }

    /// Re-run the SQL results query if watch mode is on and the interval
    /// has elapsed. Turns itself off if the view has moved away from
    /// SQL results (e.g. the user pressed Esc back to the connection tree).
    async fn tick_watch(&mut self) {
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
        self.execute_sql_query_watch_tick().await;
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

    async fn handle_external_terminal(
        &mut self,
        terminal: &mut DefaultTerminal,
    ) -> Result<(), color_eyre::eyre::Error> {
        if self.open_editor_requested {
            self.open_editor_requested = false;
            let new_sql = if std::env::var("D7S_DEMO").is_ok() {
                std::env::var("D7S_DEMO_SQL")
                    .ok()
                    .and_then(|path| std::fs::read_to_string(path).ok())
                    .unwrap_or_default()
            } else {
                let temp_path = std::env::temp_dir()
                    .join(format!("d7s_sql_{}.sql", std::process::id()));
                let current_sql =
                    self.database_explorer.sql_executor.sql_input();
                std::fs::write(&temp_path, &current_sql)?;
                Self::run_editor(terminal, &temp_path)?;
                std::fs::read_to_string(&temp_path).unwrap_or_default()
            };
            self.apply_editor_sql(new_sql).await;
        }

        Ok(())
    }

    async fn apply_editor_sql(&mut self, new_sql: String) {
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
                self.prepare_sql_statement_execution(statement.text.clone())
                    .await;
            }
        } else {
            let options =
                statements.into_iter().map(|s| s.text).collect::<Vec<_>>();
            self.modal_manager.open_sql_query_selection_modal(options);
        }
    }

    /// Reload saved connections from disk and re-probe health.
    /// Status is runtime-only, so a reload without a ping would blank the column.
    pub(crate) async fn refresh_connections(&mut self) {
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
        self.ping_connections_if_any().await;
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
                std::io::stdout(),
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

    /// Build the field/value rows describing whatever object is currently
    /// selected (connection, table, column, or table-data row).
    ///
    /// Never spawned onto another task, so the returned future not being
    /// `Send` (due to interior-mutability fields on `App`) is harmless.
    #[allow(clippy::future_not_send, clippy::too_many_lines)]
    pub(crate) async fn build_describe_rows(&self) -> Vec<DescribeRow> {
        let explorer = &self.database_explorer;
        match &explorer.state {
            DatabaseExplorerState::Connections => explorer
                .connections
                .table
                .view
                .state
                .selected()
                .and_then(|i| explorer.connections.table.model.items.get(i))
                .map_or_else(Vec::new, connection_describe_rows),
            DatabaseExplorerState::Tables(_) => {
                let Some(table) = explorer.tables.as_ref().and_then(|t| {
                    let i = t.table.view.state.selected()?;
                    t.table.model.items.get(i).cloned()
                }) else {
                    return Vec::new();
                };
                let mut rows = vec![
                    DescribeRow::section("Table"),
                    DescribeRow::new("Name", &table.name),
                    DescribeRow::new("Schema", &table.schema),
                ];
                if let Some(db) = explorer.database.as_ref() {
                    let (count_res, cols_res, pk_res, indexes_res, size_res) = tokio::join!(
                        db.get_table_row_count(&table.schema, &table.name),
                        db.get_columns(&table.schema, &table.name),
                        db.get_primary_key_columns(&table.schema, &table.name),
                        db.get_table_index_names(&table.schema, &table.name),
                        db.get_table_size(&table.schema, &table.name),
                    );
                    if let Ok(count) = count_res {
                        rows.push(DescribeRow::new(
                            "Row count",
                            count.to_string(),
                        ));
                    }
                    if let Ok(cols) = cols_res {
                        rows.push(DescribeRow::new(
                            "Columns",
                            cols.len().to_string(),
                        ));
                    }
                    if let Ok(pk) = pk_res
                        && !pk.is_empty()
                    {
                        rows.push(DescribeRow::new(
                            "Primary key",
                            pk.join(", "),
                        ));
                    }
                    if let Ok(indexes) = indexes_res
                        && !indexes.is_empty()
                    {
                        rows.push(DescribeRow::new(
                            "Indexes",
                            indexes.join(", "),
                        ));
                    }
                    let size =
                        size_res.ok().flatten().or_else(|| table.size.clone());
                    if let Some(size) = size {
                        rows.push(DescribeRow::new("Size", size));
                    }
                } else if let Some(size) = table.size.clone() {
                    rows.push(DescribeRow::new("Size", size));
                }
                rows
            }
            DatabaseExplorerState::Columns(schema, table_name) => {
                let Some(col) = explorer.columns.as_ref().and_then(|c| {
                    let i = c.table.view.state.selected()?;
                    c.table.model.items.get(i).cloned()
                }) else {
                    return Vec::new();
                };
                let mut rows = vec![
                    DescribeRow::section("Column"),
                    DescribeRow::new("Table", format!("{schema}.{table_name}")),
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
                if let Some(db) = explorer.database.as_ref()
                    && let Ok(pk) =
                        db.get_primary_key_columns(schema, table_name).await
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
            DatabaseExplorerState::TableData(schema, table_name) => explorer
                .table_data
                .as_ref()
                .and_then(|t| {
                    let i = t.table.view.state.selected()?;
                    let row = t.table.model.items.get(i)?;
                    let names = t.table.model.dynamic_column_names.as_ref();
                    Some((row, names))
                })
                .map_or_else(Vec::new, |(row, names)| {
                    let mut rows = vec![
                        DescribeRow::section("Row"),
                        DescribeRow::new(
                            "Table",
                            format!("{schema}.{table_name}"),
                        ),
                    ];
                    for (idx, value) in row.values.iter().enumerate() {
                        let field = names
                            .and_then(|n| n.get(idx))
                            .cloned()
                            .unwrap_or_else(|| format!("col{idx}"));
                        rows.push(DescribeRow::new(field, value.clone()));
                    }
                    rows
                }),
            DatabaseExplorerState::Databases
            | DatabaseExplorerState::Schemas
            | DatabaseExplorerState::SqlResults(_) => Vec::new(),
        }
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
                std::io::stdout(),
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
        let path = std::env::temp_dir()
            .join(format!("d7s_results_{}.tsv", std::process::id()));
        match std::fs::write(&path, out) {
            Ok(()) => self.set_status(format!("Wrote {}", path.display())),
            Err(e) => self.set_status(format!("Export failed: {e}")),
        }
    }

    /// Set running to false to quit the application.
    pub(crate) const fn quit(&mut self) {
        self.running = false;
    }

    /// Set the status line message
    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status_line.set_message(message);
    }

    /// Clear the status line
    pub fn clear_status(&mut self) {
        self.status_line.clear();
    }

    pub(crate) fn connect_status_message(
        &mut self,
        name: &str,
        env: crate::db::connection::Environment,
    ) -> String {
        if self.showed_help_hint {
            format!("Connected to {name} ({env})")
        } else {
            self.showed_help_hint = true;
            format!("Connected to {name} ({env}) — press ? for help")
        }
    }

    fn run_editor(terminal: &mut DefaultTerminal, path: &Path) -> Result<()> {
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vim".to_string());
        let (program, args) = Self::parse_editor_command(&editor);

        execute!(std::io::stdout(), DisableBracketedPaste)?;
        std::io::stdout().execute(LeaveAlternateScreen)?;
        disable_raw_mode()?;
        let mut cmd = Command::new(&program);
        cmd.args(args);
        cmd.arg(path).status()?;
        std::io::stdout().execute(EnterAlternateScreen)?;
        enable_raw_mode()?;
        execute!(std::io::stdout(), EnableBracketedPaste)?;
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

    pub(crate) async fn prepare_sql_statement_execution(
        &mut self,
        statement: String,
    ) {
        if classify_statement(&statement)
            == StatementSafety::RequiresConfirmation
        {
            self.modal_manager
                .open_sql_execution_confirmation_modal(statement);
        } else {
            self.execute_sql_statement_now(statement).await;
        }
    }

    pub(crate) async fn execute_sql_statement_now(
        &mut self,
        statement: String,
    ) {
        self.enter_sql_results_state(statement.clone());
        self.database_explorer
            .sql_executor
            .set_selected_statement(statement);
        self.execute_sql_query().await;
    }

    /// `L`: show every query d7s ran this session, newest first.
    pub(crate) fn open_query_log_view(&mut self) {
        self.query_log_table = TableDataState::new(self.query_log.snapshot());
        self.show_query_log = true;
    }

    /// `A`: show currently-running Postgres backends via `pg_stat_activity`.
    /// SQLite is an embedded/file engine with no server process to inspect,
    /// so there's nothing equivalent to show there.
    pub(crate) async fn open_activity_view(&mut self) {
        if !self.database_explorer.connection.r#type.has_activity_view() {
            self.set_status(
                "Activity view is Postgres-only (pg_stat_activity has no equivalent here).",
            );
            return;
        }
        self.execute_sql_statement_now(ACTIVITY_QUERY.to_string())
            .await;
    }
}

/// Turn a connection's existing `Display` output ("` Field: value`" per line)
/// into describe rows, reusing the formatting already defined for it.
fn connection_describe_rows(
    conn: &crate::db::connection::Connection,
) -> Vec<DescribeRow> {
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
        format!("Name: {}", crate::app::PKG_NAME),
        format!("Version: {}", crate::app::PKG_VERSION),
    ];
    if std::env::var("D7S_DEMO").is_err() {
        let path_buf = std::env::current_dir()?;
        let cwd = path_buf.as_path().to_str().unwrap_or(".");
        lines.push(format!(
            "Path: {}",
            crate::db::connection::shorten_home_path(cwd)
        ));
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::db::{
        connection::{Connection, ConnectionStatus, ConnectionType},
        query_log::{QueryLogEntry, QueryOrigin, tests::temp_sqlite},
    };

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    /// App connected to a throwaway `SQLite` file, wired through the log.
    fn sqlite_app<'a>(tag: &str) -> App<'a> {
        let sqlite = temp_sqlite(tag);
        rusqlite::Connection::open(&sqlite.path)
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
        let db = app.query_log.wrap(Box::new(sqlite));
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
            .map(|row| {
                row.iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
            })
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
        app.on_key_event(key('L')).await.unwrap();

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

        app.on_key_event(key('L')).await.unwrap();
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
            .await
            .unwrap();
        assert!(!app.show_query_log);
        assert!(!render_text(&mut app).contains("Origin"));

        // `q` also returns (and does not quit)
        app.running = true;
        app.on_key_event(key('L')).await.unwrap();
        app.on_key_event(key('q')).await.unwrap();
        assert!(!app.show_query_log && app.running);
    }

    #[tokio::test]
    async fn activity_key_is_ignored_on_sqlite() {
        use crate::{
            db::connection::ConnectionType,
            ui::widgets::hotkeys::database_hotkeys,
        };
        let mut app = sqlite_app("noact");
        app.on_key_event(key('A')).await.unwrap();
        assert!(app.query_log.snapshot().is_empty(), "nothing may run");
        assert!(matches!(
            app.database_explorer.state,
            DatabaseExplorerState::Tables(_)
        ));
        assert!(!render_text(&mut app).contains("Postgres-only"));

        let has_activity = |t| {
            database_hotkeys(t)
                .iter()
                .any(|h| h.keycode == crossterm::event::KeyCode::Char('A'))
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
        app.on_key_event(key('w')).await.unwrap();
        assert!(!app.watch_active, "w is inert outside SQL results");

        app.enter_sql_results_state("SELECT a FROM t".into());
        app.on_key_event(key('w')).await.unwrap();
        assert!(app.watch_active);
        // The toggle's own status message temporarily replaces the idle hint.
        assert!(render_text(&mut app).contains("Watching (every 2s)"));
        app.clear_status();
        assert!(render_text(&mut app).contains("WATCHING"));

        app.on_key_event(key('w')).await.unwrap();
        assert!(!app.watch_active);
        assert!(!render_text(&mut app).contains("WATCHING"));
    }

    #[tokio::test]
    async fn watch_tick_is_logged_but_not_added_to_sql_history() {
        let mut app = sqlite_app("watchtick");
        app.enter_sql_results_state("SELECT a FROM t".into());
        app.database_explorer
            .sql_executor
            .set_selected_statement("SELECT a FROM t");
        let history_before = PreferencesService::load_sql_history();

        app.execute_sql_query_watch_tick().await;

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
        rusqlite::Connection::open(&app.database_explorer.connection.url)
            .unwrap()
            .execute_batch(
                "CREATE TABLE users(id); CREATE TABLE orders(id);
                 CREATE TABLE order_items(id);
                 INSERT INTO users VALUES (1),(2),(3);",
            )
            .unwrap();
        app.load_tables("sqlite_schema").await.unwrap();
        app
    }

    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }

    #[allow(clippy::future_not_send)]
    async fn type_str(app: &mut App<'_>, s: &str) {
        for c in s.chars() {
            app.on_key_event(key(c)).await.unwrap();
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
        std::fs::write(&present.path, b"").unwrap();
        let missing = temp_sqlite("ping-missing");
        // A port that was just free, so nothing is listening on it.
        let closed_port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
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
        app.on_key_event(key('p')).await.unwrap();

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
            !std::path::Path::new(&missing.path).exists(),
            "ping must not create a missing sqlite file"
        );

        app.database_explorer.connections.clear_filter();
        let text = render_text(&mut app);
        assert!(text.contains("Status"));
        assert!(text.contains("● up"));
        assert_eq!(text.matches("● down").count(), 2);
        let _ = std::fs::remove_file(&present.path);
    }

    #[tokio::test]
    async fn ping_if_any_skips_when_no_connections() {
        let mut app = App::default();
        app.ping_connections_if_any().await;
        assert!(app.database_explorer.connections.original.is_empty());
    }

    #[tokio::test]
    async fn ping_if_any_sets_statuses() {
        let present = temp_sqlite("launch-ping-present");
        std::fs::write(&present.path, b"").unwrap();
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

        app.ping_connections_if_any().await;

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
        let _ = std::fs::remove_file(&present.path);
    }

    #[tokio::test]
    async fn refresh_wiping_status_is_restored_by_reprobe() {
        // refresh_connections replaces rows from disk (Unknown status). Simulate
        // that wipe, then the same reprobe refresh ends with.
        let present = temp_sqlite("refresh-ping-present");
        std::fs::write(&present.path, b"").unwrap();
        let mut app = App::default();
        app.database_explorer.connections =
            FilteredData::new(vec![sqlite_conn("present", &present.path)]);
        app.ping_connections_if_any().await;
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
        app.ping_connections_if_any().await;
        assert_eq!(status(&app), Some(ConnectionStatus::Up));
        let _ = std::fs::remove_file(&present.path);
    }

    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }

    #[allow(clippy::future_not_send)]
    async fn run_cmd(app: &mut App<'_>, cmd: &str) {
        app.on_key_event(key(':')).await.unwrap();
        type_str(app, cmd).await;
        app.on_key_event(enter()).await.unwrap();
    }

    #[tokio::test]
    async fn colon_bar_types_completes_and_executes() {
        let mut app = tables_app("cmd_flow").await;
        app.on_key_event(key(':')).await.unwrap();
        assert!(app.command_mode && app.search_filter.is_some());
        type_str(&mut app, "tab").await;
        assert_eq!(app.command_suggestion().as_deref(), Some("table "));
        app.on_key_event(tab()).await.unwrap();
        assert_eq!(app.command_text(), "table ");
        type_str(&mut app, "us").await;
        assert_eq!(app.command_suggestion().as_deref(), Some("table users"));
        let text = render_text(&mut app);
        assert!(text.contains("users"), "ghost completion rendered");
        app.on_key_event(tab()).await.unwrap();
        assert_eq!(app.command_text(), "table users");
        app.on_key_event(enter()).await.unwrap();
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
    async fn colon_bar_renders_prompt_and_backspace_edits() {
        let mut app = tables_app("cmd_render").await;
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "xq").await;
        app.on_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
            .await
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
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "table u").await;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let buf = terminal.backend().buffer();
        let rows: Vec<&[ratatui::buffer::Cell]> =
            buf.content().chunks(usize::from(buf.area.width)).collect();
        let title = rows
            .iter()
            .position(|r| {
                r.iter()
                    .map(ratatui::buffer::Cell::symbol)
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
        assert!(
            after(8)
                .modifier
                .contains(ratatui::style::Modifier::REVERSED)
        );
    }

    #[tokio::test]
    async fn colon_bar_esc_cancels_and_ctrl_c_quits() {
        let mut app = tables_app("cmd_esc").await;
        app.running = true;
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "users").await;
        app.on_key_event(esc()).await.unwrap();
        assert!(!app.command_mode && app.search_filter.is_none());
        assert_eq!(
            app.database_explorer.state,
            DatabaseExplorerState::Tables("sqlite_schema".into())
        );
        app.on_key_event(key(':')).await.unwrap();
        app.on_key_event(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ))
        .await
        .unwrap();
        assert!(!app.running);
    }

    #[tokio::test]
    async fn colon_is_ignored_in_inputs_modals_and_overlays() {
        let mut app = tables_app("cmd_ignored").await;
        // Inside the `/` filter bar it is just text.
        app.on_key_event(key('/')).await.unwrap();
        app.on_key_event(key(':')).await.unwrap();
        assert!(!app.command_mode);
        assert_eq!(app.command_text(), ":");
        app.on_key_event(esc()).await.unwrap();
        // Help overlay.
        app.on_key_event(key('?')).await.unwrap();
        app.on_key_event(key(':')).await.unwrap();
        assert!(app.search_filter.is_none() && app.show_help);
        app.on_key_event(esc()).await.unwrap();
        // Modal.
        app.modal_manager.open_new_connection_modal();
        app.on_key_event(key(':')).await.unwrap();
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

    #[tokio::test]
    async fn draft_row_blocks_navigation_commands() {
        let mut app = tables_app("cmd_draft").await;
        run_cmd(&mut app, "table users").await;
        app.on_key_event(key('a')).await.unwrap();
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
            .map(|row| {
                row.iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn named_tables_app<'a>(tag: &str, tables: &[&str]) -> App<'a> {
        let mut app = sqlite_app(tag);
        app.database_explorer.tables = Some(FilteredData::new(
            tables
                .iter()
                .map(|n| crate::db::Table {
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
        rusqlite::Connection::open(&app.database_explorer.connection.url)
            .unwrap()
            .execute_batch(
                "CREATE TABLE log(id); CREATE TABLE act(id);
                 CREATE TABLE \"tables\"(id); CREATE TABLE \"my data\"(id);",
            )
            .unwrap();
        app.load_tables("sqlite_schema").await.unwrap();
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
        app.on_key_event(esc()).await.unwrap();
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
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "table or").await;
        assert_eq!(
            app.command_suggestion().as_deref(),
            Some("table orders"),
            "best (first) prefix match"
        );
        app.on_key_event(tab()).await.unwrap();
        assert_eq!(app.command_text(), "table orders");
        assert_eq!(app.command_suggestion(), None);
        app.on_key_event(tab()).await.unwrap();
        assert_eq!(app.command_text(), "table orders", "Tab is idempotent");
        app.on_key_event(esc()).await.unwrap();
        // Spaces inside the argument.
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "table order i").await;
        app.on_key_event(tab()).await.unwrap();
        assert_eq!(app.command_text(), "table Order Items");
        app.on_key_event(esc()).await.unwrap();
        // Empty argument completes to the first table; non-table verbs
        // never get an argument completion.
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "table ").await;
        assert_eq!(app.command_suggestion().as_deref(), Some("table orders"));
        app.on_key_event(esc()).await.unwrap();
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "columns u").await;
        assert_eq!(app.command_suggestion(), None);
        app.on_key_event(esc()).await.unwrap();
        // No match: nothing to complete.
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "table zz").await;
        app.on_key_event(tab()).await.unwrap();
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
        app.on_key_event(key('/')).await.unwrap();
        type_str(&mut app, "users").await;
        app.on_key_event(enter()).await.unwrap();
        assert_eq!(total(&app), Some(1));
        assert!(app.has_active_filter());
        // Command typing must not touch it; Esc leaves it as it was.
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "zzz").await;
        assert_eq!(total(&app), Some(1), "no live filtering behind the bar");
        app.on_key_event(esc()).await.unwrap();
        assert!(app.has_active_filter());
        assert_eq!(total(&app), Some(1), "Esc must not clear/restore rows");
        // `/` after `:` is a normal filter bar again.
        app.on_key_event(key('/')).await.unwrap();
        assert!(!app.command_mode && app.search_filter.is_some());
        app.on_key_event(esc()).await.unwrap();
        assert_eq!(total(&app), Some(4), "/ Esc clears the filter as before");
        // Same on the connection list.
        let mut app = App::default();
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "zzz").await;
        assert!(app.command_mode);
        assert!(!app.has_active_filter());
        app.on_key_event(enter()).await.unwrap();
        assert!(!app.command_mode && app.search_filter.is_none());
        assert!(render_text(&mut app).contains("Unknown command: zzz"));
    }

    #[tokio::test]
    async fn command_flags_reset_on_every_exit_path() {
        let mut app = tables_app("cmd_reset").await;
        for exit in [esc(), enter()] {
            app.on_key_event(key(':')).await.unwrap();
            assert!(app.command_mode);
            app.on_key_event(exit).await.unwrap();
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
        app.on_key_event(key('/')).await.unwrap();
        assert!(app.search_filter.is_some() && !app.command_mode);
    }

    #[tokio::test]
    async fn ctrl_j_does_not_create_a_second_line() {
        let mut app = tables_app("cmd_ctrlj").await;
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "us").await;
        for c in ['j', 'm'] {
            app.on_key_event(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::CONTROL,
            ))
            .await
            .unwrap();
        }
        type_str(&mut app, "ers").await;
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
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "🦀é表").await;
        assert_eq!(app.command_text(), "🦀é表");
        assert_eq!(app.command_suggestion(), None);
        app.on_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
            .await
            .unwrap();
        assert_eq!(app.command_text(), "🦀é");
        app.on_key_event(tab()).await.unwrap();
        assert_eq!(app.command_text(), "🦀é", "nothing to complete");
        let _ = render_text(&mut app);
        app.on_key_event(enter()).await.unwrap();
        assert!(render_text(&mut app).contains("Unknown command: 🦀"));
        // Ghost for a wide-glyph table: typed is 2 cells wide per glyph.
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "表").await;
        assert_eq!(
            app.command_suggestion(),
            None,
            "verbs only before `table `"
        );
        type_str(&mut app, "x").await;
        app.on_key_event(esc()).await.unwrap();
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "table 表").await;
        assert_eq!(app.command_suggestion().as_deref(), Some("table 表格"));
        let text = render_text(&mut app);
        assert!(text.contains("格"), "{text}");
        app.on_key_event(tab()).await.unwrap();
        assert_eq!(app.command_text(), "table 表格");
        app.on_key_event(esc()).await.unwrap();
        // Spaces / quotes / case in table names.
        for (typed, shown) in [
            ("table order i", "table Order Items"),
            ("table O'B", "table O'Brien"),
            ("table 📦", "table 📦boxes"),
        ] {
            app.on_key_event(key(':')).await.unwrap();
            type_str(&mut app, typed).await;
            app.on_key_event(tab()).await.unwrap();
            assert_eq!(app.command_text(), shown);
            app.on_key_event(esc()).await.unwrap();
        }
    }

    #[tokio::test]
    async fn bar_renders_without_panic_at_tiny_and_long_input_sizes() {
        let long = "a_very_long_table_name_that_overflows_a_small_bar";
        let mut app = named_tables_app("cmd_narrow", &[long, "表格"]);
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "table a_v").await;
        for (w, h) in [(20, 5), (80, 24), (120, 30), (6, 3), (3, 3), (1, 1)] {
            let _ = render_sized(&mut app, w, h);
        }
        let text = render_sized(&mut app, 80, 24);
        assert!(text.contains(long), "ghost fits at 80 cols");
        // Ghost is clipped, not wrapped, in a 20x5 terminal.
        let text = render_sized(&mut app, 20, 5);
        assert!(!text.contains(long));
        // Typed text wider than the bar: ghost vanishes, no panic.
        type_str(&mut app, &"x".repeat(300)).await;
        for (w, h) in [(20, 5), (80, 24), (5, 3)] {
            let _ = render_sized(&mut app, w, h);
        }
        // Wide-glyph input at the narrow sizes.
        app.on_key_event(esc()).await.unwrap();
        app.on_key_event(key(':')).await.unwrap();
        type_str(&mut app, "table 表").await;
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
        app.on_key_event(key('a')).await.unwrap();
        assert!(app.has_table_draft_rows());
        run_cmd(&mut app, "1").await;
        assert!(app.has_table_draft_rows(), "draft survives :N");
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
            app.on_key_event(key(':')).await.unwrap();
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
        app.tick_watch().await;
        assert!(!app.watch_active);
        assert!(!app.command_mode);
    }

    #[tokio::test]
    async fn hotkey_bar_shows_command_hint_at_120_cols() {
        let mut app = tables_app("cmd_hotkeys").await;
        app.hotkeys = crate::ui::widgets::hotkeys::DATABASE_HOTKEYS.to_vec();
        let text = render_sized(&mut app, 120, 30);
        assert!(text.contains("<:> Command"), "{text}");
        assert!(text.contains("Query log"), "{text}");
    }

    /// Needs `just docker-up` (Postgres on localhost:5432):
    /// `cargo test -p d7s -- --ignored pg_command`
    #[tokio::test]
    #[ignore = "requires docker compose Postgres"]
    async fn pg_command_schemas_and_activity() {
        let pg = crate::db::postgres::Postgres {
            name: "it".into(),
            host: Some("localhost".into()),
            port: Some("5432".into()),
            user: "d7s_user".into(),
            database: "d7s_test".into(),
            password: "d7s_password".into(),
            tls: crate::db::tls::TlsConfig::default(),
        };
        let mut app = App::default();
        app.database_explorer = DatabaseExplorer::new(
            Connection {
                name: "it".into(),
                r#type: ConnectionType::Postgres,
                ..Connection::default()
            },
            Some(app.query_log.wrap(Box::new(pg))),
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
}
