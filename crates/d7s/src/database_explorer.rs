use core::fmt;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent};
use k9tui::widgets::{
    navigation::TableNavigationHandler, table::TableDataState,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    app::{ACTIVITY_QUERY, App},
    app_state::DatabaseExplorerState,
    bg::{
        BgEvent, SqlExecuteOk, TableColumnMetaOk, TableMutationReload,
        TablePageLoadOk, spawn_keyed,
    },
    db::{
        Column, Database, DatabaseInfo, DbRowId, RowDeleteSpec, Schema, Table,
        TableDataPage, connection::ConnectionType, query_log::QueryOrigin,
    },
    error::Result,
    filtered_data::FilteredData,
    in_flight::{
        database_key, schema_key, schemas_key, select_database_key, sql_key,
        table_key,
    },
    ui::widgets::{
        connection_modal::CellValueApply, raw_table::RawTableStateExt,
    },
    virtual_table::VirtualTableMeta,
};

const STATUS_DB_NOT_CONNECTED: &str = "Not connected to database.";
pub const STATUS_CONNECT_FAILED: &str = "Failed to connect to database.";

pub fn column_index_for_name(names: &[String], name: &str) -> Option<usize> {
    names
        .iter()
        .position(|c| c == name)
        .or_else(|| names.iter().position(|c| c.eq_ignore_ascii_case(name)))
}

