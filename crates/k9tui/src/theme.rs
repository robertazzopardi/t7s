use ratatui::style::{Color, Modifier, Style};

#[must_use]
pub fn border() -> Style {
    Style::default().fg(Color::DarkGray)
}

#[must_use]
pub fn title() -> Style {
    Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

#[must_use]
pub fn muted() -> Style {
    border()
}

// No hue here by design — color is reserved for state (env tags, errors,
// the cursor row), not decoration. Weight carries emphasis instead.
#[must_use]
pub fn accent() -> Style {
    title()
}

#[must_use]
pub fn info_label() -> Style {
    border()
}

#[must_use]
pub fn info_value() -> Style {
    Style::default().fg(Color::White)
}

#[must_use]
pub fn hotkey_key() -> Style {
    info_value()
}

#[must_use]
pub fn focus_field() -> Style {
    Style::default().fg(Color::Yellow).bg(Color::DarkGray)
}

#[must_use]
pub fn focus_cursor() -> Style {
    Style::default().bg(Color::Yellow).fg(Color::Black)
}

#[must_use]
pub fn selection_row() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

#[must_use]
pub fn selection_col() -> Style {
    // No UNDERLINED here — ratatui applies column_highlight_style to every
    // row in the column, so underline would run down the whole column.
    Style::default().fg(Color::Cyan)
}

#[must_use]
pub fn selection_cell() -> Style {
    Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(Color::White)
        .bg(Color::DarkGray)
}

#[must_use]
pub fn draft_row() -> Style {
    Style::default().fg(Color::Green)
}

#[must_use]
pub const fn multi_select_bg() -> Color {
    Color::Blue
}

#[must_use]
pub fn status_idle() -> Style {
    border()
}

#[must_use]
pub fn status_message() -> Style {
    selection_col()
}

#[must_use]
pub fn error() -> Style {
    Style::default().fg(Color::Red)
}

#[must_use]
pub fn success() -> Style {
    draft_row()
}

#[must_use]
pub fn modal_default_border() -> Style {
    selection_col()
}

#[must_use]
pub fn modal_danger_border() -> Style {
    error()
}

#[must_use]
pub fn modal_confirm_border() -> Style {
    Style::default().fg(Color::Yellow)
}

#[must_use]
pub fn modal_connection_border() -> Style {
    Style::default().fg(Color::Blue)
}

#[must_use]
pub fn header_row() -> Style {
    // Plain, unbold — no background bar, no weight.
    info_value()
}

#[must_use]
pub fn null_cell() -> Style {
    border()
}
