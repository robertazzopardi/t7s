use ratatui::style::{Modifier, Style};

use crate::{theme, widgets::table::TableData};

/// One row in the help table: key label and description.
#[derive(Clone, Debug)]
pub struct HelpRow {
    pub key: &'static str,
    pub desc: &'static str,
    pub section_header: bool,
}

impl TableData for HelpRow {
    fn title() -> &'static str {
        "Help"
    }

    fn ref_array(&self) -> Vec<String> {
        vec![self.key.to_string(), self.desc.to_string()]
    }

    fn num_columns(&self) -> usize {
        2
    }

    fn cols() -> Vec<&'static str> {
        vec!["Key", "Description"]
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
            Some(theme::muted())
        }
    }
}
