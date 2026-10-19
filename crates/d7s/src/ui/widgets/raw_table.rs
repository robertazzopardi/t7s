use alloc::sync::Arc;

use k9tui::widgets::table::{TableData, TableDataState};

use crate::db::DbRowId;

/// A wrapper type for raw table data with dynamic column names
#[derive(Clone, Debug, Default)]
pub struct RawTableRow {
    pub values: Vec<String>,
    #[allow(dead_code)]
    pub column_names: Arc<Vec<String>>,
    /// Set when rows are loaded from a concrete DB table (for `UPDATE`).
    pub db_row_id: Option<DbRowId>,
    /// Pending row not yet `INSERT`ed; edited locally until commit (`s`).
    pub is_draft: bool,
}

impl TableData for RawTableRow {
    fn title() -> &'static str {
        "Table Data"
    }

    fn ref_array(&self) -> Vec<String> {
        self.values.clone()
    }

    fn num_columns(&self) -> usize {
        self.values.len()
    }

    fn cols() -> Vec<&'static str> {
        // This is a limitation - we can't return dynamic column names from a static method
        // We'll handle this specially in DataTable's render method
        vec![]
    }

    fn is_draft_row(&self) -> bool {
        self.is_draft
    }
}

/// `TableDataState<RawTableRow>` is a foreign type (`TableDataState` lives in `k9tui`), so these
/// `RawTableRow`-specific operations are added via an extension trait rather than an inherent
/// `impl` block (the orphan rules forbid inherent impls on foreign generic types even when the
/// type parameter is local).
pub trait RawTableStateExt {
    fn reset(
        &mut self,
        items: Vec<Vec<String>>,
        column_names: &[String],
        row_ids: Option<Vec<Option<DbRowId>>>,
    );

    fn recompute_column_widths(&mut self);
}

impl RawTableStateExt for TableDataState<RawTableRow> {
    /// Reset the table state with new raw data
    fn reset(
        &mut self,
        items: Vec<Vec<String>>,
        column_names: &[String],
        row_ids: Option<Vec<Option<DbRowId>>>,
    ) {
        let column_names_arc = Arc::new(column_names.to_owned());
        let row_ids = row_ids.filter(|r| r.len() == items.len());
        let raw_rows: Vec<RawTableRow> = items
            .into_iter()
            .enumerate()
            .map(|(i, values)| RawTableRow {
                values,
                column_names: Arc::clone(&column_names_arc),
                db_row_id: row_ids
                    .as_ref()
                    .and_then(|r| r.get(i))
                    .cloned()
                    .flatten(),
                is_draft: false,
            })
            .collect();
        let longest_item_lens =
            constraint_len_calculator_for_raw_data(&raw_rows, column_names);

        self.model.items = raw_rows;
        self.model.longest_item_lens = longest_item_lens;
        self.model.dynamic_column_names = Some(column_names_arc);
        self.view.state.select(Some(0));
        self.view.column_offset = 0;
        self.multi_row_selection.clear();
    }

    /// Recompute column display widths after cell text changes.
    fn recompute_column_widths(&mut self) {
        let Some(names) = self.model.dynamic_column_names.as_deref() else {
            return;
        };
        self.model.longest_item_lens =
            constraint_len_calculator_for_raw_data(&self.model.items, names);
    }
}

// Helper function to calculate constraints for raw table data
fn constraint_len_calculator_for_raw_data(
    items: &[RawTableRow],
    column_names: &[String],
) -> Vec<usize> {
    use unicode_width::UnicodeWidthStr;

    let mut longest_lens = column_names
        .iter()
        .map(|name| UnicodeWidthStr::width(name.as_str()))
        .collect::<Vec<usize>>();

    for item in items {
        for (i, value) in item.values.iter().enumerate() {
            if i < longest_lens.len() {
                let max_width = value
                    .lines()
                    .map(UnicodeWidthStr::width)
                    .max()
                    .unwrap_or(0);

                if let Some(longest_len) = longest_lens.get_mut(i) {
                    *longest_len = (*longest_len).max(max_width);
                }
            }
        }
    }

    longest_lens
}