pub fn pk_value_pairs(
    col_names: &[String],
    row_values: &[String],
    pk_names: &[String],
) -> Vec<(String, String)> {
    pk_names
        .iter()
        .filter_map(|pk| {
            let idx = column_index_for_name(col_names, pk)?;
            Some((pk.clone(), row_values.get(idx)?.clone()))
        })
        .collect()
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

    fn spawn_explorer_load(
        &mut self,
        key: &str,
        busy_message: &str,
        status: &str,
    ) -> Option<(Arc<dyn Database>, UnboundedSender<BgEvent>)> {
        if !self.in_flight.try_begin(key.to_owned()) {
            self.set_status(busy_message);
            return None;
        }
        let Some(database) = self.database_explorer.database.clone() else {
            self.in_flight.end(key);
            self.set_status(STATUS_DB_NOT_CONNECTED);
            return None;
        };
        self.set_status(status);
        Some((database, self.bg_tx.clone()))
    }

    fn table_mutation_reload(
        schema: &str,
        table: &str,
        success_status: Option<String>,
    ) -> TableMutationReload {
        TableMutationReload {
            schema: schema.to_string(),
            table: table.to_string(),
            success_status,
        }
    }

    /// Persist a cell edit from the modal or inline editor (off the UI thread).
    /// Returns `true` when the edit was queued or applied in-memory (clear inline session).
    pub fn spawn_cell_value_edit(&mut self, apply: &CellValueApply) -> bool {
        if self.row_is_draft_at(apply.row_index) {
            self.apply_cell_value_edit_in_memory(apply);
            self.set_status("Draft cell updated — commit with s when ready.");
            return true;
        }
        let key = table_key(
            &self.database_explorer.connection.name,
            &apply.schema_name,
            &apply.table_name,
        );
        let status = "Saving cell…";
        let Some((database, tx)) = self.spawn_explorer_load(
            &key,
            "Already busy with this table",
            status,
        ) else {
            return false;
        };
        let schema = apply.schema_name.clone();
        let table = apply.table_name.clone();
        let set_column = apply.set_column.clone();
        let new_value = apply.new_value.clone();
        let primary_key = apply.primary_key.clone();
        let db_row_id = apply.db_row_id.clone();
        let reload = Self::table_mutation_reload(
            &schema,
            &table,
            Some(format!("UPDATED {set_column}")),
        );
        spawn_keyed(tx.clone(), key.clone(), async move {
            let event = match database
                .update_table_cell(
                    &schema,
                    &table,
                    &set_column,
                    &new_value,
                    &primary_key,
                    db_row_id,
                )
                .await
            {
                Ok(0) => BgEvent::ActionFailed {
                    key,
                    message:
                        "No row was updated (it may have changed in the database)."
                            .into(),
                },
                Ok(_) => BgEvent::ActionDone {
                    key,
                    message: format!("UPDATED {set_column}"),
                    reload_table: Some(reload),
                },
                Err(e) => BgEvent::ActionFailed {
                    key,
                    message: format!("Update failed: {e}"),
                },
            };
            let _ = tx.send(event);
        });
        true
    }

    pub(crate) fn spawn_table_row_insert(
        &mut self,
        schema: &str,
        table: &str,
        values: Vec<String>,
    ) {
        let key =
            table_key(&self.database_explorer.connection.name, schema, table);
        let status = "Saving row…";
        let Some((database, tx)) = self.spawn_explorer_load(
            &key,
            "Already busy with this table",
            status,
        ) else {
            return;
        };
        let schema = schema.to_string();
        let table = table.to_string();
        let reload = Self::table_mutation_reload(
            &schema,
            &table,
            Some("Row inserted.".into()),
        );
        spawn_keyed(tx.clone(), key.clone(), async move {
            let event =
                match database.insert_table_row(&schema, &table, &values).await
                {
                    Ok(0) => BgEvent::ActionFailed {
                        key,
                        message: "Insert affected 0 rows.".into(),
                    },
                    Ok(_) => BgEvent::ActionDone {
                        key,
                        message: "Row inserted.".into(),
                        reload_table: Some(reload),
                    },
                    Err(e) => BgEvent::ActionFailed {
                        key,
                        message: format!("Insert failed: {e}"),
                    },
                };
            let _ = tx.send(event);
        });
    }

    pub(crate) fn spawn_table_row_deletes(
        &mut self,
        schema: &str,
        table: &str,
        specs: Vec<RowDeleteSpec>,
    ) {
        let key =
            table_key(&self.database_explorer.connection.name, schema, table);
        let status = "Deleting…";
        let Some((database, tx)) = self.spawn_explorer_load(
            &key,
            "Already busy with this table",
            status,
        ) else {
            return;
        };
        let schema = schema.to_string();
        let table = table.to_string();
        spawn_keyed(tx.clone(), key.clone(), async move {
            let mut any_ok = false;
            let mut errs = 0u32;
            for spec in specs {
                match database
                    .delete_table_row(
                        &schema,
                        &table,
                        &spec.primary_key,
                        spec.row_id_fallback,
                    )
                    .await
                {
                    Ok(n) if n > 0 => any_ok = true,
                    _ => errs += 1,
                }
            }
            let event = if any_ok {
                let message = if errs > 0 {
                    format!("Some deletes failed ({errs}). Refetching.")
                } else {
                    "Row(s) deleted.".into()
                };
                let reload_status = message.clone();
                BgEvent::ActionDone {
                    key,
                    message,
                    reload_table: Some(Self::table_mutation_reload(
                        &schema,
                        &table,
                        Some(reload_status),
                    )),
                }
            } else {
                BgEvent::ActionFailed {
                    key,
                    message: "Delete failed.".into(),
                }
            };
            let _ = tx.send(event);
        });
    }

    /// Load databases from the connection
    pub fn load_databases(&mut self) {
        let conn = self.database_explorer.connection.name.clone();
        let key = database_key(&conn);
        let Some((database, tx)) = self.spawn_explorer_load(
            &key,
            "Already loading databases",
            "Loading databases…",
        ) else {
            return;
        };
        spawn_keyed(tx.clone(), key.clone(), async move {
            match database.get_databases().await {
                Ok(databases) => {
                    let _ =
                        tx.send(BgEvent::DatabasesLoaded { key, databases });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::ExplorerLoadFailed {
                        key,
                        resource: "databases".into(),
                        schema: None,
                        table: None,
                        message: e.to_string(),
                    });
                }
            }
        });
    }

    /// Select a database and reconnect to it (off the UI thread).
    pub fn spawn_select_database(&mut self, database_name: &str) {
        let explorer = &mut self.database_explorer;
        if explorer.database.is_none() {
            return;
        }
        let conn_name = explorer.connection.name.clone();
        let key = select_database_key(&conn_name, database_name);
        if !self.in_flight.try_begin(key.clone()) {
            self.set_status("Already opening database…");
            return;
        }
        let connection = explorer.connection.clone();
        let db_name = database_name.to_string();
        let query_log = self.query_log.clone();
        let tx = self.bg_tx.clone();
        self.set_status(format!("Opening database {database_name}…"));
        spawn_keyed(tx.clone(), key.clone(), async move {
            let mut conn = connection;
            conn.selected_database = Some(db_name.clone());
            let db: Box<dyn Database> = conn.to_database();
            let wrapped = query_log.wrap(db);
            if wrapped.test().await {
                let _ = tx.send(BgEvent::DatabaseSelected {
                    key,
                    database_name: db_name,
                    database: Arc::from(wrapped),
                });
            } else {
                let _ = tx.send(BgEvent::DatabaseSelectFailed { key });
            }
        });
    }

    pub(crate) fn apply_database_selected(
        &mut self,
        database_name: &str,
        database: Arc<dyn Database>,
    ) {
        self.database_explorer.connection.selected_database =
            Some(database_name.to_string());
        self.database_explorer.database = Some(database);
        self.load_schemas();
        self.set_status(format!("Opened database {database_name}"));
    }

    /// Load schemas from the database
    pub fn load_schemas(&mut self) {
        if self.database_explorer.connection.r#type == ConnectionType::Sqlite {
            self.load_tables("sqlite_schema");
            return;
        }

        let conn = self.database_explorer.connection.name.clone();
        let key = schemas_key(&conn);
        let Some((database, tx)) = self.spawn_explorer_load(
            &key,
            "Already loading schemas",
            "Loading schemas…",
        ) else {
            return;
        };
        spawn_keyed(tx.clone(), key.clone(), async move {
            match database.get_schemas().await {
                Ok(schemas) => {
                    let _ = tx.send(BgEvent::SchemasLoaded { key, schemas });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::ExplorerLoadFailed {
                        key,
                        resource: "schemas".into(),
                        schema: None,
                        table: None,
                        message: e.to_string(),
                    });
                }
            }
        });
    }

    /// Load tables for a schema
    pub fn load_tables(&mut self, schema_name: &str) {
        let schema = schema_name.to_string();
        let key =
            schema_key(&self.database_explorer.connection.name, schema_name);
        let status = format!("Loading tables in {schema_name}…");
        let Some((database, tx)) = self.spawn_explorer_load(
            &key,
            "Already loading tables for this schema",
            &status,
        ) else {
            return;
        };
        spawn_keyed(tx.clone(), key.clone(), async move {
            match database.get_tables(&schema).await {
                Ok(tables) => {
                    let _ = tx.send(BgEvent::TablesLoaded {
                        key,
                        schema,
                        tables,
                    });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::ExplorerLoadFailed {
                        key,
                        resource: "tables".into(),
                        schema: Some(schema),
                        table: None,
                        message: e.to_string(),
                    });
                }
            }
        });
    }

    /// Load columns for a table
    pub fn load_columns(&mut self, schema_name: &str, table_name: &str) {
        let schema = schema_name.to_string();
        let table = table_name.to_string();
        let key = table_key(
            &self.database_explorer.connection.name,
            schema_name,
            table_name,
        );
        let status = format!("Loading columns for {schema_name}.{table_name}…");
        let Some((database, tx)) = self.spawn_explorer_load(
            &key,
            "Already loading this table",
            &status,
        ) else {
            return;
        };
        spawn_keyed(tx.clone(), key.clone(), async move {
            match database.get_columns(&schema, &table).await {
                Ok(columns) => {
                    let _ = tx.send(BgEvent::ColumnsLoaded {
                        key,
                        schema,
                        table,
                        columns,
                    });
                }
                Err(e) => {
                    let _ = tx.send(BgEvent::ExplorerLoadFailed {
                        key,
                        resource: "columns".into(),
                        schema: Some(schema),
                        table: Some(table),
                        message: e.to_string(),
                    });
                }
            }
        });
    }

    /// Column + PK metadata for table-data edits (e.g. Shift+N defaults draft).
    pub(crate) fn spawn_table_column_meta(
        &mut self,
        schema_name: &str,
        table_name: &str,
    ) -> bool {
        let schema = schema_name.to_string();
        let table = table_name.to_string();
        let key = table_key(
            &self.database_explorer.connection.name,
            schema_name,
            table_name,
        );
        let Some((database, tx)) = self.spawn_explorer_load(
            &key,
            "Already busy with this table",
            "Loading columns…",
        ) else {
            return false;
        };
        spawn_keyed(tx.clone(), key.clone(), async move {
            let columns = match database.get_columns(&schema, &table).await {
                Ok(columns) => columns,
                Err(e) => {
                    let _ = tx.send(BgEvent::TableColumnMetaLoaded {
                        key,
                        schema,
                        table,
                        result: Err(e.to_string()),
                    });
                    return;
                }
            };
            let pk_names = database
                .get_primary_key_columns(&schema, &table)
                .await
                .unwrap_or_default();
            let _ = tx.send(BgEvent::TableColumnMetaLoaded {
                key,
                schema,
                table,
                result: Ok(TableColumnMetaOk { columns, pk_names }),
            });
        });
        true
    }

    /// Load table data for a table (first page of a paged / virtual table).
    pub fn load_table_data(&mut self, schema_name: &str, table_name: &str) {
        self.spawn_table_page_load(
            schema_name,
            table_name,
            0,
            None,
            true,
            None,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_table_page_load(
        &mut self,
        schema_name: &str,
        table_name: &str,
        window_start: u64,
        total_rows: Option<u64>,
        record_recent: bool,
        select_row_at: Option<usize>,
        success_status: Option<String>,
    ) {
        let schema = schema_name.to_string();
        let table = table_name.to_string();
        let key = table_key(
            &self.database_explorer.connection.name,
            schema_name,
            table_name,
        );
        let status = format!("Loading {table_name}…");
        let page_size = self.page_size;
        let fetch_meta = window_start == 0 && record_recent;
        let Some((database, tx)) = self.spawn_explorer_load(
            &key,
            "Already loading this table",
            &status,
        ) else {
            return;
        };
        spawn_keyed(tx.clone(), key.clone(), async move {
            let total_rows = if window_start == 0 && total_rows.is_none() {
                database.get_table_row_count(&schema, &table).await.ok()
            } else {
                total_rows
            };
            let page = match database
                .get_table_data_page(&schema, &table, window_start, page_size)
                .await
            {
                Ok(page) => page,
                Err(e) => {
                    let _ = tx.send(BgEvent::TablePageLoaded {
                        key,
                        schema,
                        table,
                        record_recent,
                        result: Err(e.to_string()),
                    });
                    return;
                }
            };
            let (table_columns, table_pk_names) = if fetch_meta {
                let col_meta = database.get_columns(&schema, &table).await.ok();
                let pk_meta = database
                    .get_primary_key_columns(&schema, &table)
                    .await
                    .ok();
                (col_meta, pk_meta)
            } else {
                (None, None)
            };
            let _ = tx.send(BgEvent::TablePageLoaded {
                key,
                schema: schema.clone(),
                table: table.clone(),
                record_recent,
                result: Ok(TablePageLoadOk {
                    schema,
                    table,
                    window_start,
                    page_size,
                    total_rows,
                    page,
                    table_columns,
                    table_pk_names,
                    record_recent,
                    select_row_at,
                    success_status,
                }),
            });
        });
    }

    /// Jump to a 1-based row in the current table (unfiltered view only).
    pub fn jump_to_table_row(&mut self, row_1based: u64) {
        if row_1based == 0 {
            self.set_status("Row number must be >= 1");
            return;
        }
        if self.has_active_filter() {
            self.set_status("Clear filter before jumping to row");
            return;
        }
        let DatabaseExplorerState::TableData(schema, table) =
            self.database_explorer.state.clone()
        else {
            return;
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
        self.spawn_table_page_load(
            &schema,
            &table,
            window_start,
            total_rows,
            false,
            Some(local_idx),
            None,
        );
    }

    /// `:table <name>`: open the best-matching table of the current schema.
    /// `candidates` are the same-tier matches, best first.
    pub fn jump_to_table_by_name(&mut self, candidates: &[String]) {
        let Some(table_name) = candidates.first() else {
            return;
        };
        let Some(schema) = self.command_schema() else {
            self.set_status("Open a schema first.");
            return;
        };
        self.load_table_data(&schema, table_name);
        if candidates.len() > 1 {
            let shown = candidates.iter().take(4).cloned().collect::<Vec<_>>();
            self.set_status(format!(
                "{} matches: {}; opened {table_name}",
                candidates.len(),
                shown.join(", ")
            ));
        }
    }

    /// `:schema <name>`: open the tables list for the best-matching schema.
    pub fn jump_to_schema_by_name(&mut self, candidates: &[String]) {
        let Some(schema_name) = candidates.first() else {
            return;
        };
        self.load_tables(schema_name);
        if candidates.len() > 1 {
            let shown = candidates.iter().take(4).cloned().collect::<Vec<_>>();
            self.set_status(format!(
                "{} matches: {}; opened {schema_name}",
                candidates.len(),
                shown.join(", ")
            ));
        }
    }

    fn fetch_adjacent_table_page(&mut self, next: bool) {
        if self.discard_table_draft() {
            self.set_status("Draft discarded (page change).");
        }
        let Some(meta) = self.database_explorer.table_data_virtual.as_ref()
        else {
            return;
        };
        if next {
            if !meta.has_more_after {
                self.set_status("Already at last page.");
                return;
            }
        } else if !meta.has_more_before {
            self.set_status("Already at first page.");
            return;
        }
        let page_size = meta.page_size;
        let new_start = if next {
            meta.window_start + meta.loaded_count as u64
        } else {
            meta.window_start.saturating_sub(u64::from(page_size))
        };
        let total_rows = meta.total_rows;
        let DatabaseExplorerState::TableData(schema, table) =
            self.database_explorer.state.clone()
        else {
            return;
        };
        let select_row_at = if next {
            None
        } else {
            Some(page_size.saturating_sub(1) as usize)
        };
        self.spawn_table_page_load(
            &schema,
            &table,
            new_start,
            total_rows,
            false,
            select_row_at,
            None,
        );
    }

    /// Load the next page of rows for the current table data view.
    pub fn fetch_next_table_page(&mut self) {
        self.fetch_adjacent_table_page(true);
    }

    /// Load the previous page of rows for the current table data view.
    pub fn fetch_prev_table_page(&mut self) {
        self.fetch_adjacent_table_page(false);
    }

    /// At the first/last row of a loaded page, j/k loads the previous/next page.
    pub fn try_step_virtual_table_page(&mut self, key: KeyEvent) -> bool {
        if !key.modifiers.is_empty() {
            return false;
        }
        let code = key.code;
        if !matches!(
            code,
            KeyCode::Down | KeyCode::Up | KeyCode::Char('j' | 'k')
        ) {
            return false;
        }

        let edge: Option<bool> = {
            let explorer = &self.database_explorer;
            if !matches!(explorer.state, DatabaseExplorerState::TableData(..)) {
                return false;
            }
            let Some(meta) = explorer.table_data_virtual.as_ref() else {
                return false;
            };
            let Some(table_fd) = &explorer.table_data else {
                return false;
            };
            if table_fd.is_filtered() {
                return false;
            }
            let Some(selected) = table_fd.table.view.state.selected() else {
                return false;
            };
            let len = table_fd.table.model.items.len();
            if len == 0 {
                return false;
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
                self.fetch_next_table_page();
                true
            }
            Some(false) => {
                self.fetch_prev_table_page();
                true
            }
            None => false,
        }
    }

    /// Handle database navigation when Enter is pressed
    pub fn handle_database_navigation(&mut self) {
        let explorer_state = self.database_explorer.state.clone();

        match explorer_state {
            DatabaseExplorerState::Connections => {
                self.connect_to_database();
            }
            DatabaseExplorerState::Databases => {
                if let Some(database_name) = self.get_selected_database_name() {
                    self.spawn_select_database(&database_name);
                }
            }
            DatabaseExplorerState::Schemas => {
                if let Some(schema_name) = self.get_selected_schema_name() {
                    self.database_explorer.connection.schema =
                        Some(schema_name.clone());
                    self.load_tables(&schema_name);
                    self.set_status(format!("Opened schema {schema_name}"));
                }
            }
            DatabaseExplorerState::Tables(schema_name) => {
                if let Some(table_name) = self.get_selected_table_name() {
                    self.database_explorer.connection.table =
                        Some(table_name.clone());
                    self.load_table_data(&schema_name, &table_name);
                    self.set_status(format!(
                        "Opened {schema_name}.{table_name}"
                    ));
                }
            }
            DatabaseExplorerState::Columns(schema_name, table_name) => {
                self.load_table_data(&schema_name, &table_name);
                self.set_status(format!("Opened {schema_name}.{table_name}"));
            }
            DatabaseExplorerState::TableData(_, _) => {
                self.begin_cell_edit(None);
            }
            DatabaseExplorerState::SqlResults(_) => {
                // Enter should not re-run SQL in results mode.
            }
        }
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
    pub(crate) fn get_selected_cell_for_modal(
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

    pub fn finish_cell_edit(&mut self) {
        let DatabaseExplorerState::TableData(schema, table) =
            self.database_explorer.state.clone()
        else {
            return;
        };
        let unchanged = self.cell_edit.as_ref().is_some_and(|s| {
            s.buffer == s.pre_edit_value
                // Enter on a NULL display starts with an empty buffer; leaving
                // without typing must not write '' over SQL NULL.
                || (s.pre_edit_value == "NULL" && s.buffer.is_empty())
        });
        if unchanged {
            // Unchanged: leave edit mode without a write (NULL cells render as "NULL").
            self.cell_edit = None;
            return;
        }
        let Some(new_value) = self.cell_edit.as_ref().map(|s| s.buffer.clone())
        else {
            return;
        };
        let Some(apply) =
            self.cell_value_apply_from_session(schema, table, &new_value)
        else {
            return;
        };
        self.sync_cell_edit_buffer_to_grid();
        if self.spawn_cell_value_edit(&apply) {
            self.cell_edit = None;
        }
    }

    /// Build a `CellValueApply` from the active inline edit session.
    /// Uses cached `table_pk_names` (filled on table load) instead of re-fetching.
    pub(crate) fn cell_value_apply_from_session(
        &self,
        schema: String,
        table: String,
        new_value: &str,
    ) -> Option<CellValueApply> {
        let session = self.cell_edit.as_ref()?;
        let col_names = self.table_data_column_names();
        let set_column = col_names.get(session.col_index)?.clone();
        let row_snapshot = session.row_snapshot.clone();
        let is_draft = self.row_is_draft_at(session.row_index);
        let db_row_id = self.row_db_id_at(session.row_index);
        let pk_names: &[String] = if is_draft {
            &[]
        } else {
            self.database_explorer
                .table_pk_names
                .as_deref()
                .unwrap_or(&[])
        };
        let primary_key = pk_value_pairs(&col_names, &row_snapshot, pk_names);
        Some(CellValueApply {
            schema_name: schema,
            table_name: table,
            set_column,
            row_snapshot,
            row_index: session.row_index,
            col_index: session.col_index,
            new_value: new_value.to_string(),
            primary_key,
            db_row_id,
        })
    }

    pub(crate) fn table_data_column_names(&self) -> Vec<String> {
        self.database_explorer
            .table_data
            .as_ref()
            .and_then(|fd| fd.table.model.dynamic_column_names.as_ref())
            .map(|names| names.to_vec())
            .unwrap_or_default()
    }

    pub(crate) fn row_is_draft_at(&self, row_index: usize) -> bool {
        self.database_explorer
            .table_data
            .as_ref()
            .and_then(|fd| fd.table.model.items.get(row_index))
            .is_some_and(|r| r.is_draft)
    }

    pub(crate) fn row_db_id_at(&self, row_index: usize) -> Option<DbRowId> {
        self.database_explorer
            .table_data
            .as_ref()
            .and_then(|fd| fd.table.model.items.get(row_index))
            .and_then(|r| r.db_row_id.clone())
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
        // ponytail: full-row equality scan of the page; store original_index in
        // the apply payload if pages grow past VIRTUAL_TABLE_PAGE_SIZE cost.
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
        // ponytail: full-page width scan per cell; bump only the edited column
        // (or skip when new width ≤ current) if Tab on wide pages feels laggy.
        fd.table.recompute_column_widths();
    }

    /// Execute SQL query from the SQL executor
    pub(crate) fn execute_sql_query(&mut self) {
        self.execute_sql_query_inner(true);
    }

    /// Re-run the currently selected statement without touching SQL history.
    /// Used by watch mode's periodic refresh.
    pub(crate) fn execute_sql_query_watch_tick(&mut self) {
        self.execute_sql_query_inner(false);
    }

    fn execute_sql_query_inner(&mut self, record_history: bool) {
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

        let Some(database) = self.database_explorer.database.clone() else {
            self.set_status(STATUS_DB_NOT_CONNECTED);
            return;
        };

        let key = sql_key(&self.database_explorer.connection.name);
        if record_history {
            if !self.in_flight.try_begin(key.clone()) {
                self.set_status("Already running a query on this connection");
                return;
            }
        } else if !self.in_flight.try_begin(key.clone()) {
            return;
        }

        self.database_explorer.sql_executor.clear_results();
        self.query_log.set_origin(if !record_history {
            QueryOrigin::Watch
        } else if sql == ACTIVITY_QUERY {
            QueryOrigin::Activity
        } else {
            QueryOrigin::User
        });
        self.set_status("Running query…");

        let tx = self.bg_tx.clone();
        spawn_keyed(tx.clone(), key.clone(), async move {
            let result = match database.execute_sql(&sql).await {
                Ok(results) => {
                    let Some(first) = results.first() else {
                        let _ = tx.send(BgEvent::SqlFinished {
                            key,
                            sql,
                            record_history,
                            result: Err(
                                "Query executed successfully but returned no data"
                                    .into(),
                            ),
                        });
                        return;
                    };
                    let cols = first.column_names.clone();
                    let rows = results.into_iter().map(|r| r.values).collect();
                    Ok(SqlExecuteOk { cols, rows })
                }
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(BgEvent::SqlFinished {
                key,
                sql,
                record_history,
                result,
            });
        });
    }

    fn should_apply_explorer_load_failed(
        state: &DatabaseExplorerState,
        resource: &str,
        schema: Option<&str>,
        table: Option<&str>,
    ) -> bool {
        match resource {
            "databases" => {
                matches!(state, DatabaseExplorerState::Databases)
            }
            "schemas" => matches!(
                state,
                DatabaseExplorerState::Databases
                    | DatabaseExplorerState::Schemas
            ),
            "tables" => match state {
                DatabaseExplorerState::Schemas => true,
                DatabaseExplorerState::Tables(s)
                | DatabaseExplorerState::Columns(s, _)
                | DatabaseExplorerState::TableData(s, _) => {
                    schema.is_some_and(|sch| s == sch)
                }
                DatabaseExplorerState::Connections
                | DatabaseExplorerState::Databases
                | DatabaseExplorerState::SqlResults(_) => false,
            },
            "columns" => match (schema, table) {
                (Some(schema), Some(table)) => {
                    Self::should_apply_table_page(state, schema, table, true)
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn should_apply_table_page(
        state: &DatabaseExplorerState,
        schema: &str,
        table: &str,
        record_recent: bool,
    ) -> bool {
        match state {
            DatabaseExplorerState::TableData(s, t) => s == schema && t == table,
            DatabaseExplorerState::Columns(s, t) if record_recent => {
                s == schema && t == table
            }
            DatabaseExplorerState::Tables(s) if record_recent => s == schema,
            DatabaseExplorerState::Tables(_)
            | DatabaseExplorerState::Columns(_, _)
            | DatabaseExplorerState::Connections
            | DatabaseExplorerState::Databases
            | DatabaseExplorerState::Schemas
            | DatabaseExplorerState::SqlResults(_) => false,
        }
    }

    pub(crate) fn apply_databases_loaded(
        &mut self,
        databases: Vec<DatabaseInfo>,
    ) {
        if !matches!(
            self.database_explorer.state,
            DatabaseExplorerState::Databases
        ) {
            return;
        }
        self.database_explorer.databases = Some(FilteredData::new(databases));
        self.database_explorer.state = DatabaseExplorerState::Databases;
    }

    pub(crate) fn apply_schemas_loaded(&mut self, schemas: Vec<Schema>) {
        if !matches!(
            self.database_explorer.state,
            DatabaseExplorerState::Databases | DatabaseExplorerState::Schemas
        ) {
            return;
        }
        self.database_explorer.schemas = Some(FilteredData::new(schemas));
        self.database_explorer.state = DatabaseExplorerState::Schemas;
    }

    pub(crate) fn apply_tables_loaded(
        &mut self,
        schema: &str,
        tables: Vec<Table>,
    ) {
        let ok = match &self.database_explorer.state {
            DatabaseExplorerState::Schemas => true,
            DatabaseExplorerState::Tables(s)
            | DatabaseExplorerState::Columns(s, _)
            | DatabaseExplorerState::TableData(s, _) => s == schema,
            DatabaseExplorerState::Connections
            | DatabaseExplorerState::Databases
            | DatabaseExplorerState::SqlResults(_) => false,
        };
        if !ok {
            return;
        }
        self.database_explorer.tables = Some(FilteredData::new(tables));
        self.database_explorer.state =
            DatabaseExplorerState::Tables(schema.to_string());
    }

    pub(crate) fn apply_columns_loaded(
        &mut self,
        schema: &str,
        table: &str,
        columns: Vec<Column>,
    ) {
        if !Self::should_apply_table_page(
            &self.database_explorer.state,
            schema,
            table,
            true,
        ) {
            return;
        }
        self.database_explorer.columns = Some(FilteredData::new(columns));
        self.database_explorer.state = DatabaseExplorerState::Columns(
            schema.to_string(),
            table.to_string(),
        );
    }

    pub(crate) fn apply_table_column_meta_loaded(
        &mut self,
        schema: &str,
        table: &str,
        result: Result<TableColumnMetaOk, String>,
    ) {
        let on_table = matches!(
            &self.database_explorer.state,
            DatabaseExplorerState::TableData(s, t) if s == schema && t == table
        );
        if !on_table {
            self.pending_defaults_draft = false;
            return;
        }
        match result {
            Err(e) => {
                self.pending_defaults_draft = false;
                self.status_load_failed("columns", e);
            }
            Ok(meta) => {
                let explorer = &mut self.database_explorer;
                explorer.table_columns = Some(meta.columns);
                explorer.table_pk_names = Some(meta.pk_names);
                if self.pending_defaults_draft {
                    self.pending_defaults_draft = false;
                    match self.insert_defaults_draft_from_meta() {
                        Ok(Some(defaults_n)) => {
                            self.begin_cell_edit_on_first_editable_column();
                            self.set_status(format!(
                                "Draft with defaults ({defaults_n} cols) · Ctrl+e modal"
                            ));
                        }
                        Ok(None) => {}
                        Err(e) => self.set_status(e.to_string()),
                    }
                }
            }
        }
    }

    pub(crate) fn apply_table_page_loaded(
        &mut self,
        schema: &str,
        table: &str,
        record_recent: bool,
        result: Result<TablePageLoadOk, String>,
    ) {
        match result {
            Err(e) => {
                if !Self::should_apply_table_page(
                    &self.database_explorer.state,
                    schema,
                    table,
                    record_recent,
                ) {
                    return;
                }
                self.database_explorer.table_data_virtual = None;
                self.status_load_failed("table data", e);
            }
            Ok(payload) => {
                if !Self::should_apply_table_page(
                    &self.database_explorer.state,
                    &payload.schema,
                    &payload.table,
                    payload.record_recent,
                ) {
                    return;
                }
                let loaded = payload.page.rows.len();
                self.replace_explorer_table_page(
                    payload.page,
                    payload.window_start,
                    payload.page_size,
                    payload.total_rows,
                );
                let explorer = &mut self.database_explorer;
                if let Some(cols) = payload.table_columns {
                    explorer.table_columns = Some(cols);
                }
                if let Some(pk) = payload.table_pk_names {
                    explorer.table_pk_names = Some(pk);
                }
                if payload.record_recent {
                    explorer.state = DatabaseExplorerState::TableData(
                        payload.schema.clone(),
                        payload.table.clone(),
                    );
                    explorer.record_recent_table_open(
                        &payload.schema,
                        &payload.table,
                    );
                }
                if let Some(idx) = payload.select_row_at {
                    if idx < loaded {
                        if let Some(fd) = explorer.table_data.as_mut() {
                            fd.table.view.state.select(Some(idx));
                        }
                        let row_1based = payload.window_start
                            + u64::try_from(idx).unwrap_or(0)
                            + 1;
                        self.set_status(format!("Jumped to row {row_1based}"));
                    } else {
                        self.set_status("Row is beyond loaded page");
                    }
                } else if let Some(msg) = payload.success_status {
                    self.set_status(msg);
                } else if payload.record_recent
                    && self.status_line.message().contains("Loading")
                {
                    self.clear_status();
                }
            }
        }
    }

    pub(crate) fn apply_explorer_load_failed(
        &mut self,
        resource: &str,
        schema: Option<&str>,
        table: Option<&str>,
        message: &str,
    ) {
        if !Self::should_apply_explorer_load_failed(
            &self.database_explorer.state,
            resource,
            schema,
            table,
        ) {
            return;
        }
        self.set_status(format!("Failed to load {resource}: {message}"));
    }

    /// Go back to previous level in database navigation
    pub fn go_back_in_database(&mut self) {
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
                    self.refresh_connections();
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
                self.refresh_connections();
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
