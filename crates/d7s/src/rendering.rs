use core::fmt;

use k9tui::{
    theme,
    widgets::{
        hotkey::Hotkey,
        table::{DataTable, TableData},
        top_bar::TopBarView,
    },
};
use ratatui::{
    Frame,
    prelude::*,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{APP_NAME, App},
    app_state::DatabaseExplorerState,
    db::{connection::Connection, query_log::QueryLogEntry},
    filtered_data::FilteredData,
    ui::{
        sql_executor::SqlExecutor,
        theme as d7s_theme,
        widgets::{
            connection_modal::ConnectionModalWidget,
            describe_content::DescribeRow, global_hotkeys::global_hotkeys,
            help_content::HelpRow,
            hotkeys::{CELL_EDIT_HOTKEYS, TABLE_DATA_VIEW_HOTKEYS},
            idle_hint::default_idle_hint,
        },
    },
};

const TOPBAR_HEIGHT: u16 = 7;
const FOOTER_HEIGHT: u16 = 1;
const FILTER_BAR_HEIGHT: u16 = 3;

impl App<'_> {
    #[allow(clippy::too_many_lines)]
    pub fn render(&mut self, frame: &mut Frame) {
        let idle_hint =
            default_idle_hint(self.state, &self.database_explorer.state);
        self.status_line.set_idle_hint(if self.watch_active {
            format!("● WATCHING · {idle_hint}")
        } else {
            idle_hint
        });

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(TOPBAR_HEIGHT),
                Constraint::Min(0),
                Constraint::Length(FOOTER_HEIGHT),
            ])
            .split(frame.area());

        let top_area = layout.first().copied().unwrap_or_else(Rect::default);
        let content_area = layout.get(1).copied().unwrap_or_else(Rect::default);
        let footer_area = layout.get(2).copied().unwrap_or_else(Rect::default);

        let on_connection_list = matches!(
            self.database_explorer.state,
            DatabaseExplorerState::Connections
        );
        let global = global_hotkeys(on_connection_list);

        let (connection, build_info, recent_hotkeys) = if on_connection_list {
            (
                &Connection::default(),
                Some(self.build_info.clone()),
                Vec::new(),
            )
        } else {
            (
                &self.database_explorer.connection,
                None,
                self.database_explorer.recent_table_hotkeys(),
            )
        };

        let summary = connection.summary_stack();

        let table_data_ext: Vec<Hotkey> = if matches!(
            self.database_explorer.state,
            DatabaseExplorerState::TableData(_, _)
        ) {
            let table_hotkeys: &[Hotkey] = if self.cell_edit.is_some() {
                &CELL_EDIT_HOTKEYS
            } else {
                &TABLE_DATA_VIEW_HOTKEYS
            };
            self.hotkeys
                .iter()
                .chain(table_hotkeys)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let hotkey_bar: &[Hotkey] = if table_data_ext.is_empty() {
            &self.hotkeys
        } else {
            &table_data_ext
        };

        frame.render_widget(
            TopBarView {
                summary: &summary,
                recent_hotkeys: recent_hotkeys.as_slice(),
                hotkeys: hotkey_bar,
                global_hotkeys: global.as_slice(),
                app_name: APP_NAME,
                build_info,
            },
            top_area,
        );

        let main_area = if self.search_filter.is_some() {
            let search_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(FILTER_BAR_HEIGHT),
                    Constraint::Min(0),
                ])
                .split(content_area);

            let search_layout_rect =
                search_layout.first().copied().unwrap_or_else(Rect::default);

            if let Some(textarea) = &self.search_filter {
                let title = if self.command_mode {
                    " Command "
                } else {
                    " Filter (current view) "
                };
                let filter_block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border())
                    .title(title)
                    .title_style(theme::title());
                let inner = filter_block.inner(search_layout_rect);
                frame.render_widget(filter_block, search_layout_rect);
                if self.command_mode {
                    let [prompt, text] = Layout::horizontal([
                        Constraint::Length(1),
                        Constraint::Min(0),
                    ])
                    .areas(inner);
                    frame.render_widget(
                        Paragraph::new(":").style(theme::accent()),
                        prompt,
                    );
                    frame.render_widget(textarea, text);
                    self.render_command_ghost(frame, text);
                } else {
                    frame.render_widget(textarea, inner);
                }
            }

            search_layout.get(1).copied().unwrap_or_else(Rect::default)
        } else {
            content_area
        };

        let block = Block::new()
            .borders(Borders::ALL)
            .border_style(theme::border())
            .title(self.panel_title_line())
            .title_alignment(Alignment::Center);

        let inner_area = block.inner(main_area);
        frame.render_widget(block, main_area);
        self.render_database_table(frame, inner_area);

        if let Some(hint) = self.empty_state_hint() {
            Paragraph::new(hint)
                .style(theme::muted())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .render(inner_area, frame.buffer_mut());
        }

        frame.render_widget(self.status_line.clone(), footer_area);

        self.render_modals(frame);
    }

    #[allow(clippy::option_if_let_else)] // clearer than nested map_or_else
    fn panel_title_line(&self) -> Line<'static> {
        if self.show_help {
            return Line::from(Span::styled(" Help ", theme::title()));
        }
        if self.show_query_log {
            return Line::from(Span::styled(" Query log ", theme::title()));
        }

        if self.show_describe {
            return Line::from(Span::styled(" Describe ", theme::title()));
        }

        if matches!(
            self.database_explorer.state,
            DatabaseExplorerState::Connections
        ) {
            let n = self.database_explorer.connections.table.model.items.len();
            return Line::from(vec![
                Span::styled(" Connections ", theme::title()),
                Span::styled(format!("[{n}] "), theme::accent()),
            ]);
        }

        let conn = &self.database_explorer.connection;
        let env_tag = format!("[{}]", conn.environment);

        let body = match &self.database_explorer.state {
            DatabaseExplorerState::TableData(_, _) => {
                let base = self.database_explorer.state.to_string();
                match &self.database_explorer.table_data_virtual {
                    Some(meta) => {
                        let filtered = self
                            .database_explorer
                            .table_data
                            .as_ref()
                            .is_some_and(FilteredData::is_filtered);
                        let (visible, local_draft_rows) = self
                            .database_explorer
                            .table_data
                            .as_ref()
                            .map_or((0, 0), |t| {
                                let vis = t.table.model.items.len();
                                let dr = t
                                    .table
                                    .model
                                    .items
                                    .iter()
                                    .filter(|r| r.is_draft)
                                    .count();
                                (vis, dr)
                            });
                        format!(
                            "{}{}",
                            base.trim(),
                            meta.title_suffix(
                                filtered,
                                visible,
                                local_draft_rows
                            )
                        )
                    }
                    None => base.trim().to_string(),
                }
            }
            DatabaseExplorerState::Connections
            | DatabaseExplorerState::Databases
            | DatabaseExplorerState::Schemas
            | DatabaseExplorerState::Tables(_)
            | DatabaseExplorerState::Columns(_, _)
            | DatabaseExplorerState::SqlResults(_) => {
                let base = self.database_explorer.state.to_string();
                if self.has_active_filter() {
                    format!("{} · filtered", base.trim())
                } else {
                    base.trim().to_string()
                }
            }
        };

        let prefix = {
            let db = conn
                .selected_database
                .as_deref()
                .unwrap_or(conn.name.as_str());
            format!("{db} · ")
        };

        Line::from(vec![
            Span::styled(env_tag, d7s_theme::env_style(conn.environment)),
            Span::styled(format!(" {prefix}{body}"), theme::title()),
        ])
    }

    fn empty_state_hint(&self) -> Option<&'static str> {
        if self.show_help || self.show_describe || self.show_query_log {
            return None;
        }

        if self.has_active_filter() && self.active_table_is_empty() {
            return Some("No matches — Esc to clear filter");
        }

        match &self.database_explorer.state {
            DatabaseExplorerState::Connections
                if self
                    .database_explorer
                    .connections
                    .table
                    .model
                    .items
                    .is_empty() =>
            {
                Some("Press n to add your first connection")
            }
            DatabaseExplorerState::Connections
            | DatabaseExplorerState::Databases
            | DatabaseExplorerState::Schemas
            | DatabaseExplorerState::Tables(_)
            | DatabaseExplorerState::Columns(_, _)
            | DatabaseExplorerState::TableData(_, _)
            | DatabaseExplorerState::SqlResults(_) => None,
        }
    }

    fn active_table_is_empty(&self) -> bool {
        let explorer = &self.database_explorer;
        match &explorer.state {
            DatabaseExplorerState::Connections => {
                explorer.connections.table.model.items.is_empty()
            }
            DatabaseExplorerState::Databases => explorer
                .databases
                .as_ref()
                .is_some_and(|d| d.table.model.items.is_empty()),
            DatabaseExplorerState::Schemas => explorer
                .schemas
                .as_ref()
                .is_some_and(|d| d.table.model.items.is_empty()),
            DatabaseExplorerState::Tables(_) => explorer
                .tables
                .as_ref()
                .is_some_and(|d| d.table.model.items.is_empty()),
            DatabaseExplorerState::Columns(_, _) => explorer
                .columns
                .as_ref()
                .is_some_and(|d| d.table.model.items.is_empty()),
            DatabaseExplorerState::TableData(_, _) => explorer
                .table_data
                .as_ref()
                .is_some_and(|d| d.table.model.items.is_empty()),
            DatabaseExplorerState::SqlResults(_) => false,
        }
    }

    pub fn render_modals(&mut self, frame: &mut Frame) {
        if !self.modal_manager.is_any_modal_open() {
            return;
        }

        let area = frame.area();

        if let Some(modal) = self.modal_manager.get_connection_modal_mut() {
            frame.render_stateful_widget(ConnectionModalWidget, area, modal);
        }

        if let Some(modal) = self.modal_manager.get_confirmation_modal() {
            frame.render_widget(modal.clone(), area);
        }

        if let Some(modal) =
            self.modal_manager.get_sql_execution_confirmation_modal()
        {
            frame.render_widget(modal.clone(), area);
        }

        if let Some(modal) = self.modal_manager.get_sql_query_selection_modal()
        {
            frame.render_widget(modal.clone(), area);
        }

        if let Some(modal) = self.modal_manager.get_cell_value_modal() {
            frame.render_widget(modal.clone(), area);
        }

        if let Some(modal) = self.modal_manager.get_password_modal() {
            frame.render_widget(modal.clone(), area);
        }

        if let Some(modal) = self.modal_manager.get_jump_to_row_modal() {
            frame.render_widget(modal.clone(), area);
        }
    }

    /// Dimmed best-match completion drawn right after the typed text; its
    /// first cell is reversed so it doubles as the cursor.
    fn render_command_ghost(&self, frame: &mut Frame, text_area: Rect) {
        let typed = self.command_text();
        let Some(full) = self.command_suggestion() else {
            return;
        };
        let ghost: String = full.chars().skip(typed.chars().count()).collect();
        let x = text_area.x.saturating_add(
            u16::try_from(UnicodeWidthStr::width(typed.as_str()))
                .unwrap_or(u16::MAX),
        );
        let width = text_area.right().saturating_sub(x);
        if width == 0 {
            return;
        }
        let mut chars = ghost.chars();
        let first: String = chars.by_ref().take(1).collect();
        let line = Line::from(vec![
            Span::styled(
                first,
                theme::muted().add_modifier(Modifier::REVERSED),
            ),
            Span::styled(chars.collect::<String>(), theme::muted()),
        ]);
        frame.render_widget(
            Paragraph::new(line),
            Rect::new(x, text_area.y, width, 1),
        );
    }

    pub fn render_database_table(&mut self, frame: &mut Frame, area: Rect) {
        if self.show_help {
            frame.render_stateful_widget(
                DataTable::<HelpRow>::default(),
                area,
                &mut self.help_table,
            );
            return;
        }

        if self.show_describe {
            frame.render_stateful_widget(
                DataTable::<DescribeRow>::default(),
                area,
                &mut self.describe_table,
            );
            return;
        }

        if self.show_query_log {
            frame.render_stateful_widget(
                DataTable::<QueryLogEntry>::default(),
                area,
                &mut self.query_log_table,
            );
            return;
        }

        let explorer = &self.database_explorer;
        match &explorer.state {
            DatabaseExplorerState::Connections => {
                frame.render_stateful_widget(
                    DataTable::<Connection>::default(),
                    area,
                    &mut self.database_explorer.connections.table,
                );
            }
            DatabaseExplorerState::Databases => {
                render_filtered_data_table(
                    frame,
                    explorer.databases.as_ref(),
                    area,
                );
            }
            DatabaseExplorerState::Schemas => {
                render_filtered_data_table(
                    frame,
                    explorer.schemas.as_ref(),
                    area,
                );
            }
            DatabaseExplorerState::Tables(_) => {
                render_filtered_data_table(
                    frame,
                    explorer.tables.as_ref(),
                    area,
                );
            }
            DatabaseExplorerState::Columns(_, _) => {
                render_filtered_data_table(
                    frame,
                    explorer.columns.as_ref(),
                    area,
                );
            }
            DatabaseExplorerState::TableData(_, _) => {
                render_filtered_data_table(
                    frame,
                    explorer.table_data.as_ref(),
                    area,
                );
            }
            DatabaseExplorerState::SqlResults(_) => {
                frame.render_stateful_widget(
                    SqlExecutor,
                    area,
                    &mut self.database_explorer.sql_executor,
                );
            }
        }
    }
}

fn render_filtered_data_table<T: TableData + Clone + fmt::Debug>(
    frame: &mut Frame,
    filtered_data: Option<&FilteredData<T>>,
    area: Rect,
) {
    if let Some(filtered_data) = filtered_data {
        frame.render_stateful_widget(
            DataTable::<T>::default(),
            area,
            &mut filtered_data.table.clone(),
        );
    }
}
