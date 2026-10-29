use ratatui::{
    prelude::{Buffer, Constraint, Layout, Rect, Widget},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::{
    theme,
    widgets::{hotkey::Hotkey, hotkey_view::HotkeyView},
};

/// Flex weights for the info / recent / primary hotkey segments; the global
/// hotkey segment is sized to its content so long labels are not clipped.
const MAIN_COLUMN_FILLS: [Constraint; 3] = [
    Constraint::Fill(24),
    Constraint::Fill(20),
    Constraint::Fill(36),
];
/// Same, when there are no recent hotkeys: their share goes to the primary
/// hotkeys so a second column (more than one bar-height of hints) is not clipped.
const NO_RECENT_COLUMN_FILLS: [Constraint; 3] = [
    Constraint::Fill(24),
    Constraint::Length(0),
    Constraint::Fill(56),
];
// Second row is a blank spacer before the box below — no rule drawn into it.
const ROW_CONSTRAINTS: [Constraint; 2] =
    [Constraint::Fill(1), Constraint::Length(1)];
const MIN_APP_LABEL_WIDTH: u16 = 8;
const APP_LABEL_RIGHT_MARGIN: u16 = 1;

/// Rows the app reserves for [`TopBarView`].
pub const TOPBAR_HEIGHT: u16 = 7;

pub struct TopBarView<'a> {
    pub summary: &'a str,
    pub recent_hotkeys: &'a [Hotkey],
    pub hotkeys: &'a [Hotkey],
    pub global_hotkeys: &'a [Hotkey],
    pub app_name: &'a str,
    pub build_info: Option<String>,
}

impl Widget for TopBarView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let row = Layout::vertical(ROW_CONSTRAINTS)
            .spacing(0)
            .split(area)
            .first()
            .copied()
            .unwrap_or(area);

        let app_name_lines = self.app_name.trim_matches('\n').lines();
        let app_name_width =
            app_name_lines.clone().map(str::len).max().unwrap_or(0);
        let app_label_width = u16::try_from(app_name_width.max(1))
            .unwrap_or(u16::MAX)
            .max(MIN_APP_LABEL_WIDTH);

        let [main_area, app_logo_cell] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(
                app_label_width.saturating_add(APP_LABEL_RIGHT_MARGIN),
            ),
        ])
        .spacing(1)
        .areas(row);

        // Global hotkeys get exactly the width they need (capped at a third
        // of the bar so the other segments are never starved).
        let global_width = HotkeyView::new(self.global_hotkeys)
            .required_width(main_area.height)
            .min(main_area.width / 3);
        let [left_area, global_cell] = Layout::horizontal([
            Constraint::Fill(1),
            Constraint::Length(global_width),
        ])
        .spacing(1)
        .areas(main_area);
        let [app_info_cell, recent_cell, hotkey_cell] =
            Layout::horizontal(if self.recent_hotkeys.is_empty() {
                NO_RECENT_COLUMN_FILLS
            } else {
                MAIN_COLUMN_FILLS
            })
            .spacing(1)
            .areas(left_area);

        if let Some(build_info) = &self.build_info {
            render_info_stack(build_info, app_info_cell, buf);
        } else {
            render_info_stack(self.summary, app_info_cell, buf);
        }

        HotkeyView::new(self.recent_hotkeys).render(recent_cell, buf);
        HotkeyView::new(self.hotkeys).render(hotkey_cell, buf);
        HotkeyView::new(self.global_hotkeys).render(global_cell, buf);

        let label_align_width =
            (app_logo_cell.width.saturating_sub(APP_LABEL_RIGHT_MARGIN))
                as usize;
        let padding = label_align_width.saturating_sub(app_name_width);
        let padded = app_name_lines
            .map(|line| {
                format!("{:>width$}", line, width = line.len() + padding)
            })
            .collect::<Vec<_>>()
            .join("\n");
        Paragraph::new(padded)
            .style(theme::border())
            .render(app_logo_cell, buf);
    }
}

/// Shorten a value by keeping its tail, prefixed with `…`, when it exceeds `max_val` chars.
fn shorten_value(val: &str, max_val: usize) -> String {
    if max_val > 1 && val.chars().count() > max_val {
        let keep = max_val.saturating_sub(1);
        format!(
            "…{}",
            val.chars()
                .skip(val.chars().count().saturating_sub(keep))
                .collect::<String>()
        )
    } else {
        val.to_string()
    }
}

fn render_info_stack(text: &str, area: Rect, buf: &mut Buffer) {
    let max_val = area.width.saturating_sub(12) as usize;
    let lines: Vec<Line> = text
        .lines()
        .map(|line| {
            if let Some((label, value)) = line.split_once(':') {
                let val = shorten_value(value.trim(), max_val);
                Line::from(vec![
                    Span::styled(
                        format!("{}:", label.trim()),
                        theme::info_label(),
                    ),
                    Span::raw(" "),
                    Span::styled(val, theme::info_value()),
                ])
            } else {
                Line::from(Span::styled(line.to_string(), theme::muted()))
            }
        })
        .collect();
    Paragraph::new(lines).render(area, buf);
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    fn render_text(width: u16, global: &[Hotkey]) -> String {
        let hotkeys = [
            Hotkey::new('d', "describe"),
            Hotkey::new('l', "logs"),
            Hotkey::new('s', "start"),
            Hotkey::new('x', "stop"),
            Hotkey::new('r', "restart"),
            Hotkey::new('D', "delete"),
        ];
        let mut terminal =
            Terminal::new(TestBackend::new(width, 8)).expect("terminal");
        terminal
            .draw(|f| {
                f.render_widget(
                    TopBarView {
                        summary: "Containers: 3",
                        recent_hotkeys: &[],
                        hotkeys: &hotkeys,
                        global_hotkeys: global,
                        app_name: "  c8s\n  ___\n",
                        build_info: None,
                    },
                    f.area(),
                );
            })
            .expect("draw");
        let buf = terminal.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn long_global_hint_not_truncated_at_120_cols() {
        let global = [
            Hotkey::new('1', "containers"),
            Hotkey::new('2', "images"),
            Hotkey::new('3', "volumes"),
            Hotkey::new('4', "networks"),
            Hotkey::new('/', "search"),
            Hotkey::new('q', "clear filter"),
        ];
        let text = render_text(120, &global);
        assert!(text.contains("clear filter"), "{text}");
    }
}
