use crossterm::event::KeyCode;
use k9tui::widgets::{
    navigation::TableNavigationHandler,
    table::{TableData, TableDataState},
};

/// A wrapper for managing filtered data with original data preservation
#[derive(Clone, Default)]
pub struct FilteredData<T: TableData + Clone> {
    /// Original unfiltered data
    pub original: Vec<T>,
    /// Table state with potentially filtered items
    pub table: TableDataState<T>,
}

impl<T: TableData + Clone> FilteredData<T> {
    /// Create a new `FilteredData` from a vector of items
    pub fn new(data: Vec<T>) -> Self {
        Self {
            original: data.clone(),
            table: TableDataState::new(data),
        }
    }

    /// Apply a filter to the data
    pub fn apply_filter(&mut self, query: &str) {
        self.table.model.items = self.table.filter(query);
        self.table.multi_row_selection.clear();
        TableNavigationHandler::wrap_rows(
            &mut self.table.view.state,
            &self.table.model.items,
        );
    }

    /// Clear the filter and restore original data
    pub fn clear_filter(&mut self) {
        self.table.model.items.clone_from(&self.original);
        self.table.multi_row_selection.clear();
        TableNavigationHandler::wrap_rows(
            &mut self.table.view.state,
            &self.table.model.items,
        );
    }

    /// Navigate the table using a key event
    pub fn navigate(&mut self, key: KeyCode) {
        TableNavigationHandler::navigate_table(
            &self.table.model,
            &mut self.table.view,
            key,
        );
    }

    /// Check if the data is currently filtered
    pub const fn is_filtered(&self) -> bool {
        self.table.model.items.len() != self.original.len()
    }
}
