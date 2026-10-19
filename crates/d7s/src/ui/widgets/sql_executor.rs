use k9tui::{
    theme,
    widgets::table::{DataTable, TableDataState},
};
use ratatui::{
    prelude::*,
    text::{Line, Span},
    widgets::{Paragraph, StatefulWidget, Wrap},
};
use ratatui_textarea::{CursorMove, TextArea};

use crate::ui::widgets::raw_table::{RawTableRow, RawTableStateExt};

/// State for the SQL executor widget
#[derive(Debug, Clone)]
pub struct SqlExecutorState {
    input: TextArea<'static>,
    pub results: Option<Vec<Vec<String>>>,
    pub column_names: Vec<String>,
    pub error_message: Option<String>,
    selected_statement: Option<String>,
    pub is_active: bool,
    pub table_state: TableDataState<RawTableRow>,
}

impl Default for SqlExecutorState {
    fn default() -> Self {
        let mut input = TextArea::default();
        input.set_cursor_line_style(Style::default());
        input.set_cursor_style(Style::default());
        input.set_max_histories(0);
        Self {
            input,
            results: None,
            column_names: Vec::new(),
            error_message: None,
            selected_statement: None,
            is_active: false,
            table_state: TableDataState::default(),
        }
    }
}

impl SqlExecutorState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub const fn deactivate(&mut self) {
        self.is_active = false;
    }

    pub fn set_results(
        &mut self,
        results: Vec<Vec<String>>,
        column_names: &[String],
    ) {
        self.results = Some(results.clone());
        self.column_names.clone_from(&column_names.to_vec());
        self.error_message = None;
        self.table_state.reset(results, column_names, None);
    }

    #[allow(dead_code)]
    pub fn set_error(&mut self, error: String) {
        self.error_message = Some(error);
        self.results = None;
    }

    pub fn clear_results(&mut self) {
        self.results = None;
        self.column_names.clear();
        self.error_message = None;
        self.table_state.reset(vec![], &[], None);
    }

    pub fn set_sql(&mut self, sql: &str) {
        let lines: Vec<String> = sql.lines().map(String::from).collect();
        self.input = TextArea::new(if lines.is_empty() {
            vec![String::new()]
        } else {
            lines
        });
        self.input.set_cursor_line_style(Style::default());
        self.input.set_cursor_style(Style::default());
        self.input.set_max_histories(0);
        self.input.move_cursor(CursorMove::Bottom);
        self.input.move_cursor(CursorMove::End);
        self.selected_statement = None;
    }

    #[must_use]
    pub fn sql_input(&self) -> String {
        self.input.lines().join("\n")
    }

    pub fn set_selected_statement<S: Into<String>>(&mut self, statement: S) {
        self.selected_statement = Some(statement.into());
    }

    #[must_use]
    pub fn selected_statement(&self) -> Option<&str> {
        self.selected_statement.as_deref()
    }
}

fn idle_hint_lines() -> Line<'static> {
    Line::from(vec![
        Span::raw("Press "),
        Span::styled("e", theme::accent()),
        Span::raw(" to open SQL editor  ·  Press "),
        Span::styled("E", theme::accent()),
        Span::raw(" to run"),
    ])
}

/// SQL executor widget, which is stateless, only handles rendering
pub struct SqlExecutor;

impl StatefulWidget for SqlExecutor {
    type State = SqlExecutorState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        if let Some(error) = &state.error_message {
            Paragraph::new(error.clone())
                .style(theme::error())
                .wrap(Wrap { trim: true })
                .render(area, buf);
        } else if let Some(results) = &state.results {
            if results.is_empty() {
                Paragraph::new("No results")
                    .style(theme::muted())
                    .render(area, buf);
            } else {
                DataTable::<RawTableRow>::default().render(
                    area,
                    buf,
                    &mut state.table_state,
                );
            }
        } else {
            idle_hint_lines().centered().render(area, buf);
        }
    }
}
