use core::fmt;

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent};
use k9tui::widgets::{
    navigation::TableNavigationHandler, table::TableDataState,
};

use crate::{
    app::{ACTIVITY_QUERY, App},
    app_state::DatabaseExplorerState,
    db::{
        Database, DbRowId, TableDataPage, connection::ConnectionType,
        query_log::QueryOrigin,
    },
    filtered_data::FilteredData,
    services::PreferencesService,
    ui::widgets::{
        connection_modal::CellValueApply, raw_table::RawTableStateExt,
    },
    virtual_table::VirtualTableMeta,
};

const STATUS_DB_NOT_CONNECTED: &str = "Not connected to database.";
const STATUS_CONNECT_FAILED: &str = "Failed to connect to database.";

fn column_index_for_name(names: &[String], name: &str) -> Option<usize> {
    names
        .iter()
        .position(|c| c == name)
        .or_else(|| names.iter().position(|c| c.eq_ignore_ascii_case(name)))
}

macro_rules! explorer_selected_row_name {
    ($fd:expr) => {{
        let fd = $fd.as_ref()?;
        let i = fd.table.view.state.selected()?;
        Some(fd.table.model.items.get(i)?.name.clone())
    }};
}

impl App<'_> {
    pub(crate) fn status_load_failed<E: fmt::Display>(
        &mut self,
        resource: &str,
        err: E,
    ) {
        self.set_status(format!("Failed to load {resource}: {err}"));
    }

    pub(crate) fn status_action_failed<E: fmt::Display>(
        &mut self,
        action: &str,
        err: E,
    ) {
        self.set_status(format!("{action} failed: {err}"));
    }

    /// Replace the current table grid + virtual scroll metadata from a fetched page.
    pub(crate) fn replace_explorer_table_page(
        &mut self,
        page: TableDataPage,
        window_start: u64,
        page_size: u32,
        total_rows: Option<u64>,
    ) {
        let explorer = &mut self.database_explorer;
        let TableDataPage {
            rows: data,
            column_names,
            row_ids,
        } = page;
        let loaded = data.len();
        let meta = VirtualTableMeta::from_fetch(
            window_start,
            page_size,
            loaded,
            total_rows,
        );
        let mut table_state = TableDataState::default();
        table_state.reset(data, &column_names, Some(row_ids));
        explorer.table_data = Some(FilteredData {
            original: table_state.model.items.clone(),
            table: table_state,
        });
        explorer.table_data_virtual = Some(meta);
    }

    /// Load databases from the connection
    pub async fn load_databases(&mut self) -> Result<()> {
        let explorer = &mut self.database_explorer;

        let Some(database) = explorer.database.as_mut() else {
            self.set_status(STATUS_DB_NOT_CONNECTED);
            return Ok(());
        };

        match database.get_databases().await {
            Ok(databases) => {
                explorer.databases = Some(FilteredData::new(databases));
                explorer.state = DatabaseExplorerState::Databases;
            }
            Err(e) => self.status_load_failed("databases", e),
        }

        Ok(())
    }

    /// Select a database and reconnect to it
    pub async fn select_database(&mut self, database_name: &str) -> Result<()> {
        let log = self.query_log.clone();
        let explorer = &mut self.database_explorer;
        if explorer.database.is_some() {
            // Update connection with selected database
            explorer.connection.selected_database =
                Some(database_name.to_string());

            let db: Box<dyn Database> = explorer.connection.to_database();

            if db.test().await {
                explorer.database = Some(log.wrap(db));
                self.load_schemas().await?;
            } else {
                self.set_status(STATUS_CONNECT_FAILED);
            }
        }

        Ok(())
    }

    /// Load schemas from the database
    pub async fn load_schemas(&mut self) -> Result<()> {
        let explorer = &mut self.database_explorer;
        let Some(database) = explorer.database.as_mut() else {
            self.set_status(STATUS_DB_NOT_CONNECTED);
            return Ok(());
        };

        // SQLite doesn't need the Schemas navigation step
        // Skip directly to loading tables from the default sqlite_schema
        if explorer.connection.r#type == ConnectionType::Sqlite {
            return self.load_tables("sqlite_schema").await;
        }

        match database.get_schemas().await {
            Ok(schemas) => {
                explorer.schemas = Some(FilteredData::new(schemas));
                explorer.state = DatabaseExplorerState::Schemas;
            }
            Err(e) => self.status_load_failed("schemas", e),
        }

        Ok(())
    }

    /// Load tables for a schema
    pub async fn load_tables(&mut self, schema_name: &str) -> Result<()> {
        let explorer = &mut self.database_explorer;
        let Some(database) = explorer.database.as_mut() else {
            self.set_status(STATUS_DB_NOT_CONNECTED);
            return Ok(());
        };

        match database.get_tables(schema_name).await {
            Ok(tables) => {
                explorer.tables = Some(FilteredData::new(tables));
                explorer.state =
                    DatabaseExplorerState::Tables(schema_name.to_string());
            }
            Err(e) => self.status_load_failed("tables", e),
        }

        Ok(())
    }

    /// Load columns for a table
    pub async fn load_columns(
        &mut self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<()> {
        let explorer = &mut self.database_explorer;
        let Some(database) = explorer.database.as_mut() else {
            self.set_status(STATUS_DB_NOT_CONNECTED);
            return Ok(());
        };

        match database.get_columns(schema_name, table_name).await {
            Ok(columns) => {
                explorer.columns = Some(FilteredData::new(columns));
                explorer.state = DatabaseExplorerState::Columns(
                    schema_name.to_string(),
                    table_name.to_string(),
                );
            }
            Err(e) => self.status_load_failed("columns", e),
        }

        Ok(())
    }

    /// Load table data for a table (first page of a paged / virtual table).
    pub async fn load_table_data(
        &mut self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<()> {
        let Some(database) = self.database_explorer.database.as_ref() else {
            self.set_status(STATUS_DB_NOT_CONNECTED);
            return Ok(());
        };

        let total_rows = database
            .get_table_row_count(schema_name, table_name)
            .await
            .ok();
        let page_size = self.page_size;

        match database
            .get_table_data_page(schema_name, table_name, 0, page_size)
            .await
        {
            Ok(page) => {
                self.replace_explorer_table_page(
                    page, 0, page_size, total_rows,
                );
                let explorer = &mut self.database_explorer;
                explorer.state = DatabaseExplorerState::TableData(
                    schema_name.to_string(),
                    table_name.to_string(),
                );
                explorer.record_recent_table_open(schema_name, table_name);
            }
            Err(e) => {
                self.database_explorer.table_data_virtual = None;
                self.status_load_failed("table data", e);
            }
        }

        Ok(())
    }

    /// Jump to a 1-based row in the current table (unfiltered view only).
    pub async fn jump_to_table_row(&mut self, row_1based: u64) -> Result<()> {
        if row_1based == 0 {
            self.set_status("Row number must be >= 1");
            return Ok(());
        }
        if self.has_active_filter() {
            self.set_status("Clear filter before jumping to row");
            return Ok(());
        }
        let DatabaseExplorerState::TableData(schema, table) =
            self.database_explorer.state.clone()
        else {
            return Ok(());
        };
        let offset = row_1based - 1;
        let page_size = self.page_size;
        let window_start = offset - offset % u64::from(page_size);
        let local_idx = usize::try_from(offset - window_start).unwrap_or(0);
        let total_rows = self
            .database_explorer
            .table_data_virtual
            .as_ref()
            .and_then(|m| m.total_rows);
        let Some(database) = self.database_explorer.database.as_ref() else {
            self.set_status(STATUS_DB_NOT_CONNECTED);
            return Ok(());
        };
        match database
            .get_table_data_page(&schema, &table, window_start, page_size)
            .await
        {
            Ok(page) => {
                let loaded = page.rows.len();
                self.replace_explorer_table_page(
                    page,
                    window_start,
                    page_size,
                    total_rows,
                );
                if local_idx < loaded {
                    if let Some(fd) = self.database_explorer.table_data.as_mut()
                    {
                        fd.table.view.state.select(Some(local_idx));
                    }
                    self.set_status(format!("Jumped to row {row_1based}"));
                } else {
                    self.set_status(format!(
                        "Row {row_1based} is beyond loaded page"
                    ));
                }
            }
            Err(e) => self.status_load_failed("jump to row", e),
        }
        Ok(())
    }

    /// `:table <name>`: open the best-matching table of the current schema.
    /// `candidates` are the same-tier matches, best first.
    pub async fn jump_to_table_by_name(
        &mut self,
        candidates: &[String],
    ) -> Result<()> {
        let Some(table_name) = candidates.first() else {
            return Ok(());
        };
        let Some(schema) = self.command_schema() else {
            self.set_status("Open a schema first.");
            return Ok(());
        };
        self.load_table_data(&schema, table_name).await?;
        if candidates.len() > 1
            && self.database_explorer.state
                == DatabaseExplorerState::TableData(schema, table_name.clone())
        {
            let shown = candidates.iter().take(4).cloned().collect::<Vec<_>>();
            self.set_status(format!(
                "{} matches: {}; opened {table_name}",
                candidates.len(),
                shown.join(", ")
            ));
        }
        Ok(())
    }

    /// `:schema <name>`: open the tables list for the best-matching schema.
    pub async fn jump_to_schema_by_name(
        &mut self,
        candidates: &[String],
    ) -> Result<()> {
        let Some(schema_name) = candidates.first() else {
            return Ok(());
        };
        self.load_tables(schema_name).await?;
        if candidates.len() > 1
            && self.database_explorer.state
                == DatabaseExplorerState::Tables(schema_name.clone())
        {
            let shown = candidates.iter().take(4).cloned().collect::<Vec<_>>();
            self.set_status(format!(
                "{} matches: {}; opened {schema_name}",
                candidates.len(),
                shown.join(", ")
            ));
        }
        Ok(())
    }

    async fn fetch_adjacent_table_page(&mut self, next: bool) -> Result<()> {
        if self.discard_table_draft() {
            self.set_status("Draft discarded (page change).");
        }
        let Some(meta) = self.database_explorer.table_data_virtual.as_ref()
        else {
            return Ok(());
        };
        if next {
            if !meta.has_more_after {
                self.set_status("Already at last page.");
                return Ok(());
            }
        } else if !meta.has_more_before {
            self.set_status("Already at first page.");
            return Ok(());
        }
        let page_size = meta.page_size;
        let new_start = if next {
            meta.window_start + meta.loaded_count as u64
        } else {
            meta.window_start.saturating_sub(u64::from(page_size))
        };
        let total_rows = meta.total_rows;
        let DatabaseExplorerState::TableData(schema, table) =
            &self.database_explorer.state
        else {
            return Ok(());
        };
        let (schema, table) = (schema.clone(), table.clone());
        let Some(database) = self.database_explorer.database.as_ref() else {
            return Ok(());
        };
        match database
            .get_table_data_page(&schema, &table, new_start, page_size)
            .await
        {
            Ok(page) => self.replace_explorer_table_page(
                page, new_start, page_size, total_rows,
            ),
            Err(e) => self.status_load_failed("page", e),
        }
        Ok(())
    }

    /// Load the next page of rows for the current table data view.
    pub async fn fetch_next_table_page(&mut self) -> Result<()> {
        self.fetch_adjacent_table_page(true).await
    }

    /// Load the previous page of rows for the current table data view.
    pub async fn fetch_prev_table_page(&mut self) -> Result<()> {
        self.fetch_adjacent_table_page(false).await
    }

    /// At the first/last row of a loaded page, j/k loads the previous/next page.
    pub async fn try_step_virtual_table_page(
        &mut self,
        key: KeyEvent,
    ) -> Result<bool> {
        if !key.modifiers.is_empty() {
            return Ok(false);
        }
        let code = key.code;
        if !matches!(
            code,
            KeyCode::Down | KeyCode::Up | KeyCode::Char('j' | 'k')
        ) {
            return Ok(false);
        }

        let edge: Option<bool> = {
            let explorer = &self.database_explorer;
            if !matches!(explorer.state, DatabaseExplorerState::TableData(..)) {
                return Ok(false);
            }
            let Some(meta) = explorer.table_data_virtual.as_ref() else {
                return Ok(false);
            };
            let Some(table_fd) = &explorer.table_data else {
                return Ok(false);
            };
            if table_fd.is_filtered() {
                return Ok(false);
            }
            let Some(selected) = table_fd.table.view.state.selected() else {
                return Ok(false);
            };
            let len = table_fd.table.model.items.len();
            if len == 0 {
                return Ok(false);
            }

            if matches!(code, KeyCode::Char('j') | KeyCode::Down)
                && selected == len - 1
                && meta.has_more_after
            {
                Some(true)
            } else if matches!(code, KeyCode::Char('k') | KeyCode::Up)
                && selected == 0
                && meta.has_more_before
            {
                Some(false)
            } else {
                None
            }
        };

        match edge {
            Some(true) => {
                self.fetch_next_table_page().await?;
                Ok(true)
            }
            Some(false) => {
                self.fetch_prev_table_page().await?;
                if let Some(ref mut td) = self.database_explorer.table_data {
                    let last = td.table.model.items.len().saturating_sub(1);
                    td.table.view.state.select(Some(last));
                }
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Handle database navigation when Enter is pressed
    pub async fn handle_database_navigation(&mut self) -> Result<()> {
        let explorer_state = self.database_explorer.state.clone();

        match explorer_state {
            DatabaseExplorerState::Connections => {
                self.connect_to_database().await?;
            }
            DatabaseExplorerState::Databases => {
                if let Some(database_name) = self.get_selected_database_name() {
                    self.select_database(&database_name).await?;
                    self.set_status(format!("Opened database {database_name}"));
                }
            }
            DatabaseExplorerState::Schemas => {
                if let Some(schema_name) = self.get_selected_schema_name() {
                    self.database_explorer.connection.schema =
                        Some(schema_name.clone());
                    self.load_tables(&schema_name).await?;
                    self.set_status(format!("Opened schema {schema_name}"));
                }
            }
            DatabaseExplorerState::Tables(schema_name) => {
                if let Some(table_name) = self.get_selected_table_name() {
                    self.database_explorer.connection.table =
                        Some(table_name.clone());
                    self.load_table_data(&schema_name, &table_name).await?;
                    self.set_status(format!(
                        "Opened {schema_name}.{table_name}"
                    ));
                }
            }
            DatabaseExplorerState::Columns(schema_name, table_name) => {
                self.load_table_data(&schema_name, &table_name).await?;
                self.set_status(format!("Opened {schema_name}.{table_name}"));
            }
            DatabaseExplorerState::TableData(schema_name, table_name) => {
                if let Some((column_name, cell_value, row_idx, col_idx, snap)) =
                    self.get_selected_cell_for_modal()
                {
                    let schema = schema_name.clone();
                    let table = table_name.clone();
                    let db_row_id = self.get_selected_row_db_id();
                    let is_draft = self.table_data_selected_is_draft();
                    let Some(database) =
                        self.database_explorer.database.as_ref()
                    else {
                        return Ok(());
                    };
                    let pk_names = if is_draft {
                        Vec::new()
                    } else {
                        database
                            .get_primary_key_columns(&schema, &table)
                            .await
                            .unwrap_or_default()
                    };
                    let col_names: &[String] = self
                        .database_explorer
                        .table_data
                        .as_ref()
                        .and_then(|t| {
                            t.table.model.dynamic_column_names.as_ref()
                        })
                        .map_or(&[][..], |names| names.as_slice());
                    let primary_key: Vec<(String, String)> = pk_names
                        .into_iter()
                        .filter_map(|pk| {
                            let idx = column_index_for_name(col_names, &pk)?;
                            let val = snap.get(idx)?.clone();
                            Some((pk, val))
                        })
                        .collect();
                    self.modal_manager.open_cell_value_modal(
                        column_name,
                        &cell_value,
                        row_idx,
                        col_idx,
                        snap,
                        schema,
                        table,
                        primary_key,
                        db_row_id,
                    );
                }
            }
            DatabaseExplorerState::SqlResults(_) => {
                // Enter should not re-run SQL in results mode.
            }
        }
        Ok(())
    }

    fn get_selected_database_name(&self) -> Option<String> {
        explorer_selected_row_name!(self.database_explorer.databases)
    }

    fn get_selected_schema_name(&self) -> Option<String> {
        explorer_selected_row_name!(self.database_explorer.schemas)
    }

    pub(crate) fn get_selected_table_name(&self) -> Option<String> {
        explorer_selected_row_name!(self.database_explorer.tables)
    }

    /// Selected cell plus row index, column index, and full row snapshot (for syncing filtered data).
    fn get_selected_cell_for_modal(
        &self,
    ) -> Option<(String, String, usize, usize, Vec<String>)> {
        let explorer = &self.database_explorer;
        let table_data_filtered = explorer.table_data.as_ref()?;
        let table_data = &table_data_filtered.table;

        let selected_row = table_data.view.state.selected()?;
        let row = table_data.model.items.get(selected_row)?;
        let selected_col = table_data.view.state.selected_column().unwrap_or(0);
        let column_names = table_data.model.dynamic_column_names.as_ref()?;

        if selected_col >= column_names.len()
            || selected_col >= row.values.len()
        {
            return None;
        }

        let column_name = column_names.get(selected_col)?.clone();
        let cell_value = row.values.get(selected_col)?.clone();
        let snap = row.values.clone();
        Some((column_name, cell_value, selected_row, selected_col, snap))
    }

    fn get_selected_row_db_id(&self) -> Option<DbRowId> {
        let explorer = &self.database_explorer;
        let fd = explorer.table_data.as_ref()?;
        let selected = fd.table.view.state.selected()?;
        let row = fd.table.model.items.get(selected)?;
        row.db_row_id.clone()
    }

    /// Persist a cell edit from the modal, then refresh the grid.
    pub async fn apply_cell_value_edit(
        &mut self,
        apply: CellValueApply,
    ) -> Result<()> {
        let is_draft = self
            .database_explorer
            .table_data
            .as_ref()
            .and_then(|fd| fd.table.model.items.get(apply.row_index))
            .is_some_and(|r| r.is_draft);
        if is_draft {
            self.apply_cell_value_edit_in_memory(&apply);
            self.set_status("Draft cell updated — commit with s when ready.");
            return Ok(());
        }
        let Some(database) = self.database_explorer.database.as_ref() else {
            self.set_status(STATUS_DB_NOT_CONNECTED);
            return Ok(());
        };
        match database
            .update_table_cell(
                &apply.schema_name,
                &apply.table_name,
                &apply.set_column,
                &apply.new_value,
                &apply.primary_key,
                apply.db_row_id.clone(),
            )
            .await
        {
            Ok(0) => {
                self.set_status(
                    "No row was updated (it may have changed in the database).",
                );
            }
            Ok(_) => {
                self.apply_cell_value_edit_in_memory(&apply);
                self.set_status("Cell updated.");
            }
            Err(e) => self.status_action_failed("Update", e),
        }
        Ok(())
    }

    fn apply_cell_value_edit_in_memory(&mut self, apply: &CellValueApply) {
        let Some(fd) = self.database_explorer.table_data.as_mut() else {
            return;
        };
        if let Some(row) = fd.table.model.items.get_mut(apply.row_index)
            && let Some(cell) = row.values.get_mut(apply.col_index)
        {
            cell.clone_from(&apply.new_value);
        }
        if let Some(ix) = fd
            .original
            .iter()
            .position(|r| r.values == apply.row_snapshot)
            && let Some(cell) = fd
                .original
                .get_mut(ix)
                .and_then(|r| r.values.get_mut(apply.col_index))
        {
            cell.clone_from(&apply.new_value);
        }
        fd.table.recompute_column_widths();
    }

    /// Execute SQL query from the SQL executor
    pub(crate) async fn execute_sql_query(&mut self) {
        self.execute_sql_query_inner(true).await;
    }

    /// Re-run the currently selected statement without touching SQL history.
    /// Used by watch mode's periodic refresh.
    pub(crate) async fn execute_sql_query_watch_tick(&mut self) {
        self.execute_sql_query_inner(false).await;
    }

    async fn execute_sql_query_inner(&mut self, record_history: bool) {
        let sql = self
            .database_explorer
            .sql_executor
            .selected_statement()
            .unwrap_or_default()
            .trim()
            .to_string();
        if sql.is_empty() {
            self.set_status("No SQL statement selected for execution.");
            return;
        }

        let Some(database) = self.database_explorer.database.as_ref() else {
            self.set_status(STATUS_DB_NOT_CONNECTED);
            return;
        };

        // Clear any previous results/errors before executing
        self.database_explorer.sql_executor.clear_results();
        self.query_log.set_origin(if !record_history {
            QueryOrigin::Watch
        } else if sql == ACTIVITY_QUERY {
            QueryOrigin::Activity
        } else {
            QueryOrigin::User
        });

        match database.execute_sql(&sql).await {
            Ok(results) => {
                let Some(first) = results.first() else {
                    self.set_status(
                        "Query executed successfully but returned no data",
                    );
                    return;
                };
                let cols = first.column_names.clone();
                let data = results.into_iter().map(|r| r.values).collect();
                self.database_explorer.sql_executor.set_results(data, &cols);
                if record_history {
                    let _ = PreferencesService::push_sql_history(&sql);
                }
            }
            Err(e) => self.set_status(format!("SQL error: {e}")),
        }
    }

    /// Go back to previous level in database navigation
    pub async fn go_back_in_database(&mut self) {
        let explorer_state = self.database_explorer.state.clone();
        let explorer = &mut self.database_explorer;
        let is_sqlite = explorer.connection.r#type == ConnectionType::Sqlite;

        match explorer_state {
            DatabaseExplorerState::Connections => {
                // Nowhere to go back from connections list
            }
            DatabaseExplorerState::TableData(schema_name, _)
            | DatabaseExplorerState::Columns(schema_name, _) => {
                // Go back to tables in the same schema
                if explorer.tables.is_some() {
                    explorer.state = DatabaseExplorerState::Tables(schema_name);
                    explorer.connection.table = None;
                }
            }
            DatabaseExplorerState::Tables(_) => {
                // SQLite: Go back to connections (disconnect)
                // Postgres: Go back to schemas
                if is_sqlite {
                    self.disconnect_from_database();
                    self.refresh_connections().await;
                } else if explorer.schemas.is_some() {
                    explorer.state = DatabaseExplorerState::Schemas;
                    explorer.connection.schema = None;
                }
            }
            DatabaseExplorerState::Schemas => {
                // Go back to databases
                if explorer.databases.is_some() {
                    explorer.state = DatabaseExplorerState::Databases;
                }
            }
            DatabaseExplorerState::SqlResults(_) => {
                // SQLite: Go back to tables
                // Postgres: Go back to schemas
                if is_sqlite {
                    if explorer.tables.is_some() {
                        explorer.state = DatabaseExplorerState::Tables(
                            "sqlite_schema".to_string(),
                        );
                    }
                } else if explorer.schemas.is_some() {
                    explorer.state = DatabaseExplorerState::Schemas;
                }
            }
            DatabaseExplorerState::Databases => {
                // Go back to connection list (disconnect)
                self.disconnect_from_database();
                self.refresh_connections().await;
            }
        }
    }

    /// Handle table navigation for the current database table
    pub fn handle_database_table_navigation(&mut self, key: KeyCode) {
        match self.database_explorer.state {
            DatabaseExplorerState::Connections => {
                self.database_explorer.connections.navigate(key);
            }
            DatabaseExplorerState::SqlResults(_) => {
                TableNavigationHandler::navigate_table(
                    &self.database_explorer.sql_executor.table_state.model,
                    &mut self.database_explorer.sql_executor.table_state.view,
                    key,
                );
            }
            DatabaseExplorerState::Databases
            | DatabaseExplorerState::Schemas
            | DatabaseExplorerState::Tables(_)
            | DatabaseExplorerState::Columns(..)
            | DatabaseExplorerState::TableData(..) => {
                self.database_explorer.navigate_current(key);
            }
        }
    }
}
