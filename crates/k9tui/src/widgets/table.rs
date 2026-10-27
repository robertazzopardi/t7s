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

impl fmt::Display for ColumnWidthMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Fit => "Fit",
            Self::Content => "Content",
        })
    }
}

/// Width of the partial-column strips shown at the edges under Fit.
const PEEK_COLS: usize = 2;

const fn fit_soft_cap(area_width: usize) -> usize {
    let soft = area_width / 3;
    if soft == 0 { 1 } else { soft }
}

/// Fit soft cap, in place; returns the capped total. The soft cap is at most
/// `area_width`, so it doubles as the hard (viewport) cap.
fn cap_fit_lens(lens: &mut [usize], area_width: usize) -> usize {
    let cap = fit_soft_cap(area_width);
    lens.iter_mut()
        .map(|l| {
            *l = (*l).min(cap);
            *l
        })
        .sum()
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

/// Columns to draw plus how to size and truncate each one.
struct ColumnPlan {
    cols: Vec<usize>,
    constraints: Vec<Constraint>,
    /// Max display width per drawn column; `None` leaves the text alone.
    trunc: Vec<Option<usize>>,
    rel_selected: Option<usize>,
    start: usize,
}

fn length(w: usize) -> Constraint {
    Constraint::Length(u16::try_from(w).unwrap_or(u16::MAX))
}

/// Legacy size-to-content layout; only the fill column is truncated.
fn plan_content(
    lens: &[usize],
    fill: Option<usize>,
    selected: Option<usize>,
    hint: usize,
    area_w: u16,
) -> ColumnPlan {
    let (cols, rel_selected, start) =
        calculate_visible_columns_for_table(lens, hint, selected, area_w);
    let fill_width = fill.map(|f| {
        let others: usize = cols
            .iter()
            .filter(|&&i| i != f)
            .map(|&i| col_width(lens.get(i).copied().unwrap_or(0)))
            .sum();
        // Table puts 1 cell of spacing between each pair of columns.
        let gaps = cols.len().saturating_sub(1);
        usize::from(area_w).saturating_sub(others + gaps).max(1)
    });
    let (constraints, trunc) = cols
        .iter()
        .map(|&i| {
            if fill == Some(i) {
                (Constraint::Fill(1), fill_width)
            } else {
                (length(lens.get(i).unwrap_or(&0) + 1), None)
            }
        })
        .unzip();
    ColumnPlan {
        cols,
        constraints,
        trunc,
        rel_selected,
        start,
    }
}

/// Fit layout: capped column widths, left/right peek strips for off-screen
/// columns, spare width given to the fill column (or last full column).
#[allow(clippy::too_many_lines)]
fn plan_fit(
    content: &[usize],
    fill: Option<usize>,
    selected: Option<usize>,
    hint: usize,
    area_w: usize,
) -> ColumnPlan {
    let n = content.len();
    let mut lens = content.to_vec();
    if let Some(f) = fill.and_then(|f| lens.get_mut(f)) {
        *f = (*f).min(FILL_MIN_LEN);
    }
    // Soft-cap only when the uncapped content (+ gaps) overflows; otherwise show it all.
    let mut total: usize = lens.iter().sum();
    if total + n.saturating_sub(1) > area_w {
        total = cap_fit_lens(&mut lens, area_w);
    }
    // Pack with gap-aware units (cell + 1 spacing), so budget is `area_w + 1`.
    let units: Vec<usize> = lens.iter().map(|&l| l + 1).collect();
    let budget0 = area_w + 1;
    // Same as summing `col_width(unit)`: each column costs `len + 2`.
    let all_fit = total + 2 * n <= budget0;
    // Too narrow for peeks to be useful; fall back to plain packing.
    let peeks = area_w >= 4 * PEEK_COLS;

    let mut start = if all_fit {
        0
    } else {
        selected.map_or_else(
            || hint.min(n - 1),
            |s| horizontal_window_start(&units, budget0, s, hint),
        )
    };
    let (mut packed, mut left, mut right);
    let mut tries = 0;
    loop {
        left = peeks && start > 0;
        let base = budget0.saturating_sub(if left { PEEK_COLS + 1 } else { 0 });
        packed = visible_columns_packed(&units, start, base);
        right = peeks && packed.last().is_some_and(|&i| i + 1 < n);
        let mut budget = base;
        if right {
            budget = base.saturating_sub(PEEK_COLS + 1);
            packed = visible_columns_packed(&units, start, budget);
        }
        tries += 1;
        let Some(s) = selected.filter(|s| !packed.contains(s)) else {
            break;
        };
        if tries == 3 {
            break;
        }
        start = horizontal_window_start(&units, budget, s, hint);
    }
    debug_assert!(
        selected.is_none_or(|s| packed.contains(&s)),
        "selected column must be in the packed window"
    );

    // `cols` is `[left peek?] + packed + [right peek?]`, so packed index `p`
    // sits at `p + off`.
    let off = usize::from(left);
    let mut cols = Vec::with_capacity(packed.len() + 2);
    let mut widths = Vec::with_capacity(packed.len() + 2);
    if left {
        cols.push(start - 1);
        widths.push(PEEK_COLS);
    }
    for &i in &packed {
        cols.push(i);
        widths.push(col_width(lens.get(i).copied().unwrap_or(0)));
    }
    let used: usize = widths.iter().sum();
    // Under Fit the fill target absorbs spare width (only when nothing is peeked on the right).
    let target_pos = (!right)
        .then(|| {
            fill.and_then(|f| packed.iter().position(|&i| i == f))
                .or_else(|| packed.len().checked_sub(1))
        })
        .flatten()
        .map(|p| p + off);
    if right {
        let w = area_w.saturating_sub(used + cols.len()).max(1);
        cols.push(packed.last().map_or(0, |&i| i + 1));
        widths.push(w);
    }
    if let Some(w) = target_pos.and_then(|p| widths.get_mut(p)) {
        let others = used - *w;
        *w = area_w.saturating_sub(others + cols.len() - 1).max(1);
    }

    let last = cols.len() - 1;
    // Leave the spacing cell free between columns; the last one may use its full width.
    let (constraints, trunc) = widths
        .iter()
        .enumerate()
        .map(|(j, &w)| {
            let c = if Some(j) == target_pos {
                Constraint::Fill(1)
            } else {
                length(w)
            };
            (
                c,
                Some(if j == last {
                    w
                } else {
                    w.saturating_sub(1).max(1)
                }),
            )
        })
        .unzip();
    let rel_selected = selected
        .map(|s| packed.iter().position(|&c| c == s).map_or(off, |p| p + off));
    ColumnPlan {
        cols,
        constraints,
        trunc,
        rel_selected,
        start,
    }
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
        let selected_col_opt = state.view.state.selected_column();
        let plan = match state.view.column_width_mode {
            ColumnWidthMode::Content => {
                let mut lens = state.model.longest_item_lens.clone();
                if let Some(f) = fill.and_then(|f| lens.get_mut(f)) {
                    *f = (*f).min(FILL_MIN_LEN);
                }
                plan_content(
                    &lens,
                    fill,
                    selected_col_opt,
                    state.view.column_offset,
                    area.width,
                )
            }
            ColumnWidthMode::Fit => plan_fit(
                &state.model.longest_item_lens,
                fill,
                selected_col_opt,
                state.view.column_offset,
                usize::from(area.width),
            ),
        };
        let ColumnPlan {
            cols: visible_cols,
            constraints,
            trunc,
            rel_selected: relative_selected_col,
            start: scroll_start,
        } = plan;
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
                        if let Some(w) = trunc.get(vis_idx).copied().flatten() {
                            text = truncate_ellipsis(&text, w);
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
        render_with_mode(items, width, height, ColumnWidthMode::Fit)
    }

    fn render_with_mode<T: TableData + fmt::Debug + Clone>(
        items: Vec<T>,
        width: u16,
        height: u16,
        mode: ColumnWidthMode,
    ) -> Vec<String> {
        let mut state = TableDataState::new(items);
        state.view.column_width_mode = mode;
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
    struct WideThird(&'static str, &'static str, String);

    impl TableData for WideThird {
        fn title() -> &'static str {
            "wide"
        }
        fn ref_array(&self) -> Vec<String> {
            vec![self.0.into(), self.1.into(), self.2.clone()]
        }
        fn num_columns(&self) -> usize {
            3
        }
        fn cols() -> Vec<&'static str> {
            vec!["A", "B", "Wide"]
        }
    }

    #[derive(Debug, Clone)]
    struct Many(String);

    impl TableData for Many {
        fn title() -> &'static str {
            "many"
        }
        fn ref_array(&self) -> Vec<String> {
            (0..6).map(|i| format!("{}{}", self.0, i)).collect()
        }
        fn num_columns(&self) -> usize {
            6
        }
        fn cols() -> Vec<&'static str> {
            vec!["C0", "C1", "C2", "C3", "C4", "C5"]
        }
    }

    #[test]
    fn fit_shows_more_than_one_column_when_third_is_huge() {
        let rows = vec![WideThird("a", "b", "x".repeat(200))];
        let lines = render_with_mode(rows, 40, 3, ColumnWidthMode::Fit);
        let header = lines.first().unwrap().as_str();
        assert!(header.contains('A'), "{header:?}");
        assert!(header.contains('B'), "{header:?}");
    }

    #[test]
    fn fit_fills_short_table_to_right_edge() {
        // Last column grows to the right edge and ellipsizes there.
        let rows = vec![WideThird("a", "b", "x".repeat(200))];
        let lines = render_with_mode(rows, 40, 4, ColumnWidthMode::Fit);
        let row = lines.get(1).unwrap().as_str();
        assert!(row.trim_end().chars().count() >= 38, "{row:?}");
        assert!(row.trim_end().ends_with('…'), "{row:?}");
    }

    #[test]
    fn fit_peek_shows_ellipsis_when_columns_overflow() {
        // 6 cols of width 5 overflow 24 cols → the next column peeks as a 2-col strip.
        let lines = render_with_mode(
            vec![Many("abcd".into())],
            24,
            3,
            ColumnWidthMode::Fit,
        );
        let row = lines.get(1).unwrap().trim_end();
        assert!(row.ends_with('…') && !row.contains("abcd3"), "{row:?}");
    }

    #[test]
    fn fit_does_not_softcap_when_content_fits() {
        // [50, 5] + gap = 56 <= 80: nothing overflows, so nothing is ellipsized.
        #[derive(Debug, Clone)]
        struct Two(String);
        impl TableData for Two {
            fn title() -> &'static str {
                "two"
            }
            fn ref_array(&self) -> Vec<String> {
                vec![self.0.clone(), "short".into()]
            }
            fn num_columns(&self) -> usize {
                2
            }
            fn cols() -> Vec<&'static str> {
                vec!["A", "B"]
            }
        }
        let long = "L".repeat(50);
        let lines = render_with_mode(
            vec![Two(long.clone())],
            80,
            3,
            ColumnWidthMode::Fit,
        );
        let row = lines.get(1).unwrap();
        assert!(row.contains(&long), "{row:?}");
        assert!(!row.contains('…'), "{row:?}");
    }

    #[test]
    fn fit_right_peek_fits_area_and_truncates_every_cell() {
        // 6 cols of width 5 (+gaps) don't fit in 24: right peek strip expected.
        let lines = render_with_mode(
            vec![Many("abcd".into())],
            24,
            3,
            ColumnWidthMode::Fit,
        );
        let row = lines.get(1).unwrap().as_str();
        assert_eq!(row.chars().count(), 24);
        assert!(row.starts_with("abcd0 "), "{row:?}");
        assert!(row.trim_end().ends_with('…'), "{row:?}");
        assert!(!row.contains("abcd5"), "{row:?}");
    }

    #[test]
    fn fit_left_peek_when_scrolled_right() {
        let mut state = TableDataState::new(vec![Many("abcd".into())]);
        state.view.state.select_column(Some(5));
        let mut terminal = Terminal::new(TestBackend::new(24, 3)).unwrap();
        terminal
            .draw(|f| {
                f.render_stateful_widget(
                    DataTable::<Many>::default(),
                    f.area(),
                    &mut state,
                );
            })
            .unwrap();
        let row: String = terminal
            .backend()
            .buffer()
            .content()
            .get(24..48)
            .unwrap()
            .iter()
            .map(buffer::Cell::symbol)
            .collect();
        assert!(row.contains("abcd5"), "{row:?}");
        assert!(row.starts_with('…') || row.starts_with("a…"), "{row:?}");
        assert!(state.view.column_offset > 0);
    }

    #[test]
    fn content_mode_matches_legacy_short_layout() {
        assert_eq!(
            render_with_mode(items(), 20, 4, ColumnWidthMode::Content),
            vec![
                "Name   Value        ",
                "alpha  1            ",
                "beta   2            ",
                "                    ",
            ]
        );
    }

    #[test]
    fn fit_header_floors_width() {
        #[derive(Debug, Clone)]
        struct H;
        impl TableData for H {
            fn title() -> &'static str {
                "h"
            }
            fn ref_array(&self) -> Vec<String> {
                vec!["x".into()]
            }
            fn num_columns(&self) -> usize {
                1
            }
            fn cols() -> Vec<&'static str> {
                vec!["LongHeader"]
            }
        }
        let lines = render_with_mode(vec![H], 40, 3, ColumnWidthMode::Fit);
        assert!(lines.first().unwrap().contains("LongHeader"), "{lines:?}");
    }

    #[test]
    fn fit_tiny_areas_do_not_panic() {
        for w in 0u16..=12 {
            for h in 0u16..=3 {
                let _ = render_with_mode(
                    vec![WideThird("a", "b", "x".repeat(80))],
                    w,
                    h,
                    ColumnWidthMode::Fit,
                );
                let _ = render_with_mode(
                    vec![Many("abcd".into())],
                    w,
                    h,
                    ColumnWidthMode::Fit,
                );
            }
        }
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
            render_with_mode(items(), 20, 4, ColumnWidthMode::Content),
            vec![
                "Name   Value        ",
                "alpha  1            ",
                "beta   2            ",
                "                    ",
            ]
        );
        assert_eq!(
            render_with_mode(items(), 8, 3, ColumnWidthMode::Content),
            vec!["Name    ", "alpha   ", "beta    "]
        );
        assert_eq!(
            render_with_mode(items(), 3, 3, ColumnWidthMode::Content),
            vec!["Nam", "alp", "bet"]
        );
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
        assert_eq!(ColumnWidthMode::Fit.to_string(), "Fit");
        assert_eq!(ColumnWidthMode::Content.to_string(), "Content");
    }

    #[test]
    fn cap_fit_lens_soft_caps_and_returns_total() {
        // area 30 → soft = 10
        let mut lens = vec![5usize, 100, 8];
        assert_eq!(cap_fit_lens(&mut lens, 30), 23);
        assert_eq!(lens, vec![5, 10, 8]);
    }

    #[test]
    fn cap_fit_lens_never_exceeds_viewport() {
        for area in [0usize, 1, 2, 20] {
            let mut lens = vec![500usize];
            cap_fit_lens(&mut lens, area);
            assert!(lens.iter().all(|&l| l <= area.max(1)), "area {area}");
        }
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
