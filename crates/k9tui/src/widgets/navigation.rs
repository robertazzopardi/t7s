use ratatui::widgets::TableState;

use crate::{
    keymap::Nav,
    widgets::table::{TableData, TableDataState, TableModel, TableViewState},
};

/// Helper for table navigation operations
pub struct TableNavigationHandler;

impl TableNavigationHandler {
    /// Wraps the selection for a `TableState` - going past the end wraps to the beginning and vice versa
    pub const fn wrap_rows<T: TableData>(state: &mut TableState, items: &[T]) {
        if let Some(selected) = state.selected() {
            if items.is_empty() {
                state.select(None);
            } else if selected == items.len() {
                // Past the end - wrap to beginning
                state.select_first();
            } else if selected > items.len() {
                // Underflow (wrapped from 0) - wrap to end
                state.select_last();
            }
        }
    }

    /// Wraps the column selection for a `TableState` - going past the end wraps to the beginning and vice versa
    #[allow(dead_code)]
    pub fn wrap_columns<T: TableData>(
        state: &mut TableState,
        items: &[T],
        column_offset: &mut usize,
    ) {
        let num_columns =
            items.first().map_or_else(|| 0, TableData::num_columns);

        if num_columns == 0 {
            state.select_column(None);
            *column_offset = 0;
            return;
        }

        // Wrap selected column
        if let Some(selected_col) = state.selected_column()
            && selected_col >= num_columns
        {
            // Past the end or underflow - wrap to beginning
            state.select_column(Some(0));
        }

        // Clamp column offset (offset doesn't wrap, just clamps)
        if *column_offset >= num_columns {
            *column_offset = num_columns.saturating_sub(1);
        }
    }

    /// Generic table navigation handler for any `TableState`
    pub fn navigate_table<T: TableData + Clone>(
        model: &TableModel<T>,
        view: &mut TableViewState,
        nav: Nav,
    ) {
        match nav {
            Nav::Down => {
                if let Some(selected) = view.state.selected()
                    && !model.items.is_empty()
                {
                    if selected >= model.items.len() - 1 {
                        view.state.select(Some(0));
                    } else {
                        view.state.select_next();
                    }
                }
            }
            Nav::Up => {
                if let Some(selected) = view.state.selected()
                    && !model.items.is_empty()
                {
                    if selected == 0 {
                        view.state.select(Some(model.items.len() - 1));
                    } else {
                        view.state.select_previous();
                    }
                }
            }
            Nav::Left => {
                let num_cols =
                    model.items.first().map_or(0, TableData::num_columns);
                if num_cols == 0 {
                    return;
                }

                if let Some(selected_col) = view.state.selected_column() {
                    if selected_col == 0 {
                        view.state.select_column(Some(num_cols - 1));
                    } else {
                        view.state.select_previous_column();
                    }
                } else {
                    view.state.select_column(Some(num_cols - 1));
                }
            }
            Nav::Right => {
                let num_cols =
                    model.items.first().map_or(0, TableData::num_columns);
                if num_cols == 0 {
                    return;
                }

                if let Some(selected_col) = view.state.selected_column() {
                    if selected_col + 1 >= num_cols {
                        view.state.select_column(Some(0));
                    } else {
                        view.state.select_next_column();
                    }
                } else {
                    view.state.select_column(Some(0));
                }
            }
            Nav::Top => {
                view.state.select(Some(0));
                Self::wrap_rows(&mut view.state, &model.items);
                view.column_offset = 0;
            }
            Nav::Bottom => {
                if !model.items.is_empty() {
                    view.state.select(Some(model.items.len() - 1));
                }
            }
            // Handled by views that scroll, or not implemented for tables.
            Nav::FirstColumn
            | Nav::LastColumn
            | Nav::PageUp
            | Nav::PageDown => {}
        }
    }

    /// Handles navigation for table data widget
    #[allow(dead_code)]
    pub fn navigate<T: TableData + Clone>(
        table_data: &mut Option<TableDataState<T>>,
        nav: Nav,
    ) {
        if let Some(table) = table_data {
            Self::navigate_table(&table.model, &mut table.view, nav);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::table::TableDataState;

    #[derive(Debug, Clone)]
    struct Row(&'static str);

    impl TableData for Row {
        fn title() -> &'static str {
            "rows"
        }
        fn ref_array(&self) -> Vec<String> {
            vec![self.0.to_string()]
        }
        fn num_columns(&self) -> usize {
            1
        }
        fn cols() -> Vec<&'static str> {
            vec!["col"]
        }
    }

    fn rows(n: usize) -> Vec<Row> {
        const NAMES: [&str; 3] = ["a", "b", "c"];
        NAMES.iter().take(n).map(|&name| Row(name)).collect()
    }

    #[test]
    fn wrap_rows_selects_first_past_end() {
        let items = rows(3);
        let mut state = TableState::default().with_selected(3);
        TableNavigationHandler::wrap_rows(&mut state, &items);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn wrap_rows_selects_none_when_empty() {
        let items: Vec<Row> = vec![];
        let mut state = TableState::default().with_selected(0);
        TableNavigationHandler::wrap_rows(&mut state, &items);
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn navigate_down_wraps_to_top_from_last_row() {
        let mut table = TableDataState::new(rows(3));
        table.view.state.select(Some(2));
        TableNavigationHandler::navigate_table(
            &table.model,
            &mut table.view,
            Nav::Down,
        );
        assert_eq!(table.view.state.selected(), Some(0));
    }

    #[test]
    fn navigate_up_wraps_to_bottom_from_first_row() {
        let mut table = TableDataState::new(rows(3));
        table.view.state.select(Some(0));
        TableNavigationHandler::navigate_table(
            &table.model,
            &mut table.view,
            Nav::Up,
        );
        assert_eq!(table.view.state.selected(), Some(2));
    }

    #[test]
    fn navigate_g_jumps_to_top() {
        let mut table = TableDataState::new(rows(3));
        table.view.state.select(Some(2));
        TableNavigationHandler::navigate_table(
            &table.model,
            &mut table.view,
            Nav::Top,
        );
        assert_eq!(table.view.state.selected(), Some(0));
    }

    #[test]
    fn navigate_shift_g_jumps_to_bottom() {
        let mut table = TableDataState::new(rows(3));
        table.view.state.select(Some(0));
        TableNavigationHandler::navigate_table(
            &table.model,
            &mut table.view,
            Nav::Bottom,
        );
        assert_eq!(table.view.state.selected(), Some(2));
    }
}
