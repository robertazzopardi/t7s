use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    prelude::{Alignment, Buffer, Constraint, Direction, Layout, Rect, Widget},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};
use ratatui_textarea::{CursorMove, TextArea};

use crate::{theme, widgets::buttons::Buttons};

#[derive(Debug, Clone)]
pub struct ModalField {
    pub label: &'static str,
    pub input: TextArea<'static>,
    pub is_focused: bool,
    /// When set, this field is a dropdown; value must be one of these options.
    pub options: Option<Vec<&'static str>>,
}

impl ModalField {
    fn make_input(text: &str) -> TextArea<'static> {
        let mut input = TextArea::new(vec![text.to_string()]);
        // Disable cursor line highlight (not needed for single-line form fields)
        input.set_cursor_line_style(Style::default());
        // Hide cursor until focused
        input.set_cursor_style(Style::default());
        // No undo/redo needed for form fields
        input.set_max_histories(0);
        input
    }

    #[must_use]
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            input: Self::make_input(""),
            is_focused: false,
            options: None,
        }
    }

    /// Get the current text value of this field.
    #[must_use]
    pub fn value(&self) -> &str {
        self.input.lines().first().map_or("", |s| s.as_str())
    }

    /// Set the text value of this field, replacing all content.
    pub fn set_value<S: Into<String>>(&mut self, text: S) {
        let text = text.into();
        self.input = Self::make_input(&text);
        // Move cursor to end of the pre-filled text
        self.input.move_cursor(CursorMove::End);
        // Restore cursor style based on current focus state
        if self.is_focused {
            self.input.set_cursor_style(theme::focus_cursor());
        }
    }

    pub fn set_focus(&mut self, focused: bool) {
        self.is_focused = focused;
        if focused {
            self.input.set_style(theme::focus_field());
            self.input.set_cursor_style(theme::focus_cursor());
        } else {
            self.input.set_style(Style::default().fg(Color::White));
            self.input.set_cursor_style(Style::default());
        }
    }

    pub fn input_key(&mut self, key: KeyEvent) {
        if self.options.is_none() {
            self.input.input(key);
        }
    }

    /// Enable character masking (for password fields).
    pub fn set_masked(&mut self) {
        self.input.set_mask_char('•');
    }

    /// Set dropdown options. If value is empty, sets value to first option.
    #[allow(clippy::indexing_slicing)]
    pub fn set_options(&mut self, options: Vec<&'static str>) {
        if !options.is_empty() {
            let first = options[0].to_string();
            self.options = Some(options);
            if self.value().is_empty() {
                self.set_value(first);
            }
        }
    }

    /// Ensure value is one of the options. Sets to first option if invalid.
    #[allow(clippy::indexing_slicing)]
    pub fn clamp_to_options(&mut self) {
        if let Some(opts) = &self.options
            && !opts.is_empty()
            && !opts.iter().any(|o| *o == self.value())
        {
            self.set_value(opts[0]);
        }
    }

    #[must_use]
    pub const fn is_dropdown(&self) -> bool {
        self.options.is_some()
    }
}

/// Outcome of a key event handled by a dialog widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogAction {
    None,
    Submit,
    Cancel,
}

const CONFIRM_DIALOG_WIDTH: u16 = 50;
const CONFIRM_DIALOG_HEIGHT: u16 = 8;

/// A yes/no confirmation dialog with a title, message, and configurable border style.
#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    pub is_open: bool,
    pub message: String,
    title: String,
    border_style: Style,
    selected_button: usize,
}

impl ConfirmDialog {
    #[must_use]
    pub fn new<T: Into<String>, M: Into<String>>(
        title: T,
        message: M,
        border_style: Style,
        default_button: usize,
    ) -> Self {
        Self {
            is_open: true,
            message: message.into(),
            title: title.into(),
            border_style,
            selected_button: default_button,
        }
    }

    pub const fn close(&mut self) {
        self.is_open = false;
    }

    const fn next_button(&mut self) {
        self.selected_button = (self.selected_button + 1) % 2;
    }

    const fn prev_button(&mut self) {
        self.selected_button = (self.selected_button + 1) % 2;
    }

    #[must_use]
    pub const fn is_confirmed(&self) -> bool {
        self.selected_button == 0
    }

    pub const fn handle_key_events(&mut self, key: KeyEvent) -> DialogAction {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => {
                self.close();
                DialogAction::Cancel
            }
            (_, KeyCode::Enter) => {
                let action = if self.is_confirmed() {
                    DialogAction::Submit
                } else {
                    DialogAction::Cancel
                };
                self.close();
                action
            }
            (_, KeyCode::Left) => {
                self.prev_button();
                DialogAction::None
            }
            (_, KeyCode::Right) => {
                self.next_button();
                DialogAction::None
            }
            _ => DialogAction::None,
        }
    }
}

impl Widget for ConfirmDialog {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.is_open {
            return;
        }

