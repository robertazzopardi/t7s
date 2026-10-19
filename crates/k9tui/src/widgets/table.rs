use alloc::{collections::BTreeSet, sync::Arc};
use core::{fmt, marker::PhantomData};

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::Text,
    widgets::{Cell, HighlightSpacing, Row, StatefulWidget, Table, TableState},
};
use unicode_width::UnicodeWidthStr;

use crate::theme;

pub trait TableData {
    #[allow(dead_code)]
    fn title() -> &'static str;
    fn ref_array(&self) -> Vec<String>;
    fn num_columns(&self) -> usize;
    fn cols() -> Vec<&'static str>;

    fn col(&self, column: usize) -> String {
        self.ref_array().get(column).cloned().unwrap_or_default()
    }

    /// Column that absorbs all remaining width (other columns hug their
    /// content); its text is truncated with `…` when too long.
    #[must_use]
    fn fill_column() -> Option<usize> {
        None
    }

    /// UI draft rows (e.g. pending `INSERT`) use this for styling.
    fn is_draft_row(&self) -> bool {
        false
    }

    /// Optional per-cell styling (e.g. environment badge, help keys).
    fn cell_style(&self, _column: usize) -> Option<Style> {
        None
    }

    /// Section header row in help-like tables.
    fn is_section_header(&self) -> bool {
        false
    }
}

/// Model (data) for the table view
#[derive(Clone, Debug, Default)]
pub struct TableModel<T: TableData + Clone> {
    pub items: Vec<T>,
    pub longest_item_lens: Vec<usize>,
    // For RawTableRow, we store column names here
    pub dynamic_column_names: Option<Arc<Vec<String>>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnWidthMode {
    #[default]
    Fit,
    Content,
}

impl ColumnWidthMode {
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Fit => Self::Content,
            Self::Content => Self::Fit,
        }
    }
}

// ponytail: dead until Task 2 render uses these for Fit/Content layout.
#[allow(dead_code)]
const PEEK_COLS: usize = 2;

#[allow(dead_code)]
const fn fit_soft_cap(area_width: usize) -> usize {
    let soft = area_width / 3;
    if soft == 0 { 1 } else { soft }
}

#[allow(dead_code)]
fn capped_lens_for_mode(
    content_lens: &[usize],
    area_width: usize,
    mode: ColumnWidthMode,
) -> Vec<usize> {
    match mode {
        ColumnWidthMode::Content => content_lens.to_vec(),
        ColumnWidthMode::Fit => {
            let hard = area_width.max(1);
            let soft = fit_soft_cap(area_width);
            content_lens
                .iter()
                .map(|&len| len.min(hard).min(soft))
                .collect()
        }
    }
}

/// View state for the table (UI state like selection, scrolling)
#[derive(Clone, Debug, Default)]
pub struct TableViewState {
    pub state: TableState,
    pub column_offset: usize,
    pub column_width_mode: ColumnWidthMode,
}

/// Combined state that holds both model and view state
#[derive(Clone, Debug, Default)]
pub struct TableDataState<T: TableData + Clone> {
    pub model: TableModel<T>,
    pub view: TableViewState,
    /// Row indices toggled with Space (batch operations, e.g. delete). Table data / `RawTableRow`.
    pub multi_row_selection: BTreeSet<usize>,
}

/// Pure stateless table widget - all state is managed externally
#[derive(Clone, Debug)]
pub struct DataTable<T: TableData + Clone>(PhantomData<T>);

impl<T: TableData + Clone> Default for DataTable<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<T: TableData + Clone> TableDataState<T> {
    /// Create a new table state from items
    #[must_use]
    pub fn new(items: Vec<T>) -> Self {
        let longest_item_lens = super::constraint_len_calculator(&items);
        Self {
            model: TableModel {
                items,
                longest_item_lens,
                dynamic_column_names: None,
            },
            view: TableViewState {
                state: TableState::default().with_selected(0),
                column_offset: 0,
                column_width_mode: ColumnWidthMode::Fit,
            },
            multi_row_selection: BTreeSet::new(),
        }
    }

    /// Filter items based on query
    #[must_use]
    pub fn filter(&self, query: &str) -> Vec<T> {
        filter_rows(&self.model.items, query)
    }
}

