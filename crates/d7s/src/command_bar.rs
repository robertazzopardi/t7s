//! `:` command bar. Reuses the `/` search bar slot (`search_filter`), with
//! `command_mode` switching its meaning from row filter to command line.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use k9tui::widgets::table::TableDataState;
use ratatui::style::{Color, Style};
use ratatui_textarea::TextArea;

use crate::{
    app::App,
    app_state::{AppState, DatabaseExplorerState},
    command::{
        Command, Scope, TablePath, Verb, match_name_candidates, resolve,
        suggest,
    },
    db::connection::ConnectionType,
    services::PreferencesService,
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

    fn command_scope(&self) -> ScopeNames {
        let ex = &self.database_explorer;
        let connected = self.state == AppState::DatabaseConnected;
        ScopeNames {
            tables: self.command_table_names(),
            schemas: self.command_schema_names(),
            databases: ex.databases.as_ref().map_or_else(Vec::new, |d| {
                d.original.iter().map(|d| d.name.clone()).collect()
            }),
            connections: ex
                .connections
                .original
                .iter()
                .map(|c| (c.name.clone(), c.r#type == ConnectionType::Sqlite))
                .collect(),
            conn: if connected {
                ex.connection.name.clone()
            } else {
                String::new()
            },
            db: ex.connection.selected_database.clone().unwrap_or_default(),
            schema: self.command_schema().unwrap_or_default(),
        }
    }

    /// Dimmed completion shown after the typed text (Tab accepts).
    pub(crate) fn command_suggestion(&self) -> Option<String> {
        let text = self.command_text();
        self.command_scope()
            .with(|sc| suggest(&text, sc))
            .filter(|s| {
                // `activity` is Postgres-only: don't advertise it elsewhere.
                s != "activity"
                    || self
                        .database_explorer
                        .connection
                        .r#type
                        .has_activity_view()
            })
    }

    pub(crate) fn handle_command_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => self.quit(),
            (_, KeyCode::Esc) => self.close_command_bar(),
            // Ctrl+J would insert a newline into a single-line bar.
            (KeyModifiers::CONTROL, KeyCode::Char('j' | 'J' | 'm' | 'M')) => {}
            (_, KeyCode::Enter) => {
                let text = self.command_text();
                self.close_command_bar();
                self.execute_command(&text);
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
            && let Some(last) = PreferencesService::last_sql()
        {
            sql.set_sql(&last);
        }
        self.open_editor_requested = true;
    }

    /// Resolve and run a command line.
    pub(crate) fn execute_command(&mut self, text: &str) {
        let cmd = self.command_scope().with(|sc| resolve(text, sc));
        self.pending_nav = None;
        if let Command::TablePath(path) = cmd {
            // Paths may start from the connection list: skip the connected check.
            if self.has_table_draft_rows() {
                self.set_status(DRAFT_PENDING);
            } else {
                self.pending_nav = Some(PendingNav {
                    path,
                    awaiting: NavStep::Start,
                });
                self.advance_table_path(NavStep::Start);
            }
            return;
        }
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
            return;
        }
        if changes_view && self.has_table_draft_rows() {
            self.set_status(DRAFT_PENDING);
            return;
        }
        let sqlite =
            self.database_explorer.connection.r#type == ConnectionType::Sqlite;
        match cmd {
            Command::Empty | Command::TablePath(_) => {}
            Command::NoConnection(x) => {
                self.set_status(format!("No connection matching '{x}'"));
            }
            Command::BadTablePath(x) => self.set_status(format!(
                "Bad path '{x}': use conn.db.schema.table (SQLite: conn.table)"
            )),
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
                    self.jump_to_table_row(n);
                } else {
                    self.set_status("Row jump needs an open table.");
                }
            }
            Command::Verb(v) => self.run_verb(v),
            Command::Table(candidates) => {
                self.jump_to_table_by_name(&candidates);
            }
            Command::Schema(candidates) => {
                self.jump_to_schema_by_name(&candidates);
            }
        }
    }

    fn run_verb(&mut self, verb: Verb) {
        match verb {
            Verb::Connections => {
                self.disconnect_from_database();
                self.refresh_connections();
            }
            Verb::Schemas => {
                if self.database_explorer.connection.r#type
                    == ConnectionType::Sqlite
                {
                    self.set_status("SQLite has no schemas.");
                } else {
                    self.load_schemas();
                }
            }
            Verb::Tables => match self.command_schema() {
                Some(schema) => self.load_tables(&schema),
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
                    Some((s, t)) => self.load_columns(&s, &t),
                    None => self.set_status("Select a table first."),
                }
            }
            Verb::Sql => self.request_editor(),
            Verb::Activity => self.open_activity_view(),
            Verb::Help | Verb::Log | Verb::Quit => {}
        }
    }

    fn await_nav(&mut self, path: TablePath, awaiting: NavStep) {
        self.pending_nav = Some(PendingNav { path, awaiting });
    }

    /// Drive a `:table conn.db.schema.table` jump one hop. Called with what
    /// just loaded; a hop that must wait spawns its load and returns.
    pub(crate) fn advance_table_path(&mut self, loaded: NavStep) {
        let Some(PendingNav { mut path, .. }) =
            self.pending_nav.take_if(|n| n.awaiting == loaded)
        else {
            return;
        };
        let ex = &self.database_explorer;
        if self.state != AppState::DatabaseConnected
            || ex.connection.name != path.conn
        {
            // `apply_connected` loads databases (SQLite: tables) next.
            let conn = path.conn.clone();
            let next = if path.db.is_some() {
                NavStep::Databases
            } else {
                NavStep::Tables
            };
            self.await_nav(path, next);
            self.connect_to_named_connection(&conn);
            return;
        }
        if let Some(db) = path.db.take() {
            let Some(dbs) = &ex.databases else {
                path.db = Some(db);
                self.database_explorer.state = DatabaseExplorerState::Databases;
                self.await_nav(path, NavStep::Databases);
                self.load_databases();
                return;
            };
            let names: Vec<&str> =
                dbs.original.iter().map(|d| d.name.as_str()).collect();
            let Some(hit) = match_name_candidates(&names, &db).first().copied()
            else {
                self.set_status(format!("No database matching '{db}'"));
                return;
            };
            if ex.connection.selected_database.as_deref() != Some(hit)
                || ex.schemas.is_none()
            {
                let hit = hit.to_string();
                // `apply_database_selected` loads schemas next.
                self.database_explorer.state = DatabaseExplorerState::Databases;
                self.await_nav(path, NavStep::Schemas);
                self.spawn_select_database(&hit);
                return;
            }
        }
        if let Some(schema) = path.schema.take() {
            let names = self.command_schema_names();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let hits = match_name_candidates(&refs, &schema);
            let Some(&hit) = hits.first() else {
                self.set_status(format!("No schema matching '{schema}'"));
                return;
            };
            let ex = &self.database_explorer;
            if ex.state.schema_name().as_deref() != Some(hit)
                || ex.tables.is_none()
            {
                let hits: Vec<String> =
                    hits.into_iter().map(String::from).collect();
                self.await_nav(path, NavStep::Tables);
                self.jump_to_schema_by_name(&hits);
                return;
            }
        }
        let names = self.command_table_names();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let hits: Vec<String> = match_name_candidates(&refs, &path.table)
            .into_iter()
            .map(String::from)
            .collect();
        if hits.is_empty() {
            self.set_status(format!("No table matching '{}'", path.table));
        } else {
            self.jump_to_table_by_name(&hits);
        }
    }
}

/// What an in-progress `:table` path jump waits for before its next hop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavStep {
    Start,
    Databases,
    Schemas,
    Tables,
}

#[derive(Debug, Clone)]
pub struct PendingNav {
    path: TablePath,
    awaiting: NavStep,
}

/// Owned names behind a borrowed `command::Scope`.
struct ScopeNames {
    tables: Vec<String>,
    schemas: Vec<String>,
    databases: Vec<String>,
    connections: Vec<(String, bool)>,
    conn: String,
    db: String,
    schema: String,
}

impl ScopeNames {
    fn with<R>(&self, f: impl FnOnce(&Scope) -> R) -> R {
        fn refs(v: &[String]) -> Vec<&str> {
            v.iter().map(String::as_str).collect()
        }
        let tables = refs(&self.tables);
        let schemas = refs(&self.schemas);
        let databases = refs(&self.databases);
        let connections: Vec<(&str, bool)> = self
            .connections
            .iter()
            .map(|(n, s)| (n.as_str(), *s))
            .collect();
        f(&Scope {
            tables: &tables,
            schemas: &schemas,
            databases: &databases,
            connections: &connections,
            conn: &self.conn,
            db: &self.db,
            schema: &self.schema,
        })
    }
}