        let x = area.x + (area.width.saturating_sub(CONFIRM_DIALOG_WIDTH)) / 2;
        let y =
            area.y + (area.height.saturating_sub(CONFIRM_DIALOG_HEIGHT)) / 2;
        let modal_area =
            Rect::new(x, y, CONFIRM_DIALOG_WIDTH, CONFIRM_DIALOG_HEIGHT);

        let block = Block::default()
            .title(self.title.clone())
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(self.border_style)
            .style(Style::default().bg(Color::Black));
        Clear.render(modal_area, buf);
        block.render(modal_area, buf);

        let inner_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(1)])
            .margin(1)
            .split(modal_area);

        let content_layout = *inner_layout.first().unwrap_or(&Rect::ZERO);
        Paragraph::new(self.message)
            .style(Style::default().fg(Color::White))
            .alignment(Alignment::Center)
            .render(content_layout, buf);

        let buttons = Buttons {
            buttons: vec!["Yes", "No"],
            selected: self.selected_button,
        };
        let button_layout = *inner_layout.get(1).unwrap_or(&Rect::ZERO);
        buttons.render(button_layout, buf);
    }
}

/// What counts as a submittable value in a [`TextPromptModal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptValidation {
    NonEmpty,
    PositiveInteger,
}

/// A single-line text-input dialog: optional prompt line, input, OK/Cancel buttons.
#[derive(Debug, Clone)]
pub struct TextPromptModal {
    pub is_open: bool,
    pub submitted: bool,
    input: TextArea<'static>,
    prompt: Option<String>,
    title: String,
    width: u16,
    height: u16,
    validation: PromptValidation,
    buttons: [&'static str; 2],
    selected_button: usize,
    masked: bool,
    placeholder: Option<&'static str>,
}

impl TextPromptModal {
    fn make_input() -> TextArea<'static> {
        let mut input = TextArea::default();
        input.set_cursor_line_style(Style::default());
        input.set_cursor_style(theme::focus_cursor());
        input.set_max_histories(0);
        input
    }

    #[must_use]
    pub fn new<T: Into<String>>(title: T, width: u16, height: u16) -> Self {
        Self {
            is_open: true,
            submitted: false,
            input: Self::make_input(),
            prompt: None,
            title: title.into(),
            width,
            height,
            validation: PromptValidation::NonEmpty,
            buttons: ["OK", "Cancel"],
            selected_button: 0,
            masked: false,
            placeholder: None,
        }
    }

    #[must_use]
    pub fn with_prompt<S: Into<String>>(mut self, prompt: S) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    #[must_use]
    pub fn masked(mut self) -> Self {
        self.masked = true;
        self.input.set_mask_char('•');
        self
    }

    #[must_use]
    pub fn with_placeholder(mut self, placeholder: &'static str) -> Self {
        self.placeholder = Some(placeholder);
        self.input.set_placeholder_text(placeholder);
        self
    }

    #[must_use]
    pub const fn with_validation(
        mut self,
        validation: PromptValidation,
    ) -> Self {
        self.validation = validation;
        self
    }

    #[must_use]
    pub const fn with_buttons(
        mut self,
        ok: &'static str,
        cancel: &'static str,
    ) -> Self {
        self.buttons = [ok, cancel];
        self
    }

    /// Get the current text value of the input.
    #[must_use]
    pub fn input_value(&self) -> String {
        self.input.lines().first().cloned().unwrap_or_default()
    }

    /// Parse the current input as a positive integer, if valid.
    #[must_use]
    pub fn parsed_positive_int(&self) -> Option<u64> {
        self.input_value()
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|&n| n > 0)
    }

    pub const fn close(&mut self) {
        self.is_open = false;
    }

    /// Clear the input field back to empty.
    pub fn clear_input(&mut self) {
        self.input = Self::make_input();
        if self.masked {
            self.input.set_mask_char('•');
        }
        if let Some(placeholder) = self.placeholder {
            self.input.set_placeholder_text(placeholder);
        }
    }

    fn can_submit(&self) -> bool {
        match self.validation {
            PromptValidation::NonEmpty => !self.input_value().is_empty(),
            PromptValidation::PositiveInteger => {
                self.parsed_positive_int().is_some()
            }
        }
    }

    pub fn handle_key_events(&mut self, key: KeyEvent) -> DialogAction {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => {
                self.submitted = false;
                self.close();
                DialogAction::Cancel
            }
            (_, KeyCode::Tab | KeyCode::Down) => {
                if self.selected_button == 0 {
                    self.selected_button = 1;
                }
                DialogAction::None
            }
            (_, KeyCode::BackTab | KeyCode::Up) => {
                if self.selected_button == 1 {
                    self.selected_button = 0;
                }
                DialogAction::None
            }
            (_, KeyCode::Left) => {
                if self.selected_button == 1 {
                    self.selected_button = 0;
                } else {
                    self.input.input(key);
                }
                DialogAction::None
            }
            (_, KeyCode::Right) => {
                if self.selected_button == 0 {
                    let line = self.input_value();
                    let col = self.input.cursor().1;
                    if col >= line.len() {
                        self.selected_button = 1;
                    } else {
                        self.input.input(key);
                    }
                }
                DialogAction::None
            }
            (_, KeyCode::Enter) => match self.selected_button {
                0 if self.can_submit() => {
                    self.submitted = true;
                    self.close();
                    DialogAction::Submit
                }
                1 => {
                    self.submitted = false;
                    self.close();
                    DialogAction::Cancel
                }
                _ => DialogAction::None,
            },
            _ if self.selected_button == 0 => {
                self.input.input(key);
                DialogAction::None
            }
            _ => DialogAction::None,
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        if self.selected_button == 0 {
            self.input.insert_str(text);
        }
    }
}

