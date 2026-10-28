use super::filtered_data::FilteredData;
use crate::{app::App, app_state::DatabaseExplorerState};

impl App<'_> {
    pub fn apply_filter(&mut self) {
        let Some(search_filter) = &self.search_filter.clone() else {
            return;
        };
        let Some(query) = search_filter.lines().first() else {
            return;
        };
        self.apply_filter_with_query(query);
    }

    pub fn has_active_filter(&self) -> bool {
        let explorer = &self.database_explorer;
        match &explorer.state {
            DatabaseExplorerState::Connections => {
                self.database_explorer.connections.is_filtered()
            }
            DatabaseExplorerState::Databases => explorer
                .databases
                .as_ref()
                .is_some_and(FilteredData::is_filtered),
            DatabaseExplorerState::Schemas => explorer
                .schemas
                .as_ref()
                .is_some_and(FilteredData::is_filtered),
            DatabaseExplorerState::Tables(_) => explorer
                .tables
                .as_ref()
                .is_some_and(FilteredData::is_filtered),
            DatabaseExplorerState::Columns(_, _) => explorer
                .columns
                .as_ref()
                .is_some_and(FilteredData::is_filtered),
            DatabaseExplorerState::TableData(_, _) => explorer
                .table_data
                .as_ref()
                .is_some_and(FilteredData::is_filtered),
            DatabaseExplorerState::SqlResults(_) => false,
        }
    }

    /// Non-empty query of the current view's filter, if any.
    pub fn active_filter_query(&self) -> Option<&str> {
        let explorer = &self.database_explorer;
        let query = match &explorer.state {
            DatabaseExplorerState::Connections => {
                Some(&explorer.connections.query)
            }
            DatabaseExplorerState::Databases => {
                explorer.databases.as_ref().map(|d| &d.query)
            }
            DatabaseExplorerState::Schemas => {
                explorer.schemas.as_ref().map(|d| &d.query)
            }
            DatabaseExplorerState::Tables(_) => {
                explorer.tables.as_ref().map(|d| &d.query)
            }
            DatabaseExplorerState::Columns(_, _) => {
                explorer.columns.as_ref().map(|d| &d.query)
            }
            DatabaseExplorerState::TableData(_, _) => {
                explorer.table_data.as_ref().map(|d| &d.query)
            }
            DatabaseExplorerState::SqlResults(_) => None,
        };
        query.map(String::as_str).filter(|q| !q.is_empty())
    }

    pub fn filter_match_count(&self) -> usize {
        let explorer = &self.database_explorer;
        match &explorer.state {
            DatabaseExplorerState::Connections => {
                explorer.connections.table.model.items.len()
            }
            DatabaseExplorerState::Databases => explorer
                .databases
                .as_ref()
                .map_or(0, |d| d.table.model.items.len()),
            DatabaseExplorerState::Schemas => explorer
                .schemas
                .as_ref()
                .map_or(0, |d| d.table.model.items.len()),
            DatabaseExplorerState::Tables(_) => explorer
                .tables
                .as_ref()
                .map_or(0, |d| d.table.model.items.len()),
            DatabaseExplorerState::Columns(_, _) => explorer
                .columns
                .as_ref()
                .map_or(0, |d| d.table.model.items.len()),
            DatabaseExplorerState::TableData(_, _) => explorer
                .table_data
                .as_ref()
                .map_or(0, |d| d.table.model.items.len()),
            DatabaseExplorerState::SqlResults(_) => 0,
        }
    }

    pub fn clear_filter(&mut self) {
        if self.has_active_filter() || self.active_filter_query().is_some() {
            let explorer = &mut self.database_explorer;
            match explorer.state {
                DatabaseExplorerState::Connections => {
                    self.database_explorer.connections.clear_filter();
                }
                DatabaseExplorerState::Databases => {
                    if let Some(ref mut databases) = explorer.databases {
                        databases.clear_filter();
                    }
                }
                DatabaseExplorerState::Schemas => {
                    if let Some(ref mut schemas) = explorer.schemas {
                        schemas.clear_filter();
                    }
                }
                DatabaseExplorerState::Tables(_) => {
                    if let Some(ref mut tables) = explorer.tables {
                        tables.clear_filter();
                    }
                }
                DatabaseExplorerState::Columns(_, _) => {
                    if let Some(ref mut columns) = explorer.columns {
                        columns.clear_filter();
                    }
                }
                DatabaseExplorerState::TableData(_, _) => {
                    if let Some(ref mut table_data) = explorer.table_data {
                        table_data.clear_filter();
                    }
                }
                DatabaseExplorerState::SqlResults(_) => {}
            }
            self.set_status("Filter cleared");
        }
    }

    pub(crate) fn apply_filter_with_query(&mut self, query: &str) {
        let explorer = &mut self.database_explorer;
        match explorer.state {
            DatabaseExplorerState::Connections => {
                self.database_explorer.connections.apply_filter(query);
            }
            DatabaseExplorerState::Databases => {
                if let Some(ref mut databases) = explorer.databases {
                    databases.apply_filter(query);
                }
            }
            DatabaseExplorerState::Schemas => {
                if let Some(ref mut schemas) = explorer.schemas {
                    schemas.apply_filter(query);
                }
            }
            DatabaseExplorerState::Tables(_) => {
                if let Some(ref mut tables) = explorer.tables {
                    tables.apply_filter(query);
                }
            }
            DatabaseExplorerState::Columns(_, _) => {
                if let Some(ref mut columns) = explorer.columns {
                    columns.apply_filter(query);
                }
            }
            DatabaseExplorerState::TableData(_, _) => {
                if let Some(ref mut table_data) = explorer.table_data {
                    table_data.apply_filter(query);
                }
            }
            DatabaseExplorerState::SqlResults(_) => {}
        }

        if !query.trim().is_empty() {
            let n = self.filter_match_count();
            let label = if n == 1 { "match" } else { "matches" };
            self.set_status(format!("{n} {label}"));
        }
    }
}
