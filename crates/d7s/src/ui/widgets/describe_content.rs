//! k9s-style "describe" view: a field/value listing for whatever object
//! (connection, table, column, or row) is currently selected.

use k9tui::theme;
pub use k9tui::widgets::table::TableData;
use ratatui::style::{Modifier, Style};

/// One row in the describe table: a field label and its value.
#[derive(Clone, Debug)]
pub struct DescribeRow {
    pub field: String,
    pub value: String,
    pub section_header: bool,
}

impl DescribeRow {
    #[must_use]
    pub fn new<F: Into<String>, V: Into<String>>(field: F, value: V) -> Self {
        Self {
            field: field.into(),
            value: value.into(),
            section_header: false,
        }
    }

    #[must_use]
    pub fn section<T: Into<String>>(title: T) -> Self {
        Self {
            field: title.into(),
            value: String::new(),
            section_header: true,
        }
    }
}

impl TableData for DescribeRow {
    fn title() -> &'static str {
        "Describe"
    }

    fn ref_array(&self) -> Vec<String> {
        vec![self.field.clone(), self.value.clone()]
    }

    fn num_columns(&self) -> usize {
        2
    }

    fn cols() -> Vec<&'static str> {
        vec!["Field", "Value"]
    }

    fn is_section_header(&self) -> bool {
        self.section_header
    }

    fn cell_style(&self, column: usize) -> Option<Style> {
        if self.section_header {
            return Some(theme::accent().add_modifier(Modifier::BOLD));
        }
        if column == 0 {
            Some(theme::accent())
        } else {
            None
        }
    }
}