impl Widget for TextPromptModal {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.is_open {
            return;
        }

        let x = area.x + (area.width.saturating_sub(self.width)) / 2;
        let y = area.y + (area.height.saturating_sub(self.height)) / 2;
        let modal_area = Rect::new(x, y, self.width, self.height);

        let block = Block::default()
            .title(self.title.clone())
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(theme::modal_confirm_border())
            .style(Style::default().bg(Color::Black));
        Clear.render(modal_area, buf);
        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        if let Some(prompt) = &self.prompt {
            let [prompt_area, input_area, button_area] = Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .areas(inner);

            Paragraph::new(prompt.clone())
                .style(Style::default().fg(Color::White))
                .alignment(Alignment::Left)
                .render(prompt_area, buf);
            Widget::render(&self.input, input_area, buf);
            Buttons {
                buttons: vec![self.buttons[0], self.buttons[1]],
                selected: self.selected_button,
            }
            .render(button_area, buf);
        } else {
            let [input_area, button_area] = Layout::vertical([
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .areas(inner);

            Widget::render(&self.input, input_area, buf);
            Buttons {
                buttons: vec![self.buttons[0], self.buttons[1]],
                selected: self.selected_button,
            }
            .render(button_area, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

    use super::*;

    #[test]
    fn modal_field_set_value_updates_text_and_cursor() {
        let mut field = ModalField::new("Name");
        field.set_value("hello");
        assert_eq!(field.value(), "hello");
    }

    #[test]
    fn modal_field_clamp_to_options_resets_invalid_value() {
        let mut field = ModalField::new("Env");
        field.set_options(vec!["dev", "prod"]);
        field.set_value("garbage");
        field.clamp_to_options();
        assert_eq!(field.value(), "dev");
    }

    #[test]
    fn confirm_dialog_enter_submits_when_yes_selected() {
        let mut dialog =
            ConfirmDialog::new("Delete?", "sure?", Style::default(), 0);
        let action = dialog.handle_key_events(KeyEvent::from(KeyCode::Enter));
        assert_eq!(action, DialogAction::Submit);
        assert!(!dialog.is_open);
    }

    #[test]
    fn confirm_dialog_esc_always_cancels() {
        let mut dialog =
            ConfirmDialog::new("Delete?", "sure?", Style::default(), 0);
        let action = dialog.handle_key_events(KeyEvent::from(KeyCode::Esc));
        assert_eq!(action, DialogAction::Cancel);
    }

    #[test]
    fn confirm_dialog_left_right_toggles_selection() {
        let mut dialog =
            ConfirmDialog::new("Delete?", "sure?", Style::default(), 0);
        dialog.handle_key_events(KeyEvent::from(KeyCode::Right));
        assert!(!dialog.is_confirmed());
        dialog.handle_key_events(KeyEvent::from(KeyCode::Left));
        assert!(dialog.is_confirmed());
    }

    #[test]
    fn confirm_dialog_renders_title_and_message() {
        let dialog = ConfirmDialog::new(
            "Delete?",
            "Remove this row?",
            Style::default(),
            0,
        );
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| frame.render_widget(dialog, frame.area()))
            .unwrap();

        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(Cell::symbol)
            .collect();

        assert!(content.contains("Delete?"));
        assert!(content.contains("Remove this row?"));
        assert!(content.contains("Yes"));
        assert!(content.contains("No"));
    }

    #[test]
    fn text_prompt_modal_requires_non_empty_by_default() {
        let mut modal = TextPromptModal::new("Rename", 30, 5);
        let action = modal.handle_key_events(KeyEvent::from(KeyCode::Enter));
        assert_eq!(action, DialogAction::None);
        assert!(modal.is_open);

        modal.handle_key_events(KeyEvent::from(KeyCode::Char('x')));
        let action = modal.handle_key_events(KeyEvent::from(KeyCode::Enter));
        assert_eq!(action, DialogAction::Submit);
        assert_eq!(modal.input_value(), "x");
    }

    #[test]
    fn text_prompt_modal_positive_integer_validation_rejects_zero() {
        let mut modal = TextPromptModal::new("Count", 30, 5)
            .with_validation(PromptValidation::PositiveInteger);
        modal.handle_key_events(KeyEvent::from(KeyCode::Char('0')));
        let action = modal.handle_key_events(KeyEvent::from(KeyCode::Enter));
        assert_eq!(action, DialogAction::None);
        assert_eq!(modal.parsed_positive_int(), None);
    }
}
