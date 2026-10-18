//! `:` command bar. Reuses the `/` search bar slot (`search_filter`), with
//! `command_mode` switching its meaning from row filter to command line.

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use k9tui::widgets::table::TableDataState;
use ratatui::style::{Color, Style};
use ratatui_textarea::TextArea;

use crate::{
    app::App,
    app_state::{AppState, DatabaseExplorerState},
    command::{Command, Verb, resolve, suggest},
    db::connection::ConnectionType,
    ui::widgets::help_content::help_rows,
};

const DRAFT_PENDING: &str = "Draft row pending - Esc to discard, s to commit";

fn bar(text: &str) -> TextArea<'static> {
    let mut bar = TextArea::default();
    bar.set_cursor_line_style(Style::default());
    bar.set_style(Style::default().fg(Color::White));
    bar.set_max_histories(0);
    bar.insert_str(text);
    bar
}

impl App<'_> {
    pub(crate) fn open_command_bar(&mut self) {
        self.search_filter = Some(bar(""));
        self.command_mode = true;
    }

    fn close_command_bar(&mut self) {
        self.search_filter = None;
        self.command_mode = false;
    }

    /// Text currently typed in the command bar.
    pub(crate) fn command_text(&self) -> String {
        self.search_filter
            .as_ref()
            .and_then(|t| t.lines().first().cloned())
            .unwrap_or_default()
    }

    /// Schema whose tables `:table <name>` / `:tables` refer to.
    pub(crate) fn command_schema(&self) -> Option<String> {
        if self.state != AppState::DatabaseConnected {
            return None;
        }
        self.database_explorer.state.schema_name().or_else(|| {
            (self.database_explorer.connection.r#type == ConnectionType::Sqlite)
                .then(|| "sqlite_schema".to_string())
        })
    }

    fn command_table_names(&self) -> Vec<String> {
        if self.command_schema().is_none() {
            return Vec::new();
        }
        self.database_explorer
            .tables
            .as_ref()
            .map_or_else(Vec::new, |t| {
                t.original.iter().map(|t| t.name.clone()).collect()
            })
    }

    fn command_schema_names(&self) -> Vec<String> {
        if self.database_explorer.connection.r#type == ConnectionType::Sqlite {
            return Vec::new();
        }
        self.database_explorer
            .schemas
            .as_ref()
            .map_or_else(Vec::new, |s| {
                s.original.iter().map(|s| s.name.clone()).collect()
            })
    }

    /// Dimmed completion shown after the typed text (Tab accepts).
    pub(crate) fn command_suggestion(&self) -> Option<String> {
        let tables = self.command_table_names();
        let schemas = self.command_schema_names();
        let table_refs: Vec<&str> = tables.iter().map(String::as_str).collect();
        let schema_refs: Vec<&str> =
            schemas.iter().map(String::as_str).collect();
        suggest(&self.command_text(), &table_refs, &schema_refs).filter(|s| {
            // `activity` is Postgres-only: don't advertise it elsewhere.
            s != "activity"
                || self.database_explorer.connection.r#type.has_activity_view()
        })
    }

    pub(crate) async fn handle_command_key(
        &mut self,
        key: KeyEvent,
    ) -> Result<()> {
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => self.quit(),
            (_, KeyCode::Esc) => self.close_command_bar(),
            // Ctrl+J would insert a newline into a single-line bar.
            (KeyModifiers::CONTROL, KeyCode::Char('j' | 'J' | 'm' | 'M')) => {}
            (_, KeyCode::Enter) => {
                let text = self.command_text();
                self.close_command_bar();
                self.execute_command(&text).await?;
            }
            (_, KeyCode::Tab) => {
                if let Some(s) = self.command_suggestion() {
                    self.search_filter = Some(bar(&s));
                }
            }
            _ => {
                if let Some(t) = &mut self.search_filter {
                    t.input(key);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn open_help(&mut self) {
        self.help_table = TableDataState::new(help_rows(
            self.state,
            &self.database_explorer.state,
        ));
        self.show_help = true;
    }

    pub(crate) fn request_editor(&mut self) {
        let sql = &mut self.database_explorer.sql_executor;
        if sql.sql_input().trim().is_empty()
            && let Some(last) = crate::services::PreferencesService::last_sql()
        {
            sql.set_sql(&last);
        }
        self.open_editor_requested = true;
    }

    /// Resolve and run a command line.
    pub(crate) async fn execute_command(&mut self, text: &str) -> Result<()> {
        let tables = self.command_table_names();
        let schemas = self.command_schema_names();
        let table_refs: Vec<&str> = tables.iter().map(String::as_str).collect();
        let schema_refs: Vec<&str> =
            schemas.iter().map(String::as_str).collect();
        let cmd = resolve(text, &table_refs, &schema_refs);
        let changes_view = matches!(
            cmd,
            Command::Table(_)
                | Command::NoTable(_)
                | Command::Schema(_)
                | Command::NoSchema(_)
                | Command::Row(_)
                | Command::Verb(
                    Verb::Connections
                        | Verb::Schemas
                        | Verb::Tables
                        | Verb::Columns
                        | Verb::Sql
                        | Verb::Activity
                )
        );
        if changes_view && self.state != AppState::DatabaseConnected {
            self.set_status(if cmd == Command::Verb(Verb::Connections) {
                "Already on the connections list."
            } else {
                "Not connected - open a connection first (Enter on a connection)."
            });
            return Ok(());
        }
        if changes_view && self.has_table_draft_rows() {
            self.set_status(DRAFT_PENDING);
            return Ok(());
        }
        let sqlite =
            self.database_explorer.connection.r#type == ConnectionType::Sqlite;
        match cmd {
            Command::Empty => {}
            Command::NoTable(x) => {
                self.set_status(format!("No table matching '{x}'"));
            }
            Command::Schema(_) | Command::NoSchema(_) if sqlite => {
                self.set_status("SQLite has no schemas.");
            }
            Command::NoSchema(x) => {
                self.set_status(format!("No schema matching '{x}'"));
            }
            Command::Unknown(x) => {
                self.set_status(format!("Unknown command: {x}"));
            }
            Command::Ambiguous(c) => self.set_status(format!(
                "Ambiguous command: {} ({})",
                text.trim(),
                c.join(", ")
            )),
            Command::Verb(Verb::Quit) => self.quit(),
            Command::Verb(Verb::Help) => self.open_help(),
            Command::Verb(Verb::Log) => self.open_query_log_view(),
            Command::Row(n) => {
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::TableData(_, _)
                ) {
                    self.jump_to_table_row(n).await?;
                } else {
                    self.set_status("Row jump needs an open table.");
                }
            }
            Command::Verb(v) => self.run_verb(v).await?,
            Command::Table(candidates) => {
                self.jump_to_table_by_name(&candidates).await?;
            }
            Command::Schema(candidates) => {
                self.jump_to_schema_by_name(&candidates).await?;
            }
        }
        Ok(())
    }

    async fn run_verb(&mut self, verb: Verb) -> Result<()> {
        match verb {
            Verb::Connections => {
                self.disconnect_from_database();
                self.refresh_connections().await;
            }
            Verb::Schemas => {
                if self.database_explorer.connection.r#type
                    == ConnectionType::Sqlite
                {
                    self.set_status("SQLite has no schemas.");
                } else {
                    self.load_schemas().await?;
                }
            }
            Verb::Tables => match self.command_schema() {
                Some(schema) => self.load_tables(&schema).await?,
                None => self.set_status("Open a schema first."),
            },
            Verb::Columns => {
                let target = match &self.database_explorer.state {
                    DatabaseExplorerState::TableData(s, t)
                    | DatabaseExplorerState::Columns(s, t) => {
                        Some((s.clone(), t.clone()))
                    }
                    DatabaseExplorerState::Tables(s) => {
                        self.get_selected_table_name().map(|t| (s.clone(), t))
                    }
                    DatabaseExplorerState::Connections
                    | DatabaseExplorerState::Databases
                    | DatabaseExplorerState::Schemas
                    | DatabaseExplorerState::SqlResults(_) => None,
                };
                match target {
                    Some((s, t)) => self.load_columns(&s, &t).await?,
                    None => self.set_status("Select a table first."),
                }
            }
            Verb::Sql => self.request_editor(),
            Verb::Activity => self.open_activity_view().await,
            Verb::Help | Verb::Log | Verb::Quit => {}
        }
        Ok(())
    }
}
