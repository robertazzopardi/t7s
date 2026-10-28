use crossterm::event::KeyCode;
use k9tui::widgets::{
    navigation::TableNavigationHandler,
    table::{TableData, TableDataState, filter_rows},
};

/// A wrapper for managing filtered data with original data preservation
#[derive(Clone, Default)]
pub struct FilteredData<T: TableData + Clone> {
    /// Original unfiltered data
    pub original: Vec<T>,
    /// Table state with potentially filtered items
    pub table: TableDataState<T>,
    /// Last applied filter query, restored when `/` reopens the bar
    pub query: String,
}

impl<T: TableData + Clone> FilteredData<T> {
    /// Create a new `FilteredData` from a vector of items
    pub fn new(data: Vec<T>) -> Self {
        Self {
            original: data.clone(),
            table: TableDataState::new(data),
            query: String::new(),
        }
    }

    /// Apply a filter to the data
    pub fn apply_filter(&mut self, query: &str) {
        // Filter from `original` so deleting characters widens the result.
        self.table.model.items = filter_rows(&self.original, query);
        query.clone_into(&mut self.query);
        self.select_first_row();
    }

    /// Clear the filter and restore original data
    pub fn clear_filter(&mut self) {
        self.table.model.items.clone_from(&self.original);
        self.query.clear();
        self.select_first_row();
    }

    /// Cursor to the top row (k9s-style). An empty result clears the
    /// selection, so it must be re-set here or j/k stay dead afterwards.
    fn select_first_row(&mut self) {
        self.table.multi_row_selection.clear();
        let first = (!self.table.model.items.is_empty()).then_some(0);
        self.table.view.state.select(first);
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

#[cfg(test)]
mod tests {
    use super::FilteredData;
    use crate::db::Table;

    fn names(fd: &FilteredData<Table>) -> Vec<&str> {
        fd.table
            .model
            .items
            .iter()
            .map(|t| t.name.as_str())
            .collect()
    }

    #[test]
    fn shortening_query_restores_rows() {
        let mut fd = FilteredData::new(
            ["users", "orders"]
                .iter()
                .map(|n| Table {
                    name: (*n).to_string(),
                    schema: "main".into(),
                    size: None,
                })
                .collect(),
        );
        fd.apply_filter("us");
        assert_eq!(names(&fd), ["users"]);
        fd.apply_filter("");
        assert_eq!(names(&fd), ["users", "orders"]);
    }

    #[test]
    fn no_match_then_match_reselects_a_row() {
        let mut fd = FilteredData::new(
            ["users", "orders"]
                .iter()
                .map(|n| Table {
                    name: (*n).to_string(),
                    schema: "main".into(),
                    size: None,
                })
                .collect(),
        );
        fd.apply_filter("zzz");
        assert_eq!(fd.table.view.state.selected(), None);
        fd.apply_filter("or");
        assert_eq!(fd.table.view.state.selected(), Some(0));
    }
}
