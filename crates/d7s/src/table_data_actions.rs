//! Table data view: draft rows, multi-select, insert/delete, refresh.

use std::collections::BTreeSet;

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use k9tui::widgets::navigation::TableNavigationHandler;

use crate::{
    app::App,
    app_state::{AppState, DatabaseExplorerState},
    db::{DbRowId, RowDeleteSpec, connection::ConnectionType},
    ui::widgets::raw_table::{RawTableRow, RawTableStateExt},
};

impl App<'_> {
    pub(crate) fn table_data_selected_is_draft(&self) -> bool {
        let Some(fd) = self.database_explorer.table_data.as_ref() else {
            return false;
        };
        let Some(sel) = fd.table.view.state.selected() else {
            return false;
        };
        fd.table.model.items.get(sel).is_some_and(|r| r.is_draft)
    }

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
    pub(crate) async fn reload_current_table_data(&mut self) -> Result<()> {
        self.strip_draft_rows_from_table_data();
        let DatabaseExplorerState::TableData(schema, table) =
            self.database_explorer.state.clone()
        else {
            return Ok(());
        };
        let offset = self
            .database_explorer
            .table_data_virtual
            .as_ref()
            .map_or(0, |m| m.window_start);
        let page_size = self
            .database_explorer
            .table_data_virtual
            .as_ref()
            .map_or(self.page_size, |m| m.page_size);
        let total_rows = self
            .database_explorer
            .table_data_virtual
            .as_ref()
            .and_then(|m| m.total_rows);
        let Some(database) = self.database_explorer.database.as_ref() else {
            return Ok(());
        };
        match database
            .get_table_data_page(&schema, &table, offset, page_size)
            .await
        {
            Ok(page) => {
                self.replace_explorer_table_page(
                    page, offset, page_size, total_rows,
                );
                self.set_status("Refreshed.");
            }
            Err(e) => self.status_action_failed("Refresh", e),
        }
        Ok(())
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

    pub(crate) fn table_data_add_blank_draft(&mut self) -> Result<()> {
        let Some(fd) = self.database_explorer.table_data.as_ref() else {
            return Ok(());
        };
        if fd.is_filtered() {
            self.set_status("Clear filter before adding a row.");
            return Ok(());
        }
        let names = fd
            .table
            .model
            .dynamic_column_names
            .clone()
            .ok_or_else(|| color_eyre::eyre::eyre!("no columns"))?;
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
        Ok(())
    }

    pub(crate) async fn table_data_duplicate_as_draft(&mut self) -> Result<()> {
        let Some(fd) = self.database_explorer.table_data.as_ref() else {
            return Ok(());
        };
        if fd.is_filtered() {
            self.set_status("Clear filter before duplicating a row.");
            return Ok(());
        }
        let DatabaseExplorerState::TableData(ref schema, ref table) =
            self.database_explorer.state
        else {
            return Ok(());
        };
        let sel = fd
            .table
            .view
            .state
            .selected()
            .ok_or_else(|| color_eyre::eyre::eyre!("no selection"))?;
        let base = fd
            .table
            .model
            .items
            .get(sel)
            .cloned()
            .ok_or_else(|| color_eyre::eyre::eyre!("no row"))?;
        if base.is_draft {
            self.set_status("Pick a saved row to duplicate, not a draft.");
            return Ok(());
        }
        let Some(db) = self.database_explorer.database.as_ref() else {
            return Ok(());
        };
        let pk_names = db
            .get_primary_key_columns(schema, table)
            .await
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
            if let Some(ix) =
                col_names.iter().position(|c| c == pk).or_else(|| {
                    col_names.iter().position(|c| c.eq_ignore_ascii_case(pk))
                })
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
                return Ok(());
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
        Ok(())
    }

    pub(crate) async fn table_data_commit_draft(&mut self) -> Result<()> {
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
            self.set_status("Select a draft row to commit (a / c).");
            return Ok(());
        };
        let values = fd
            .table
            .model
            .items
            .get(sel)
            .map(|r| r.values.clone())
            .ok_or_else(|| color_eyre::eyre::eyre!("no draft row"))?;
        let Some(db) = self.database_explorer.database.as_ref() else {
            return Ok(());
        };
        {
            let n = db.insert_table_row(&schema, &table, &values).await;
            match n {
                Ok(count) if count > 0 => {}
                Ok(_) => {
                    self.set_status("Insert affected 0 rows.");
                    return Ok(());
                }
                Err(e) => {
                    self.status_action_failed("Insert", e);
                    return Ok(());
                }
            }
        }
        self.set_status("Row inserted.");
        self.reload_current_table_data().await?;
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
    pub(crate) async fn table_data_request_delete(&mut self) -> Result<()> {
        let DatabaseExplorerState::TableData(ref schema, ref table) =
            self.database_explorer.state
        else {
            return Ok(());
        };
        let Some(db) = self.database_explorer.database.as_ref() else {
            return Ok(());
        };
        let Some(fd) = self.database_explorer.table_data.as_ref() else {
            return Ok(());
        };
        if fd.is_filtered() {
            self.set_status("Clear filter before delete.");
            return Ok(());
        }
        let mut pick: BTreeSet<usize> = fd.table.multi_row_selection.clone();
        if let Some(s) = fd.table.view.state.selected()
            && pick.is_empty()
        {
            pick.insert(s);
        }
        if pick.is_empty() {
            return Ok(());
        }
        let col_names: Vec<String> = fd
            .table
            .model
            .dynamic_column_names
            .as_deref()
            .cloned()
            .unwrap_or_default();
        let pk_col_names = db
            .get_primary_key_columns(schema, table)
            .await
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
            let primary_key: Vec<(String, String)> = pk_col_names
                .iter()
                .filter_map(|pk| {
                    let idx = col_names.iter().position(|c| c == pk).or_else(
                        || {
                            col_names
                                .iter()
                                .position(|c| c.eq_ignore_ascii_case(pk))
                        },
                    )?;
                    let val = row.values.get(idx)?.clone();
                    Some((pk.clone(), val))
                })
                .collect();
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
            return Ok(());
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
        Ok(())
    }

    /// After confirmation: delete persisted rows, then refetch.
    pub(crate) async fn execute_pending_row_deletes(&mut self) -> Result<()> {
        let Some(specs) = self.pending_row_deletes.take() else {
            return Ok(());
        };
        if specs.is_empty() {
            return Ok(());
        }
        let DatabaseExplorerState::TableData(ref schema, ref table) =
            self.database_explorer.state
        else {
            return Ok(());
        };
        let Some(db) = self.database_explorer.database.as_ref() else {
            return Ok(());
        };
        let mut any_ok = false;
        let mut errs = 0u32;
        for spec in specs {
            match db
                .delete_table_row(
                    schema,
                    table,
                    &spec.primary_key,
                    spec.row_id_fallback,
                )
                .await
            {
                Ok(n) if n > 0 => any_ok = true,
                _ => errs += 1,
            }
        }
        if any_ok {
            if errs > 0 {
                self.set_status(format!(
                    "Some deletes failed ({errs}). Refetching."
                ));
            } else {
                self.set_status("Row(s) deleted.");
            }
            self.reload_current_table_data().await?;
        } else if errs > 0 {
            self.set_status("Delete failed.");
        }
        Ok(())
    }

    #[allow(clippy::wildcard_enum_match_arm)]
    pub(crate) async fn handle_table_data_hotkeys(
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
        if !allows_hotkey_modifiers(key.modifiers) {
            return Ok(false);
        }
        match key.code {
            KeyCode::Char('r' | 'R') => {
                self.reload_current_table_data().await?;
                Ok(true)
            }
            KeyCode::Char('a' | 'A') => {
                self.table_data_add_blank_draft()?;
                Ok(true)
            }
            KeyCode::Char('c' | 'C') => {
                self.table_data_duplicate_as_draft().await?;
                Ok(true)
            }
            KeyCode::Char('s' | 'S') => {
                self.table_data_commit_draft().await?;
                Ok(true)
            }
            KeyCode::Char('D') => {
                self.table_data_request_delete().await?;
                Ok(true)
            }
            KeyCode::Char(' ') => {
                self.table_data_toggle_multi_select();
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
