use crossterm::event::KeyCode;
use k9tui::{
    theme,
    widgets::{hotkey::Hotkey, table::DataTable, top_bar::TopBarView},
};
use ratatui::{
    Frame,
    prelude::*,
    style::Color,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{
    app::{APP_NAME, App},
    app_state::AppState,
    command::suggest,
    proc::{SystemInfo, signals},
    ui::widgets::hotkeys::{GLOBAL_HOTKEYS, describe_hotkeys, signal_hotkeys},
};

const TOPBAR_HEIGHT: u16 = 7;
const FOOTER_HEIGHT: u16 = 1;
const SEARCH_BAR_HEIGHT: u16 = 3;

impl App {
    pub fn render(&mut self, frame: &mut Frame) {
        match &self.state {
            AppState::List => self.render_list(frame),
            AppState::Info(info) => {
                let info = info.clone();
                Self::render_info(frame, &info);
            }
            AppState::Describe { name, text, .. } => {
                let name = name.clone();
                let text = text.clone();
                self.render_describe(frame, &name, &text);
            }
            AppState::Signal {
                pid, name, index, ..
            } => {
                let (pid, name, index) = (*pid, name.clone(), *index);
                self.render_signal(frame, pid, &name, index);
            }
        }
    }

    #[allow(clippy::too_many_lines)] // flat list-view render
    fn render_list(&mut self, frame: &mut Frame) {
        let layout = Layout::vertical([
            Constraint::Length(TOPBAR_HEIGHT),
            Constraint::Min(0),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(frame.area());

        let top_area = layout.first().copied().unwrap_or_default();
        let content_area = layout.get(1).copied().unwrap_or_default();
        let footer_area = layout.get(2).copied().unwrap_or_default();

        let mut global: Vec<Hotkey> = GLOBAL_HOTKEYS.to_vec();
        if !self.filter.is_empty() {
            // `q` clears the active filter instead of quitting while one is
            // set; keep the hint truthful.
            if let Some(q) =
                global.iter_mut().find(|h| h.keycode == KeyCode::Char('q'))
            {
                *q = Hotkey::new('q', "clear filter");
            }
        }

        let n = self.processes.model.items.len();
        let summary = format!("Processes: {n}");
        frame.render_widget(
            TopBarView {
                summary: &summary,
                recent_hotkeys: &[],
                hotkeys: &self.hotkeys,
                global_hotkeys: &global,
                app_name: APP_NAME,
                build_info: Some(self.build_info.clone()),
            },
            top_area,
        );

        let content_area = if self.search_open || !self.filter.is_empty() {
            let search_layout = Layout::vertical([
                Constraint::Length(SEARCH_BAR_HEIGHT),
                Constraint::Min(0),
            ])
            .split(content_area);
            let search_area =
                search_layout.first().copied().unwrap_or_default();
            let search_block = Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border())
                .title(if self.command_mode {
                    " Command "
                } else {
                    " Search "
                })
                .title_style(theme::title());
            let inner = search_block.inner(search_area);
            frame.render_widget(search_block, search_area);
            if self.command_mode {
                let ghost = suggest(&self.command_text)
                    .map(|s| {
                        s.chars()
                            .skip(self.command_text.chars().count())
                            .collect::<String>()
                    })
                    .unwrap_or_default();
                Paragraph::new(Line::from(vec![
                    Span::raw(format!(":{}", self.command_text)),
                    Span::styled(ghost, theme::muted()),
                ]))
                .render(inner, frame.buffer_mut());
            } else {
                let cursor = if self.search_open { "_" } else { "" };
                Paragraph::new(format!("/{}{cursor}", self.filter))
                    .render(inner, frame.buffer_mut());
            }
            search_layout.get(1).copied().unwrap_or_default()
        } else {
            content_area
        };

        let arrow = if self.sort_desc { "↓" } else { "↑" };
        let title = if self.filter.is_empty() {
            format!(
                " Processes [{n}]  sort: {} {arrow} ",
                self.sort_key.label()
            )
        } else {
            format!(
                " Processes [{n} matches]  sort: {} {arrow} ",
                self.sort_key.label()
            )
        };
        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .title(title)
            .title_alignment(Alignment::Center);
        let inner = block.inner(content_area);
        frame.render_widget(block, content_area);

        if n == 0 {
            Paragraph::new(if self.all.is_empty() {
                "No processes".to_string()
            } else {
                "No matching processes".to_string()
            })
            .style(theme::muted())
            .alignment(Alignment::Center)
            .render(inner, frame.buffer_mut());
        } else {
            frame.render_stateful_widget(
                DataTable::default(),
                inner,
                &mut self.processes,
            );
        }

        frame.render_widget(self.status_line.clone(), footer_area);
    }

    fn render_info(frame: &mut Frame, info: &SystemInfo) {
        let area = frame.area();
        let mut text = info.lines().join("\n");
        text.push_str("\n\nPress i, q, or Esc to close.");
        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .title(" System ")
            .title_alignment(Alignment::Center);
        let modal_area = centered_box(area, 44, 12);
        let inner = block.inner(modal_area);
        frame.render_widget(block, modal_area);
        Paragraph::new(text)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true })
            .render(inner, frame.buffer_mut());
    }

    fn render_describe(&mut self, frame: &mut Frame, name: &str, text: &str) {
        let layout = Layout::vertical([
            Constraint::Length(TOPBAR_HEIGHT),
            Constraint::Min(0),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(frame.area());

        let top_area = layout.first().copied().unwrap_or_default();
        let content_area = layout.get(1).copied().unwrap_or_default();
        let footer_area = layout.get(2).copied().unwrap_or_default();

        let hotkeys = describe_hotkeys();
        frame.render_widget(
            TopBarView {
                summary: &format!("Describe: {name}"),
                recent_hotkeys: &[],
                hotkeys: &hotkeys,
                global_hotkeys: &[],
                app_name: APP_NAME,
                build_info: None,
            },
            top_area,
        );

        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .title(format!(" Describe: {name} "))
            .title_alignment(Alignment::Center);
        let inner = block.inner(content_area);
        frame.render_widget(block, content_area);

        let all_lines: Vec<&str> = text.lines().collect();
        let visible = inner.height as usize;
        self.describe_viewport_height = visible;
        let max_start = all_lines.len().saturating_sub(visible);
        let start = self.describe_scroll.min(max_start);
        self.describe_scroll = start;
        let end = (start + visible).min(all_lines.len());
        let shown = all_lines.get(start..end).unwrap_or(&[]).join("\n");
        Paragraph::new(shown)
            .wrap(Wrap { trim: false })
            .render(inner, frame.buffer_mut());

        frame.render_widget(self.status_line.clone(), footer_area);
    }

    fn render_signal(
        &self,
        frame: &mut Frame,
        pid: i32,
        name: &str,
        index: usize,
    ) {
        let layout = Layout::vertical([
            Constraint::Length(TOPBAR_HEIGHT),
            Constraint::Min(0),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(frame.area());

        let top_area = layout.first().copied().unwrap_or_default();
        let content_area = layout.get(1).copied().unwrap_or_default();
        let footer_area = layout.get(2).copied().unwrap_or_default();

        let hotkeys = signal_hotkeys();
        frame.render_widget(
            TopBarView {
                summary: &format!("Signal: {name} (PID {pid})"),
                recent_hotkeys: &[],
                hotkeys: &hotkeys,
                global_hotkeys: &[],
                app_name: APP_NAME,
                build_info: None,
            },
            top_area,
        );

        let menu = signals::menu();
        let list_height = u16::try_from(menu.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2);
        let box_area = centered_box(
            content_area,
            40,
            list_height.clamp(3, content_area.height.max(3)),
        );
        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(theme::modal_confirm_border())
            .title(format!(" Signal: {name} (PID {pid}) "))
            .title_alignment(Alignment::Center);
        let inner = block.inner(box_area);
        frame.render_widget(Clear, box_area);
        frame.render_widget(block, box_area);

        let lines: Vec<Line> = menu
            .iter()
            .enumerate()
            .map(|(i, sig)| {
                let label = signals::label(*sig);
                if i == index {
                    Line::from(Span::styled(
                        format!("> {label}"),
                        theme::accent(),
                    ))
                } else {
                    Line::from(Span::styled(
                        format!("  {label}"),
                        Style::default().fg(Color::White),
                    ))
                }
            })
            .collect();
        Paragraph::new(lines).render(inner, frame.buffer_mut());

        frame.render_widget(self.status_line.clone(), footer_area);

        if let Some(dialog) = self.confirm_dialog.clone() {
            frame.render_widget(dialog, frame.area());
        }
    }
}

/// A fixed-size box centered within `area`, clamped to fit.
fn centered_box(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let vertical = Layout::vertical([Constraint::Length(height)])
        .flex(layout::Flex::Center)
        .split(area);
    let horizontal = Layout::horizontal([Constraint::Length(width)])
        .flex(layout::Flex::Center)
        .split(vertical.first().copied().unwrap_or(area));
    horizontal.first().copied().unwrap_or(area)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use k9tui::widgets::modal::ConfirmDialog;
    use ratatui::{
        Terminal,
        backend::TestBackend,
        buffer::{Buffer, Cell},
    };

    use super::*;
    use crate::proc::monitor::{DemoSource, demo_snapshot};

    fn app() -> App {
        let mut app = App::new(Arc::new(DemoSource::new()));
        app.apply_snapshot(demo_snapshot());
        app
    }

    fn text(buf: &Buffer) -> String {
        let width = usize::from(buf.area.width);
        buf.content()
            .chunks(width)
            .map(|row| row.iter().map(Cell::symbol).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render(app: &mut App, w: u16, h: u16) -> Buffer {
        let mut terminal =
            Terminal::new(TestBackend::new(w, h)).expect("test terminal");
        terminal.draw(|f| app.render(f)).expect("draw");
        terminal.backend().buffer().clone()
    }

    #[test]
    fn list_shows_columns_and_sort_indicator() {
        let mut app = app();
        let out = text(&render(&mut app, 120, 30));
        for want in
            ["PID", "USER", "CPU%", "COMMAND", "sort: cpu ↓", "Processes"]
        {
            assert!(out.contains(want), "missing {want:?} in\n{out}");
        }
    }

    #[test]
    fn every_state_renders_at_small_and_normal_sizes() {
        let describe = AppState::Describe {
            pid: 1,
            name: "init".to_string(),
            text: "line\n".repeat(60),
        };
        let signal = AppState::Signal {
            pid: 1,
            name: "init".to_string(),
            start_time: 0,
            index: 0,
        };
        for (w, h) in [(1, 1), (3, 3), (10, 4), (20, 5), (40, 12), (120, 30)] {
            for state in [
                AppState::List,
                AppState::Info(demo_snapshot().info),
                describe.clone(),
                signal.clone(),
            ] {
                let mut app = app();
                app.state = state;
                let _ = render(&mut app, w, h);
            }
        }
    }

    #[test]
    fn signal_picker_with_confirm_renders_overlay() {
        let mut app = app();
        app.state = AppState::Signal {
            pid: 1024,
            name: "nginx".to_string(),
            start_time: 0,
            index: 0,
        };
        app.confirm_dialog = Some(ConfirmDialog::new(
            " Send signal ",
            "Send SIGTERM to 'nginx' (PID 1024)?",
            theme::modal_danger_border(),
            1,
        ));
        let out = text(&render(&mut app, 120, 30));
        assert!(out.contains("Signal: nginx (PID 1024)"));
        assert!(out.contains("SIGTERM"));
        assert!(out.contains("Yes") && out.contains("No"));
    }
}
