//! Table data view: draft rows, multi-select, insert/delete, refresh.

use alloc::collections::BTreeSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use k9tui::{
    error::{Result, err},
    widgets::navigation::TableNavigationHandler,
};

use crate::{
    app::App,
    app_state::{AppState, DatabaseExplorerState},
    cell_edit::{
        CellEditSession, EditableCol, all_editable_columns,
        editable_columns_from_meta, is_table_data_browse_binding,
        next_editable_column, prev_editable_column,
    },
    database_explorer::{column_index_for_name, pk_value_pairs},
    db::{
        DbRowId, RowDeleteSpec, connection::ConnectionType,
        draft_cell_from_column_default, should_omit_for_insert_default,
        sole_pk, sqlite_rowid_pk_omit,
    },
    ui::widgets::raw_table::{RawTableRow, RawTableStateExt},
};

impl App<'_> {
    fn strip_draft_rows_from_table_data(&mut self) {
        let Some(fd) = self.database_explorer.table_data.as_mut() else {
            return;
        };
        fd.table.model.items.retain(|r| !r.is_draft);
        fd.original.retain(|r| !r.is_draft);
        fd.table.multi_row_selection.clear();
        TableNavigationHandler::wrap_rows(
            &mut fd.table.view.state,
            &fd.table.model.items,
        );
    }

    /// Remove client-only draft rows and refetch the current page from the server.
    pub(crate) fn reload_current_table_data(
        &mut self,
        success_status: Option<String>,
    ) {
        self.strip_draft_rows_from_table_data();
        let DatabaseExplorerState::TableData(schema, table) =
            self.database_explorer.state.clone()
        else {
            return;
        };
        let offset = self
            .database_explorer
            .table_data_virtual
            .as_ref()
            .map_or(0, |m| m.window_start);
        let total_rows = self
            .database_explorer
            .table_data_virtual
            .as_ref()
            .and_then(|m| m.total_rows);
        self.spawn_table_page_load(
            &schema,
            &table,
            offset,
            total_rows,
            false,
            None,
            success_status.or_else(|| Some("Refreshed.".into())),
        );
    }

    pub(crate) fn discard_table_draft(&mut self) -> bool {
        let had = self.has_table_draft_rows();
        if had {
            self.strip_draft_rows_from_table_data();
            self.draft_discard_pending = false;
        }
        had
    }

    pub(crate) fn has_table_draft_rows(&self) -> bool {
        self.database_explorer
            .table_data
            .as_ref()
            .is_some_and(|fd| fd.table.model.items.iter().any(|r| r.is_draft))
    }

    fn engine_sqlite(&self) -> bool {
        self.database_explorer.connection.r#type == ConnectionType::Sqlite
    }

    fn editable_columns_for_current_table(&self) -> Vec<EditableCol> {
        let num_cols = self
            .database_explorer
            .table_data
            .as_ref()
            .and_then(|fd| fd.table.model.items.first())
            .map_or(0, |r| r.values.len());
        let Some(cols) = self.database_explorer.table_columns.as_ref() else {
            return all_editable_columns(num_cols);
        };
        let pk = self
            .database_explorer
            .table_pk_names
            .as_deref()
            .unwrap_or(&[]);
        let ec = editable_columns_from_meta(cols, pk, self.engine_sqlite());
        if ec.len() == num_cols {
            ec
        } else {
            all_editable_columns(num_cols)
        }
    }

    pub(crate) const fn is_cell_editing(&self) -> bool {
        self.cell_edit.is_some()
    }

    // ponytail: buffer mirrored into the grid every keystroke; render from
    // session (sync once on commit/cancel) if large-cell paste allocs show up.
    pub(crate) fn sync_cell_edit_buffer_to_grid(&mut self) {
        let Some(session) = self.cell_edit.as_ref() else {
            return;
        };
        let Some(fd) = self.database_explorer.table_data.as_mut() else {
            return;
        };
        if let Some(row) = fd.table.model.items.get_mut(session.row_index)
            && let Some(cell) = row.values.get_mut(session.col_index)
        {
            cell.clone_from(&session.buffer);
        }
    }

    pub(crate) fn begin_cell_edit(&mut self, replace_with: Option<String>) {
        if self.cell_edit.is_some() {
            return;
        }
        let Some((column_name, cell_value, row_idx, col_idx, snap)) =
            self.get_selected_cell_for_modal()
        else {
            return;
        };
        if self
            .editable_columns_for_current_table()
            .iter()
            .find(|c| c.index == col_idx)
            .is_some_and(|c| !c.editable)
        {
            self.set_status(format!("Column '{column_name}' is not editable"));
            return;
        }
        // SQL NULL renders as "NULL". Start with an empty buffer so typing
        // replaces instead of appending (`NULLhello`); keep the grid showing
        // NULL until the first keystroke syncs.
        let from_null_display = replace_with.is_none() && cell_value == "NULL";
        let buffer = replace_with.unwrap_or_else(|| {
            if from_null_display {
                String::new()
            } else {
                cell_value.clone()
            }
        });
        if let Some(fd) = self.database_explorer.table_data.as_mut() {
            fd.table.view.state.select_column(Some(col_idx));
            fd.table.view.state.select(Some(row_idx));
        }
        self.cell_edit = Some(CellEditSession {
            row_index: row_idx,
            col_index: col_idx,
            buffer,
            pre_edit_value: cell_value,
            row_snapshot: snap,
        });
        if !from_null_display {
            self.sync_cell_edit_buffer_to_grid();
        }
        self.set_status(format!("EDIT col={column_name} · Ctrl+e modal"));
    }

    fn table_data_column_is_selected(&self) -> bool {
        self.database_explorer
            .table_data
            .as_ref()
            .and_then(|fd| fd.table.view.state.selected_column())
            .is_some()
    }

    pub(crate) fn cancel_cell_edit(&mut self) {
        let Some(session) = self.cell_edit.take() else {
            return;
        };
        if let Some(fd) = self.database_explorer.table_data.as_mut()
            && let Some(row) = fd.table.model.items.get_mut(session.row_index)
            && let Some(cell) = row.values.get_mut(session.col_index)
        {
            *cell = session.pre_edit_value;
        }
    }

    // ponytail: Tab spawns cell UPDATE off the UI thread; batch or dirty-grid
    // if rapid Tab on remote Postgres still feels sticky.
    fn finish_cell_edit_and_move_editable_column(&mut self, forward: bool) {
        let row_index = self.cell_edit.as_ref().map(|s| s.row_index);
        let from_col = self.cell_edit.as_ref().map_or(0, |s| s.col_index);
        self.finish_cell_edit();
        if self.cell_edit.is_some() {
            return;
        }
        let cols = self.editable_columns_for_current_table();
        let next_col = if forward {
            next_editable_column(&cols, from_col)
        } else {
            prev_editable_column(&cols, from_col)
        };
        let Some(fd) = self.database_explorer.table_data.as_mut() else {
            return;
        };
        if let Some(col) = next_col {
            fd.table.view.state.select_column(Some(col));
            if let Some(row) = row_index {
                fd.table.view.state.select(Some(row));
            }
        }
        self.begin_cell_edit(None);
    }

    fn open_cell_value_modal_from_cell_edit(&mut self) {
        let DatabaseExplorerState::TableData(schema, table) =
            self.database_explorer.state.clone()
        else {
            return;
        };
        let Some(new_value) = self.cell_edit.as_ref().map(|s| s.buffer.clone())
        else {
            return;
        };
        let Some(apply) =
            self.cell_value_apply_from_session(schema, table, &new_value)
        else {
            return;
        };
        self.modal_manager.open_cell_value_modal(
            apply.set_column,
            &apply.new_value,
            apply.row_index,
            apply.col_index,
            apply.row_snapshot,
            apply.schema_name,
            apply.table_name,
            apply.primary_key,
            apply.db_row_id,
        );
    }

    // Swallow remaining keys while editing so Browse bindings cannot fire mid-edit.
    #[allow(clippy::wildcard_enum_match_arm)]
    fn handle_cell_edit_key(&mut self, key: KeyEvent) -> bool {
        if self.cell_edit.is_none() {
            return false;
        }
        match key.code {
            KeyCode::Esc => {
                self.cancel_cell_edit();
                true
            }
            KeyCode::Backspace | KeyCode::Delete => {
                if let Some(session) = self.cell_edit.as_mut() {
                    session.buffer.pop();
                }
                self.sync_cell_edit_buffer_to_grid();
                true
            }
            KeyCode::Enter => false,
            KeyCode::Char(c)
                if !c.is_control()
                    && allows_hotkey_modifiers(key.modifiers) =>
            {
                if let Some(session) = self.cell_edit.as_mut() {
                    session.buffer.push(c);
                }
                self.sync_cell_edit_buffer_to_grid();
                true
            }
            _ => true,
        }
    }

    /// Insert a draft at `insert_at` (0..=len) and move the cursor to it.
    fn insert_draft_row_at(&mut self, insert_at: usize, row: RawTableRow) {
        let Some(fd) = self.database_explorer.table_data.as_mut() else {
            return;
        };
        let len = fd.table.model.items.len();
        let insert_at = insert_at.min(len);
        let shifted: BTreeSet<usize> = fd
            .table
            .multi_row_selection
            .iter()
            .map(|&j| if j >= insert_at { j + 1 } else { j })
            .collect();
        fd.table.model.items.insert(insert_at, row.clone());
        fd.original.insert(insert_at, row);
        fd.table.multi_row_selection = shifted;
        fd.table.view.state.select(Some(insert_at));
        fd.table.recompute_column_widths();
        TableNavigationHandler::wrap_rows(
            &mut fd.table.view.state,
            &fd.table.model.items,
        );
    }

    /// Insert a draft **below** the current selection after `discard_table_draft` (or with empty data).
    fn insert_draft_row_below_cursor(&mut self, row: RawTableRow) {
        let Some(fd) = self.database_explorer.table_data.as_ref() else {
            return;
        };
        let len = fd.table.model.items.len();
        let insert_at = match fd.table.view.state.selected() {
            None if len == 0 => 0,
            None => len,
            Some(i) => (i + 1).min(len),
        };
        self.insert_draft_row_at(insert_at, row);
    }

    pub(crate) fn table_data_add_blank_draft(&mut self) -> Result<bool> {
        let Some(fd) = self.database_explorer.table_data.as_ref() else {
            return Ok(false);
        };
        if fd.is_filtered() {
            self.set_status("Clear filter before adding a row.");
            return Ok(false);
        }
        let names = fd
            .table
            .model
            .dynamic_column_names
            .clone()
            .ok_or_else(|| err("no columns"))?;
        let n = names.len();
        self.discard_table_draft();
        let row = RawTableRow {
            values: vec![String::new(); n],
            column_names: names,
            db_row_id: None,
            is_draft: true,
        };
        self.insert_draft_row_below_cursor(row);
        self.set_status("Draft row — edit cells (Enter), commit with s");
        Ok(true)
    }

    /// `Ok(Some(n))` = draft created with `n` columns using schema defaults.
    pub(crate) fn table_data_add_defaults_draft(
        &mut self,
    ) -> Result<Option<usize>> {
        let DatabaseExplorerState::TableData(schema, table) =
            self.database_explorer.state.clone()
        else {
            return Ok(None);
        };
        let Some(fd) = self.database_explorer.table_data.as_ref() else {
            return Ok(None);
        };
        if fd.is_filtered() {
            self.set_status("Clear filter before adding a row.");
            return Ok(None);
        }
        if self.database_explorer.table_columns.is_some()
            && self.database_explorer.table_pk_names.is_some()
        {
            return self.insert_defaults_draft_from_meta();
        }
        if self.spawn_table_column_meta(&schema, &table) {
            self.pending_defaults_draft = true;
        }
        Ok(None)
    }

    pub(crate) fn insert_defaults_draft_from_meta(
        &mut self,
    ) -> Result<Option<usize>> {
        let columns = self
            .database_explorer
            .table_columns
            .clone()
            .ok_or_else(|| err("no columns"))?;
        let names: Vec<String> =
            columns.iter().map(|c| c.name.clone()).collect();
        let engine_sqlite = matches!(
            self.database_explorer.connection.r#type,
            ConnectionType::Sqlite
        );
        let pk_names = self
            .database_explorer
            .table_pk_names
            .as_deref()
            .unwrap_or(&[]);
        let pk_first = sole_pk(pk_names);
        let values: Vec<String> =
            columns.iter().map(draft_cell_from_column_default).collect();
        let defaults_n = columns
            .iter()
            .filter(|c| {
                should_omit_for_insert_default(
                    c,
                    "",
                    engine_sqlite,
                    engine_sqlite && sqlite_rowid_pk_omit(c, pk_first),
                )
            })
            .count();
        self.discard_table_draft();
        let row = RawTableRow {
            values,
            column_names: names.into(),
            db_row_id: None,
            is_draft: true,
        };
        self.insert_draft_row_below_cursor(row);
        Ok(Some(defaults_n))
    }

    pub(crate) fn begin_cell_edit_on_first_editable_column(&mut self) {
        let cols = self.editable_columns_for_current_table();
        let col = cols.iter().find(|c| c.editable).map(|c| c.index);
        if let Some(col) = col
            && let Some(fd) = self.database_explorer.table_data.as_mut()
        {
            fd.table.view.state.select_column(Some(col));
        }
        self.begin_cell_edit(None);
    }

    pub(crate) fn table_data_duplicate_as_draft(&mut self) -> Result<bool> {
        let Some(fd) = self.database_explorer.table_data.as_ref() else {
            return Ok(false);
        };
        if fd.is_filtered() {
            self.set_status("Clear filter before duplicating a row.");
            return Ok(false);
        }
        if !matches!(
            self.database_explorer.state,
            DatabaseExplorerState::TableData(_, _)
        ) {
            return Ok(false);
        }
        let sel = fd
            .table
            .view
            .state
            .selected()
            .ok_or_else(|| err("no selection"))?;
        let base = fd
            .table
            .model
            .items
            .get(sel)
            .cloned()
            .ok_or_else(|| err("no row"))?;
        if base.is_draft {
            self.set_status("Pick a saved row to duplicate, not a draft.");
            return Ok(false);
        }
        let pk_names = self
            .database_explorer
            .table_pk_names
            .clone()
            .unwrap_or_default();
        let col_names: Vec<String> = fd
            .table
            .model
            .dynamic_column_names
            .as_deref()
            .cloned()
            .unwrap_or_default();
        let source_key = (base.db_row_id.clone(), base.values.clone());
        let mut values = base.values.clone();
        for pk in &pk_names {
            if let Some(ix) = column_index_for_name(&col_names, pk)
                && let Some(v) = values.get_mut(ix)
            {
                v.clear();
            }
        }
        let row = RawTableRow {
            values,
            column_names: base.column_names,
            db_row_id: None,
            is_draft: true,
        };
        self.discard_table_draft();
        let insert_at: usize = {
            let Some(fd2) = self.database_explorer.table_data.as_ref() else {
                return Ok(false);
            };
            let len = fd2.table.model.items.len();
            fd2.table
                .model
                .items
                .iter()
                .position(|r| {
                    (r.db_row_id == source_key.0) && (r.values == source_key.1)
                })
                .map_or(len, |p| (p + 1).min(len))
        };
        self.insert_draft_row_at(insert_at, row);
        self.set_status("Draft copy — edit cells (Enter), commit with s");
        Ok(true)
    }

    pub(crate) fn table_data_commit_draft(&mut self) -> Result<()> {
        let DatabaseExplorerState::TableData(schema, table) =
            self.database_explorer.state.clone()
        else {
            return Ok(());
        };
        let Some(fd) = self.database_explorer.table_data.as_ref() else {
            return Ok(());
        };
        if fd.is_filtered() {
            self.set_status("Clear filter before committing.");
            return Ok(());
        }
        let sel = fd.table.view.state.selected().filter(|&i| {
            fd.table.model.items.get(i).is_some_and(|r| r.is_draft)
        });
        let Some(sel) = sel else {
            self.set_status("Select a draft row to commit (n / c).");
            return Ok(());
        };
        let values = fd
            .table
            .model
            .items
            .get(sel)
            .map(|r| r.values.clone())
            .ok_or_else(|| err("no draft row"))?;
        self.spawn_table_row_insert(&schema, &table, values);
        Ok(())
    }

    pub(crate) fn table_data_toggle_multi_select(&mut self) {
        let Some(fd) = self.database_explorer.table_data.as_mut() else {
            return;
        };
        if fd.is_filtered() {
            self.set_status("Clear filter for multi-select.");
            return;
        }
        let Some(i) = fd.table.view.state.selected() else {
            return;
        };
        if fd.table.model.items.get(i).is_some_and(|r| r.is_draft) {
            self.set_status("Multi-select applies to saved rows.");
            return;
        }
        if fd.table.multi_row_selection.contains(&i) {
            fd.table.multi_row_selection.remove(&i);
        } else {
            fd.table.multi_row_selection.insert(i);
        }
    }

    /// Start delete: drafts removed locally; persisted rows get a confirmation modal.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn table_data_request_delete(&mut self) {
        let DatabaseExplorerState::TableData(schema, table) =
            &self.database_explorer.state
        else {
            return;
        };
        let Some(fd) = self.database_explorer.table_data.as_ref() else {
            return;
        };
        if fd.is_filtered() {
            self.set_status("Clear filter before delete.");
            return;
        }
        let mut pick: BTreeSet<usize> = fd.table.multi_row_selection.clone();
        if let Some(s) = fd.table.view.state.selected()
            && pick.is_empty()
        {
            pick.insert(s);
        }
        if pick.is_empty() {
            return;
        }
        let col_names: Vec<String> = fd
            .table
            .model
            .dynamic_column_names
            .as_deref()
            .cloned()
            .unwrap_or_default();
        let pk_col_names = self
            .database_explorer
            .table_pk_names
            .clone()
            .unwrap_or_default();
        let mut draft_indices: Vec<usize> = Vec::new();
        let mut db_specs: Vec<RowDeleteSpec> = Vec::new();
        let mut preview: Option<String> = None;
        for &i in &pick {
            let Some(row) = fd.table.model.items.get(i) else {
                continue;
            };
            if row.is_draft {
                draft_indices.push(i);
                continue;
            }
            let primary_key =
                pk_value_pairs(&col_names, &row.values, &pk_col_names);
            if !primary_key.is_empty() {
                if preview.is_none() {
                    let w = primary_key
                        .iter()
                        .map(|(k, v)| format!("{k} = {v:?}"))
                        .collect::<Vec<_>>()
                        .join(" AND ");
                    let tbl = if self.database_explorer.connection.r#type
                        == ConnectionType::Sqlite
                    {
                        table.clone()
                    } else {
                        format!("{schema}.{table}")
                    };
                    preview = Some(format!("DELETE FROM {tbl} WHERE {w}"));
                }
                db_specs.push(RowDeleteSpec {
                    primary_key,
                    row_id_fallback: None,
                });
            } else if let Some(rid) = row.db_row_id.clone() {
                if preview.is_none() {
                    preview = match &rid {
                        DbRowId::PostgresCtid(t) => Some(format!(
                            "DELETE FROM {schema}.{table} WHERE ctid = {t:?}"
                        )),
                        DbRowId::Sqlite(rid) => Some(format!(
                            "DELETE FROM {table} WHERE rowid = {rid}"
                        )),
                    };
                }
                db_specs.push(RowDeleteSpec {
                    primary_key: vec![],
                    row_id_fallback: Some(rid),
                });
            }
        }
        draft_indices.sort_unstable();
        draft_indices.dedup();
        if !draft_indices.is_empty() {
            if let Some(fd2) = self.database_explorer.table_data.as_mut() {
                for i in draft_indices.iter().rev() {
                    if *i < fd2.table.model.items.len() {
                        fd2.table.model.items.remove(*i);
                    }
                }
                fd2.original = fd2.table.model.items.clone();
                fd2.table.multi_row_selection.clear();
                TableNavigationHandler::wrap_rows(
                    &mut fd2.table.view.state,
                    &fd2.table.model.items,
                );
            }
            self.set_status("Draft row(s) removed.");
        }
        if db_specs.is_empty() {
            return;
        }
        let p = if db_specs.len() > 1 {
            format!(
                "{}\n… and {} more",
                preview.as_deref().unwrap_or("DELETE …"),
                db_specs.len() - 1
            )
        } else {
            preview.unwrap_or_else(|| "Delete row?".to_string())
        };
        self.pending_row_deletes = Some(db_specs);
        self.modal_manager.open_sql_execution_confirmation_modal(p);
    }

    /// After confirmation: delete persisted rows off the UI thread, then refetch.
    pub(crate) fn spawn_pending_row_deletes(&mut self) {
        let Some(specs) = self.pending_row_deletes.take() else {
            return;
        };
        if specs.is_empty() {
            return;
        }
        let DatabaseExplorerState::TableData(schema, table) =
            self.database_explorer.state.clone()
        else {
            return;
        };
        self.spawn_table_row_deletes(&schema, &table, specs);
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    pub(crate) fn handle_table_data_hotkeys(
        &mut self,
        key: KeyEvent,
    ) -> Result<bool> {
        if self.state != AppState::DatabaseConnected {
            return Ok(false);
        }
        if !matches!(
            self.database_explorer.state,
            DatabaseExplorerState::TableData(_, _)
        ) {
            return Ok(false);
        }
        // Uppercase letters (`D`, `A`, ...) arrive with SHIFT set; only
        // reject other modifiers (Ctrl/Alt chords).
        if self.cell_edit.is_some() {
            if key.code == KeyCode::Tab {
                self.finish_cell_edit_and_move_editable_column(true);
                return Ok(true);
            }
            if key.code == KeyCode::BackTab {
                self.finish_cell_edit_and_move_editable_column(false);
                return Ok(true);
            }
            if key.code == KeyCode::Enter {
                self.finish_cell_edit();
                return Ok(true);
            }
            if matches!(
                (key.modifiers, key.code),
                (KeyModifiers::CONTROL, KeyCode::Char('e' | 'E'))
            ) {
                self.open_cell_value_modal_from_cell_edit();
                return Ok(true);
            }
            if matches!(
                (key.modifiers, key.code),
                (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C'))
            ) {
                self.quit();
                return Ok(true);
            }
            return Ok(self.handle_cell_edit_key(key));
        }
        if !allows_hotkey_modifiers(key.modifiers) {
            return Ok(false);
        }
        match key.code {
            KeyCode::Char('r' | 'R') => {
                self.reload_current_table_data(None);
                Ok(true)
            }
            KeyCode::Char('n' | 'a') => {
                if self.table_data_add_blank_draft()? {
                    self.begin_cell_edit_on_first_editable_column();
                }
                Ok(true)
            }
            KeyCode::Char('N') => {
                if let Some(n) = self.table_data_add_defaults_draft()? {
                    self.begin_cell_edit_on_first_editable_column();
                    // Set after begin so EDIT status does not clobber the count.
                    self.set_status(format!(
                        "Draft with defaults ({n} cols) · Ctrl+e modal"
                    ));
                }
                Ok(true)
            }
            KeyCode::Char('i') => {
                self.begin_cell_edit(None);
                Ok(true)
            }
            KeyCode::Char('c') => {
                if self.table_data_duplicate_as_draft()? {
                    self.begin_cell_edit_on_first_editable_column();
                }
                Ok(true)
            }
            KeyCode::Char('s') => {
                self.table_data_commit_draft()?;
                Ok(true)
            }
            KeyCode::Char('D') => {
                self.table_data_request_delete();
                Ok(true)
            }
            KeyCode::Char(' ') => {
                self.table_data_toggle_multi_select();
                Ok(true)
            }
            KeyCode::Backspace | KeyCode::Delete => {
                // Require an explicit column selection so a stray key on a
                // freshly opened table cannot overwrite column 0 (often the PK).
                if self.table_data_column_is_selected() {
                    self.begin_cell_edit(Some(String::new()));
                }
                Ok(true)
            }
            KeyCode::Char(c) if !is_table_data_browse_binding(c) => {
                if self.table_data_column_is_selected() {
                    self.begin_cell_edit(Some(c.to_string()));
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

/// Plain hotkeys may carry SHIFT (uppercase letters arrive as `Char('D')` +
/// SHIFT); Ctrl/Alt chords are not hotkeys.
const fn allows_hotkey_modifiers(mods: KeyModifiers) -> bool {
    mods.difference(KeyModifiers::SHIFT).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_d_is_a_hotkey_but_ctrl_d_is_not() {
        assert!(allows_hotkey_modifiers(KeyModifiers::NONE));
        assert!(allows_hotkey_modifiers(KeyModifiers::SHIFT));
        assert!(!allows_hotkey_modifiers(KeyModifiers::CONTROL));
        assert!(!allows_hotkey_modifiers(
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ));
    }
}