/// Case-insensitive substring filter over every column of `rows`.
///
/// Free function (rather than a `TableDataState` method) so callers that
/// only have a row slice — not a full table state — can reuse the same
/// matching logic instead of reimplementing it.
#[must_use]
pub fn filter_rows<T: TableData + Clone>(rows: &[T], query: &str) -> Vec<T> {
    if query.is_empty() {
        return rows.to_vec();
    }

    let query_lower = query.to_lowercase();
    rows.iter()
        .filter(|item| {
            // Check if any column contains the query
            for col_idx in 0..item.num_columns() {
                let col_value = item.col(col_idx);
                if col_value.to_lowercase().contains(&query_lower) {
                    return true;
                }
            }
            false
        })
        .cloned()
        .collect()
}

const FILL_MIN_LEN: usize = 10;

/// Truncate to `width` display columns, ending in `…` when cut.
fn truncate_ellipsis(s: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthChar;
    if UnicodeWidthStr::width(s) <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

const fn col_width(len: usize) -> usize {
    len + 1
}

/// First column index to show so that:
/// - if the full table fits, `0` (all columns visible);
/// - otherwise the window contains `selected` and fits as many columns as possible in `area_width`;
/// - on ties, prefers `start` closest to `scroll_hint` (stable scrolling).
fn horizontal_window_start(
    longest_item_lens: &[usize],
    area_width: usize,
    selected: usize,
    scroll_hint: usize,
) -> usize {
    let n = longest_item_lens.len();
    if n == 0 {
        return 0;
    }

    let total: usize = longest_item_lens.iter().map(|&l| col_width(l)).sum();
    if total <= area_width {
        return 0;
    }

    let Some(&sel_len) = longest_item_lens.get(selected) else {
        return 0;
    };
    if col_width(sel_len) > area_width {
        return selected;
    }

    let mut best_start = selected;
    let mut best_count = 0usize;
    let mut best_dist = usize::MAX;

    for start in 0..=selected {
        let mut w = 0usize;
        let mut last = start.saturating_sub(1);
        for (i, &len) in longest_item_lens.iter().enumerate().skip(start) {
            let cw = col_width(len);
            if w + cw > area_width {
                break;
            }
            w += cw;
            last = i;
        }

        if last < selected {
            continue;
        }

        let count = last - start + 1;
        let dist = start.abs_diff(scroll_hint);

        if count > best_count {
            best_count = count;
            best_start = start;
            best_dist = dist;
        } else if count == best_count
            && (dist < best_dist || (dist == best_dist && start < best_start))
        {
            best_start = start;
            best_dist = dist;
        }
    }

    best_start.min(n.saturating_sub(1))
}

/// Visible column indices and optional relative selection index for ratatui's subset table.
fn visible_columns_packed(
    longest_item_lens: &[usize],
    start: usize,
    area_width: usize,
) -> Vec<usize> {
    let mut vis_cols = Vec::new();
    let mut cumulative_width = 0usize;

    for (idx, &len) in longest_item_lens.iter().enumerate().skip(start) {
        let cw = col_width(len);
        if cumulative_width + cw > area_width {
            break;
        }
        cumulative_width += cw;
        vis_cols.push(idx);
    }

    if vis_cols.is_empty() && !longest_item_lens.is_empty() {
        vis_cols.push(start.min(longest_item_lens.len() - 1));
    }

    vis_cols
}

/// Row offset so the selected row stays centered in the viewport instead of
/// hugging the bottom edge (ratatui's stock `TableState` offset only ever
/// increases to keep the selection in view, so once `selected >= visible_rows`
/// it tracks the bottom 1:1). Clamped so it never scrolls past the last page.
fn calculate_row_offset(
    selected: usize,
    total_rows: usize,
    visible_rows: usize,
) -> usize {
    if visible_rows == 0 {
        return 0;
    }
    let max_offset = total_rows.saturating_sub(visible_rows);
    selected.saturating_sub(visible_rows / 2).min(max_offset)
}

/// Helper function to calculate visible columns for `DataTable`
fn calculate_visible_columns_for_table(
    longest_item_lens: &[usize],
    column_offset: usize,
    selected_col_opt: Option<usize>,
    area_width: u16,
) -> (Vec<usize>, Option<usize>, usize) {
    let area_width = area_width as usize;
    let n = longest_item_lens.len();

    let start = selected_col_opt.map_or_else(
        || column_offset.min(n.saturating_sub(1)),
        |selected_col| {
            horizontal_window_start(
                longest_item_lens,
                area_width,
                selected_col,
                column_offset,
            )
        },
    );

    let vis_cols = visible_columns_packed(longest_item_lens, start, area_width);
    let rel = selected_col_opt.map(|selected_col| {
        vis_cols
            .iter()
            .position(|&idx| idx == selected_col)
            .unwrap_or(0)
    });

    (vis_cols, rel, start)
}

impl<T: TableData + fmt::Debug + Clone> StatefulWidget for DataTable<T> {
    type State = TableDataState<T>;

    #[allow(clippy::too_many_lines)]
    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        if state.model.longest_item_lens.is_empty() {
            return;
        }

        // A fill column only needs a small floor to stay in the visible set;
        // it then takes whatever width the other columns leave.
        let fill = T::fill_column();
        let mut lens = state.model.longest_item_lens.clone();
        if let Some(f) = fill.and_then(|f| lens.get_mut(f)) {
            *f = (*f).min(FILL_MIN_LEN);
        }

        let selected_col_opt = state.view.state.selected_column();
        let (visible_cols, relative_selected_col, scroll_start) =
            calculate_visible_columns_for_table(
                &lens,
                state.view.column_offset,
                selected_col_opt,
                area.width,
            );
        state.view.column_offset = scroll_start;

        let original_col = state.view.state.selected_column();
        state.view.state.select_column(relative_selected_col);

        let visible_rows = area.height.saturating_sub(1) as usize; // minus header
        if let Some(selected) = state.view.state.selected() {
            *state.view.state.offset_mut() = calculate_row_offset(
                selected,
                state.model.items.len(),
                visible_rows,
            );
        }

        let (
            selected_row_style,
            selected_col_style,
            selected_cell_style,
            highlight_symbol,
            highlight_spacing,
        ) = create_table_styles();

        // `row_highlight_style` overrides each cell's row style, so a focused row that is also
        // multi-selected would lose the blue background. Patch the default row highlight with blue
        // so the cursor row still reads as "in the multi set" while keeping the same emphasis
        // (reversed, bold) as the normal selection row.
        let row_highlight_style = match state.view.state.selected() {
            Some(i) if state.multi_row_selection.contains(&i) => {
                let style = selected_row_style
                    .patch(Style::new().bg(theme::multi_select_bg()));
                if state
                    .model
                    .items
                    .get(i)
                    .is_some_and(TableData::is_draft_row)
                {
                    style.patch(theme::draft_row())
                } else {
                    style
                }
            }
            _ => selected_row_style,
        };

        // Use dynamic column names if available (for RawTableRow), otherwise use static cols()
        let header = state.model.dynamic_column_names.as_ref().map_or_else(
            || {
                let all_cols = T::cols();
                visible_cols
                    .iter()
                    .map(|&idx| {
                        let col_name =
                            all_cols.get(idx).copied().unwrap_or_default();
                        Cell::from(col_name)
                    })
                    .collect::<Row>()
                    .height(1)
                    .style(theme::header_row())
            },
            |dyn_cols| {
                visible_cols
                    .iter()
                    .map(|&idx| {
                        let col_name =
                            dyn_cols.get(idx).cloned().unwrap_or_default();
                        Cell::from(col_name)
                    })
                    .collect::<Row>()
                    .height(1)
                    .style(theme::header_row())
            },
        );

        let fill_width = fill.map_or(0, |f| {
            let others: usize = visible_cols
                .iter()
                .filter(|&&i| i != f)
                .map(|&i| col_width(lens.get(i).copied().unwrap_or(0)))
                .sum();
            // Table puts 1 cell of spacing between each pair of columns.
            let gaps = visible_cols.len().saturating_sub(1);
            (area.width as usize).saturating_sub(others + gaps).max(1)
        });

        let rows =
            state.model.items.iter().enumerate().map(|(row_idx, data)| {
                let row_data = data.ref_array();
                let mut row_style = Style::new();
                if data.is_section_header() {
                    row_style = theme::accent().add_modifier(Modifier::BOLD);
                } else if data.is_draft_row() {
                    row_style = theme::draft_row();
                }
                if state.multi_row_selection.contains(&row_idx) {
                    row_style = row_style.bg(theme::multi_select_bg());
                }
                visible_cols
                    .iter()
                    .enumerate()
                    .map(|(vis_idx, &idx)| {
                        let value =
                            row_data.get(idx).cloned().unwrap_or_default();
                        let (mut text, cell_style) =
                            format_display_cell(data, idx, value);
                        if fill == Some(idx) {
                            text = truncate_ellipsis(&text, fill_width);
                        }
                        if data.is_draft_row() && vis_idx == 0 {
                            text = format!("~{text}");
                        }
                        let mut cell = Cell::from(text);
                        if let Some(style) = cell_style {
                            cell = cell.style(style);
                        }
                        cell
                    })
                    .collect::<Row>()
                    .style(row_style)
                    .height(1)
            });

        let constraints = visible_cols
            .iter()
            .map(|&idx| {
                if fill == Some(idx) {
                    return Constraint::Fill(1);
                }
                let width = lens.get(idx).unwrap_or(&0) + 1;
                Constraint::Length(u16::try_from(width).unwrap_or(u16::MAX))
            })
            .collect::<Vec<_>>();

        let t = Table::new(rows, constraints)
            .header(header)
            .row_highlight_style(row_highlight_style)
            .column_highlight_style(selected_col_style)
            .cell_highlight_style(selected_cell_style)
            .highlight_symbol(highlight_symbol)
            .highlight_spacing(highlight_spacing);

        StatefulWidget::render(t, area, buf, &mut state.view.state);
        state.view.state.select_column(original_col);
    }
}

