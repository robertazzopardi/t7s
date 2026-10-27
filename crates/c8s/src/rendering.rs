use crossterm::event::KeyCode;
use k9tui::{
    theme,
    widgets::{
        hotkey::Hotkey, table::DataTable, text_search::filter_and_highlight,
        top_bar::TopBarView,
    },
};
use ratatui::{
    Frame,
    prelude::*,
    style::Color,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::{
    app::{APP_NAME, App},
    app_state::{AppState, ResourceKind},
    command::suggest,
    docker::client::DaemonHealth,
    log_dedup::fold,
    ui::widgets::hotkeys::{
        GLOBAL_HOTKEYS, VIEW_SWITCH_HOTKEYS, describe_hotkeys, log_hotkeys,
        process_hotkeys,
    },
};

const TOPBAR_HEIGHT: u16 = 7;
const FOOTER_HEIGHT: u16 = 1;
const SEARCH_BAR_HEIGHT: u16 = 3;

impl App {
    pub fn render(&mut self, frame: &mut Frame) {
        self.sync_column_width_mode();
        match &self.state {
            AppState::Connecting => Self::render_connecting(frame),
            AppState::ConnectError(message) => {
                Self::render_connect_error(frame, message);
            }
            AppState::List => self.render_list(frame),
            AppState::Logs { name, .. } => {
                let name = name.clone();
                self.render_logs(frame, &name);
            }
            AppState::Info(health) => {
                let health = health.clone();
                Self::render_daemon_health(frame, &health);
            }
            AppState::Describe { name, text } => {
                let name = name.clone();
                let text = text.clone();
                self.render_describe(frame, &name, &text);
            }
            AppState::Processes { name, .. } => {
                let name = name.clone();
                self.render_processes(frame, &name);
            }
        }
    }

    fn render_processes(&mut self, frame: &mut Frame, name: &str) {
        let layout = Layout::vertical([
            Constraint::Length(TOPBAR_HEIGHT),
            Constraint::Min(0),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(frame.area());

        let top_area = layout.first().copied().unwrap_or_default();
        let content_area = layout.get(1).copied().unwrap_or_default();
        let footer_area = layout.get(2).copied().unwrap_or_default();

        let hotkeys = process_hotkeys();
        frame.render_widget(
            TopBarView {
                summary: &format!("Processes: {name}"),
                recent_hotkeys: &[],
                hotkeys: &hotkeys,
                global_hotkeys: &[],
                app_name: APP_NAME,
                build_info: None,
            },
            top_area,
        );

        let n = self.processes.model.items.len();
        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .title(format!(" Processes: {name} [{n}] "))
            .title_alignment(Alignment::Center);
        let inner = block.inner(content_area);
        frame.render_widget(block, content_area);

        if n == 0 {
            Paragraph::new("No processes")
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

    fn render_connecting(frame: &mut Frame) {
        let area = frame.area();
        Paragraph::new("Connecting to Docker daemon…")
            .style(theme::muted())
            .alignment(Alignment::Center)
            .render(centered(area), frame.buffer_mut());
    }

    fn render_connect_error(frame: &mut Frame, message: &str) {
        let area = frame.area();
        let text = format!("{message}\n\nPress r to retry, q to quit.");
        Paragraph::new(text)
            .style(Style::default().fg(Color::Red))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .render(centered(area), frame.buffer_mut());
    }

    /// c8s's single-daemon analog of a fleet health dashboard: this daemon's
    /// version/info, since there's only ever one daemon connected to summarize.
    fn render_daemon_health(frame: &mut Frame, health: &DaemonHealth) {
        let area = frame.area();
        let mut text = health.lines().join("\n");
        text.push_str("\n\nPress i, q, or Esc to close.");
        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .title(" Daemon Health ")
            .title_alignment(Alignment::Center);
        let modal_area = centered_box(area, 42, 12);
        let inner = block.inner(modal_area);
        frame.render_widget(block, modal_area);
        Paragraph::new(text)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: true })
            .render(inner, frame.buffer_mut());
    }

    #[allow(clippy::too_many_lines)]
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

        let mut global: Vec<Hotkey> = VIEW_SWITCH_HOTKEYS.to_vec();
        global.extend(GLOBAL_HOTKEYS);
        if !self.list_filter.is_empty() {
            // `q` clears the active filter instead of quitting while one is
            // set (see event_handlers.rs); keep the hint truthful.
            if let Some(q) =
                global.iter_mut().find(|h| h.keycode == KeyCode::Char('q'))
            {
                *q = Hotkey::new('q', "clear filter");
            }
        }
        let label = self.view.to_string();
        let n = match self.view {
            ResourceKind::Containers => self.containers.model.items.len(),
            ResourceKind::Images => self.images.model.items.len(),
            ResourceKind::Volumes => self.volumes.model.items.len(),
            ResourceKind::Networks => self.networks.model.items.len(),
        };
        let summary = format!("{label}: {n}");

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

        let content_area =
            if self.list_search_open || !self.list_filter.is_empty() {
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
                    let cursor = if self.list_search_open { "_" } else { "" };
                    Paragraph::new(format!("/{}{cursor}", self.list_filter))
                        .render(inner, frame.buffer_mut());
                }
                search_layout.get(1).copied().unwrap_or_default()
            } else {
                content_area
            };

        let running =
            if self.running_only && self.view == ResourceKind::Containers {
                " running"
            } else {
                ""
            };
        let title = if self.list_filter.is_empty() {
            format!(" {label}{running} [{n}] ")
        } else {
            format!(" {label}{running} [{n} matches] ")
        };
        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .title(title)
            .title_alignment(Alignment::Center);
        let inner = block.inner(content_area);
        frame.render_widget(block, content_area);

        if n == 0 {
            Paragraph::new(format!("No {} found", label.to_lowercase()))
                .style(theme::muted())
                .alignment(Alignment::Center)
                .render(inner, frame.buffer_mut());
        } else {
            match self.view {
                ResourceKind::Containers => frame.render_stateful_widget(
                    DataTable::default(),
                    inner,
                    &mut self.containers,
                ),
                ResourceKind::Images => frame.render_stateful_widget(
                    DataTable::default(),
                    inner,
                    &mut self.images,
                ),
                ResourceKind::Volumes => frame.render_stateful_widget(
                    DataTable::default(),
                    inner,
                    &mut self.volumes,
                ),
                ResourceKind::Networks => frame.render_stateful_widget(
                    DataTable::default(),
                    inner,
                    &mut self.networks,
                ),
            }
        }

        frame.render_widget(self.status_line.clone(), footer_area);

        if let Some(dialog) = self.confirm_dialog.clone() {
            frame.render_widget(dialog, frame.area());
        }
    }

    fn render_logs(&mut self, frame: &mut Frame, name: &str) {
        let layout = Layout::vertical([
            Constraint::Length(TOPBAR_HEIGHT),
            Constraint::Min(0),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(frame.area());

        let top_area = layout.first().copied().unwrap_or_default();
        let content_area = layout.get(1).copied().unwrap_or_default();
        let footer_area = layout.get(2).copied().unwrap_or_default();

        let hotkeys = log_hotkeys();
        frame.render_widget(
            TopBarView {
                summary: &format!("Logs: {name}"),
                recent_hotkeys: &[],
                hotkeys: &hotkeys,
                global_hotkeys: &[],
                app_name: APP_NAME,
                build_info: None,
            },
            top_area,
        );

        let content_area =
            if self.log_search_open || !self.log_filter.is_empty() {
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
                    .title(" Search ")
                    .title_style(theme::title());
                let inner = search_block.inner(search_area);
                frame.render_widget(search_block, search_area);
                let cursor = if self.log_search_open { "_" } else { "" };
                Paragraph::new(format!("/{}{cursor}", self.log_filter))
                    .render(inner, frame.buffer_mut());
                search_layout.get(1).copied().unwrap_or_default()
            } else {
                content_area
            };

        let folded = fold(&self.log_lines, self.log_dedup);
        let filtered = filter_and_highlight(&folded, &self.log_filter);
        self.log_view_len = filtered.len();

        let dedup = self.log_dedup;
        let title = if !self.log_filter.is_empty() {
            format!(
                " Logs: {name} ({} matches, dedup: {dedup}) ",
                filtered.len()
            )
        } else if self.log_follow {
            format!(" Logs: {name} (live, dedup: {dedup}) ")
        } else {
            format!(" Logs: {name} (scrolled, G to follow, dedup: {dedup}) ")
        };
        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .title(title)
            .title_alignment(Alignment::Center);
        let inner = block.inner(content_area);
        frame.render_widget(block, content_area);

        // log_scroll is the absolute index of the first visible line.
        let visible = inner.height as usize;
        self.log_viewport_height = visible;
        let max_start = filtered.len().saturating_sub(visible);
        let start = if self.log_follow {
            max_start
        } else {
            self.log_scroll.min(max_start)
        };
        self.log_scroll = start;
        let end = (start + visible).min(filtered.len());
        let lines = filtered.get(start..end).unwrap_or(&[]).to_vec();
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .render(inner, frame.buffer_mut());

        frame.render_widget(self.status_line.clone(), footer_area);
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

fn centered(area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(3)])
        .flex(layout::Flex::Center)
        .split(area);
    let horizontal = Layout::horizontal([Constraint::Percentage(60)])
        .flex(layout::Flex::Center)
        .split(vertical.first().copied().unwrap_or(area));
    horizontal.first().copied().unwrap_or(area)
}

#[cfg(test)]
mod tests {
    use ratatui::{
        Terminal,
        backend::TestBackend,
        buffer::{Buffer, Cell},
    };

    use crate::{
        app::App,
        app_state::AppState,
        docker::{ContainerRow, client::DaemonHealth},
    };

    fn render(app: &mut App) -> Buffer {
        let mut terminal =
            Terminal::new(TestBackend::new(120, 24)).expect("test terminal");
        terminal.draw(|f| app.render(f)).expect("draw");
        terminal.backend().buffer().clone()
    }

    fn text(buf: &Buffer) -> String {
        let width = usize::from(buf.area.width);
        buf.content()
            .chunks(width)
            .map(|row| row.iter().map(Cell::symbol).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn app() -> App {
        let mut app = App::new();
        app.state = AppState::List;
        app.containers_all = vec![
            ContainerRow {
                name: "web".into(),
                image: "nginx".into(),
                ..Default::default()
            },
            ContainerRow {
                name: "db".into(),
                image: "postgres".into(),
                ..Default::default()
            },
        ];
        app.reapply_list_filter();
        app
    }

    #[test]
    fn no_filter_shows_quit_hint_and_no_search_bar() {
        let mut app = app();
        let out = text(&render(&mut app));
        assert!(!out.contains(" Search "));
        assert!(out.contains("quit"));
        assert!(!out.contains("clear filter"));
        assert!(out.contains("[2]"));
    }

    #[test]
    fn open_search_shows_bar_with_cursor() {
        let mut app = app();
        app.list_search_open = true;
        app.list_filter = "ngi".into();
        app.reapply_list_filter();
        let out = text(&render(&mut app));
        assert!(out.contains(" Search "));
        assert!(out.contains("/ngi_"));
    }

    #[test]
    fn active_filter_shows_bar_matches_and_clear_hint() {
        let mut app = app();
        app.list_filter = "ngi".into();
        app.reapply_list_filter();
        let out = text(&render(&mut app));
        assert!(out.contains(" Search "));
        assert!(out.contains("/ngi"));
        assert!(!out.contains("/ngi_"));
        assert!(out.contains("clear filter"));
        assert!(out.contains("[1 matches]"));
        assert!(out.contains("nginx"));
        assert!(!out.contains("postgres"));
    }

    #[test]
    fn info_state_renders_daemon_health_panel() {
        let mut app = App::new();
        app.state = AppState::Info(DaemonHealth {
            server_version: "28.0.1".into(),
            api_version: "1.48".into(),
            os: "linux".into(),
            arch: "arm64".into(),
            containers_running: 3,
            containers_paused: 1,
            containers_stopped: 7,
            images: 12,
        });
        let out = text(&render(&mut app));
        for want in [
            " Daemon Health ",
            "Docker version: 28.0.1",
            "API version: 1.48",
            "OS/Arch: linux/arm64",
            "Containers running: 3",
            "Containers paused: 1",
            "Containers stopped: 7",
            "Images: 12",
            "Press i, q, or Esc to close.",
        ] {
            assert!(out.contains(want), "missing {want:?} in:\n{out}");
        }
    }

    #[test]
    fn info_panel_fits_a_small_terminal_without_panicking() {
        let mut app = App::new();
        app.state = AppState::Info(DaemonHealth::default());
        let mut terminal =
            Terminal::new(TestBackend::new(20, 5)).expect("test terminal");
        terminal.draw(|f| app.render(f)).expect("draw");
    }

    #[test]
    fn command_bar_renders_colon_prompt_text_and_ghost() {
        let mut app = app();
        app.command_mode = true;
        app.list_search_open = true;
        app.command_text = "vol".into();
        let out = text(&render(&mut app));
        assert!(out.contains(" Command "));
        assert!(out.contains(":volumes"));
        assert!(!out.contains(" Search "));
    }

    #[test]
    fn command_bar_first_char_sits_right_after_prompt() {
        let mut app = app();
        app.command_mode = true;
        app.list_search_open = true;
        app.command_text = "vol".into();
        let buf = render(&mut app);
        let rows: Vec<&[Cell]> =
            buf.content().chunks(usize::from(buf.area.width)).collect();
        let title = rows
            .iter()
            .position(|r| {
                r.iter()
                    .map(Cell::symbol)
                    .collect::<String>()
                    .contains(" Command ")
            })
            .expect("title row");
        let cells = rows.get(title + 1).expect("bar row");
        let colon = cells
            .iter()
            .position(|c| c.symbol() == ":")
            .expect("prompt cell");
        let after = |n: usize| cells.get(colon + n).expect("cell").symbol();
        assert_eq!(after(1), "v", "first char right after ':'");
        assert_eq!(after(3), "l", "last typed char");
        assert_eq!(after(4), "u", "ghost right after typed text");
    }
}