// Helper function to create table styles
fn create_table_styles()
-> (Style, Style, Style, Text<'static>, HighlightSpacing) {
    let selected_row_style = theme::selection_row();
    let selected_col_style = theme::selection_col();
    let selected_cell_style = theme::selection_cell();
    // k9s highlights the cursor row as a plain full-width bar — no marker glyph.
    let highlight_symbol = Text::from("");
    (
        selected_row_style,
        selected_col_style,
        selected_cell_style,
        highlight_symbol,
        HighlightSpacing::Never,
    )
}

fn format_display_cell<T: TableData>(
    data: &T,
    column: usize,
    value: String,
) -> (String, Option<Style>) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        // Blank optional fields (e.g. Auth) — don't paint a null placeholder.
        return (String::new(), None);
    }
    if trimmed.eq_ignore_ascii_case("null") {
        return ("·".to_string(), Some(theme::null_cell()));
    }
    (value, data.cell_style(column))
}

#[cfg(test)]
mod tests {
    use core::fmt;

    use ratatui::{Terminal, backend::TestBackend, buffer};

    use super::*;

    #[derive(Debug, Clone)]
    struct Item {
        name: &'static str,
        value: &'static str,
    }

    impl TableData for Item {
        fn title() -> &'static str {
            "items"
        }
        fn ref_array(&self) -> Vec<String> {
            vec![self.name.to_string(), self.value.to_string()]
        }
        fn num_columns(&self) -> usize {
            2
        }
        fn cols() -> Vec<&'static str> {
            vec!["Name", "Value"]
        }
    }

    fn items() -> Vec<Item> {
        vec![
            Item {
                name: "alpha",
                value: "1",
            },
            Item {
                name: "beta",
                value: "2",
            },
        ]
    }

    #[test]
    fn horizontal_window_start_shows_all_columns_when_fits() {
        let lens = vec![5, 5];
        assert_eq!(horizontal_window_start(&lens, 80, 0, 0), 0);
    }

    #[test]
    fn horizontal_window_start_keeps_selected_visible() {
        let lens = vec![5, 5, 5, 5];
        // Narrow area: only ~2 columns fit, selecting the last must scroll to include it.
        let start = horizontal_window_start(&lens, 12, 3, 0);
        let visible = visible_columns_packed(&lens, start, 12);
        assert!(visible.contains(&3));
    }

    #[test]
    fn visible_columns_packed_always_shows_at_least_one() {
        // Column wider than the area still yields a non-empty visible set.
        let lens = vec![100];
        let visible = visible_columns_packed(&lens, 0, 10);
        assert_eq!(visible, vec![0]);
    }

    #[test]
    fn row_offset_centers_selection_instead_of_hugging_bottom() {
        // 100 rows, 10 visible: selecting row 50 should center it (offset 45),
        // not pin it to the bottom (offset 41).
        assert_eq!(calculate_row_offset(50, 100, 10), 45);
        // Near the end of the list, offset clamps so it never scrolls past
        // the last page.
        assert_eq!(calculate_row_offset(99, 100, 10), 90);
        // Near the start, offset never goes negative.
        assert_eq!(calculate_row_offset(2, 100, 10), 0);
    }

    #[test]
    fn renders_header_and_rows_into_buffer() {
        let mut state = TableDataState::new(items());
        let backend = TestBackend::new(20, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                frame.render_stateful_widget(
                    DataTable::<Item>::default(),
                    frame.area(),
                    &mut state,
                );
            })
            .unwrap();

        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(buffer::Cell::symbol)
            .collect();

        assert!(content.contains("Name"));
        assert!(content.contains("alpha"));
        assert!(content.contains("beta"));
    }

    fn render_lines<T: TableData + fmt::Debug + Clone>(
        items: Vec<T>,
        width: u16,
        height: u16,
    ) -> Vec<String> {
        let mut state = TableDataState::new(items);
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_stateful_widget(
                    DataTable::<T>::default(),
                    frame.area(),
                    &mut state,
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        buf.content()
            .chunks(usize::from(width.max(1)))
            .map(|row| row.iter().map(buffer::Cell::symbol).collect::<String>())
            .collect()
    }

    #[derive(Debug, Clone)]
    struct Filled(String);

    impl TableData for Filled {
        fn title() -> &'static str {
            "filled"
        }
        fn ref_array(&self) -> Vec<String> {
            vec!["a".to_string(), self.0.clone()]
        }
        fn num_columns(&self) -> usize {
            2
        }
        fn cols() -> Vec<&'static str> {
            vec!["A", "Fill"]
        }
        fn fill_column() -> Option<usize> {
            Some(1)
        }
    }

    #[derive(Debug, Clone)]
    struct Five(String);

    impl TableData for Five {
        fn title() -> &'static str {
            "five"
        }
        fn ref_array(&self) -> Vec<String> {
            vec![
                "a".into(),
                "b".into(),
                "c".into(),
                "d".into(),
                self.0.clone(),
            ]
        }
        fn num_columns(&self) -> usize {
            5
        }
        fn cols() -> Vec<&'static str> {
            vec!["A", "B", "C", "D", "Fill"]
        }
        fn fill_column() -> Option<usize> {
            Some(4)
        }
    }

    #[test]
    fn fill_ellipsis_lands_at_right_edge_with_many_columns() {
        let lines = render_lines(vec![Five("x".repeat(200))], 40, 3);
        let row = lines.get(1).unwrap();
        assert_eq!(row.chars().count(), 40);
        assert!(row.ends_with('…'), "{row:?}");
    }

    #[test]
    fn truncate_ellipsis_counts_display_width() {
        assert_eq!(truncate_ellipsis("abc", 3), "abc");
        assert_eq!(truncate_ellipsis("abcdef", 4), "abc…");
        assert_eq!(truncate_ellipsis("abcdef", 1), "…");
        // width 0 still yields the ellipsis (callers clamp to >= 1)
        assert_eq!(truncate_ellipsis("abc", 0), "…");
        // wide chars (width 2) never overflow: 2 + 1 for the ellipsis
        assert_eq!(truncate_ellipsis("日本語日本語", 5), "日本…");
        assert_eq!(truncate_ellipsis("日本語", 6), "日本語");
        // emoji
        assert_eq!(truncate_ellipsis("😀😀😀😀", 4), "😀…");
        assert_eq!(truncate_ellipsis("😀😀", 1), "…");
        assert_eq!(truncate_ellipsis("", 0), "");
    }

    #[test]
    fn fill_column_default_none_and_other_layout_unchanged() {
        assert_eq!(Item::fill_column(), None);
        // Fixed layout for a table without a fill column (matches main).
        assert_eq!(
            render_lines(items(), 20, 4),
            vec![
                "Name   Value        ",
                "alpha  1            ",
                "beta   2            ",
                "                    ",
            ]
        );
        assert_eq!(
            render_lines(items(), 8, 3),
            vec!["Name    ", "alpha   ", "beta    "]
        );
        assert_eq!(render_lines(items(), 3, 3), vec!["Nam", "alp", "bet"]);
    }

    #[test]
    fn fill_column_takes_remaining_width_and_ellipsizes() {
        let long = "x".repeat(200);
        let lines = render_lines(vec![Filled(long)], 40, 3);
        let row = lines.get(1).unwrap();
        assert!(row.starts_with("a "), "{row:?}");
        assert!(row.trim_end().ends_with('…'), "{row:?}");
        // fills (almost) to the right edge
        assert!(row.trim_end().chars().count() >= 38, "{row:?}");
        let short = render_lines(vec![Filled("hi".into())], 40, 3);
        assert!(short.get(1).unwrap().contains("a  hi"), "{short:?}");
    }

    #[test]
    fn fill_column_wide_chars_stay_within_area() {
        for w in [14u16, 20, 33] {
            let lines = render_lines(vec![Filled("😀".repeat(50))], w, 3);
            let row = lines.get(1).unwrap();
            assert!(row.trim_end().ends_with('…'), "{row:?}");
        }
    }

    #[test]
    fn fill_column_tiny_areas_do_not_panic() {
        for w in 0u16..=12 {
            for h in 0u16..=3 {
                let _ = render_lines(vec![Filled("x".repeat(80))], w, h);
                let _ = render_lines(vec![Filled(String::new())], w, h);
            }
        }
    }

    #[test]
    fn column_width_mode_defaults_to_fit_and_toggles() {
        assert_eq!(ColumnWidthMode::default(), ColumnWidthMode::Fit);
        assert_eq!(ColumnWidthMode::Fit.toggle(), ColumnWidthMode::Content);
        assert_eq!(ColumnWidthMode::Content.toggle(), ColumnWidthMode::Fit);
    }

    #[test]
    fn capped_lens_fit_hard_and_soft_caps() {
        // area 30 → soft = 10; hard = 30
        let content = vec![5usize, 100, 8];
        let capped = capped_lens_for_mode(&content, 30, ColumnWidthMode::Fit);
        assert_eq!(capped, vec![5, 10, 8]);
    }

    #[test]
    fn capped_lens_fit_never_exceeds_viewport() {
        let content = vec![500usize];
        let capped = capped_lens_for_mode(&content, 20, ColumnWidthMode::Fit);
        assert!(capped[0] <= 20);
        assert_eq!(capped[0], fit_soft_cap(20)); // 20/3 = 6
    }

    #[test]
    fn capped_lens_content_is_identity() {
        let content = vec![5usize, 100, 8];
        assert_eq!(
            capped_lens_for_mode(&content, 30, ColumnWidthMode::Content),
            content
        );
    }

    #[test]
    fn table_view_state_defaults_to_fit() {
        assert_eq!(
            TableViewState::default().column_width_mode,
            ColumnWidthMode::Fit
        );
    }
}

#[cfg(test)]
mod filter_rows_tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Row(&'static str, &'static str);

    impl TableData for Row {
        fn title() -> &'static str {
            "rows"
        }
        fn ref_array(&self) -> Vec<String> {
            vec![self.0.to_string(), self.1.to_string()]
        }
        fn num_columns(&self) -> usize {
            2
        }
        fn cols() -> Vec<&'static str> {
            vec!["a", "b"]
        }
    }

    fn rows() -> Vec<Row> {
        vec![
            Row("Alpha", "one"),
            Row("beta", "TWO"),
            Row("gamma", "three"),
        ]
    }

    #[test]
    fn empty_query_returns_all() {
        assert_eq!(filter_rows(&rows(), ""), rows());
    }

    #[test]
    fn matches_case_insensitively_in_any_column() {
        assert_eq!(filter_rows(&rows(), "ALPHA"), vec![Row("Alpha", "one")]);
        assert_eq!(filter_rows(&rows(), "two"), vec![Row("beta", "TWO")]);
    }

    #[test]
    fn substring_across_rows_and_no_match() {
        assert_eq!(filter_rows(&rows(), "a").len(), 3);
        assert_eq!(filter_rows(&rows(), "zzz").len(), 0);
    }
}
