#![allow(clippy::indexing_slicing)]

use std::{fmt::Display, str::FromStr};

use crossterm::event::{KeyCode, KeyEvent};
use k9tui::{
    theme,
    widgets::{
        buttons::Buttons,
        modal::{
            ConfirmDialog, DialogAction, ModalField, PromptValidation,
            TextPromptModal,
        },
    },
};
use ratatui::{
    prelude::{
        Alignment, Buffer, Constraint, Direction, Layout, Line, Rect, Widget,
    },
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders, Clear, Paragraph, StatefulWidget},
};
use ratatui_textarea::TextArea;
use tui_menu::{MenuEvent, MenuItem, MenuState};
use unicode_width::UnicodeWidthStr;

use crate::db::{
    DbRowId,
    connection::{
        Connection, ConnectionType, build_server_url, default_database,
        default_port, parse_connection_string, parse_server_url,
        with_tls_query,
    },
    tls::TlsConfig,
};

// Modal dimension constants
const CONNECTION_MODAL_WIDTH: u16 = 40;
const STEP1_MODAL_WIDTH: u16 = 48;

#[derive(Clone, Copy, Debug, Default)]
pub enum Mode {
    #[default]
    New,
    Edit,
}

#[derive(Clone, Debug, Default)]
pub enum ModalType {
    #[default]
    Connection,
    Confirmation,
    SqlExecutionConfirmation,
    SqlQuerySelection,
    CellValue,
    Password,
    JumpToRow,
}

#[derive(Clone, Debug, Default)]
pub enum TestResult {
    #[default]
    NotTested,
    Testing,
    Success,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PasswordStorageType {
    #[default]
    Keyring,
    DontSave,
}

impl Display for PasswordStorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Keyring => write!(f, "keyring"),
            Self::DontSave => write!(f, "dont_save"),
        }
    }
}

/// Step of the new-connection flow: choose type + optional URL, then connection form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ConnectionModalStep {
    #[default]
    /// Step 1: selectable list of database types and optional "Import from URL" input.
    ChooseType,
    /// Step 2: type-specific form (Postgres: Host/Port/User/DB; `SQLite`: Path).
    ConnectionForm,
}

/// Database types shown in step 1 list (order matches `step1_type_index`).
const STEP1_DB_TYPES: [&str; 3] = ["postgres", "sqlite", "mysql"];

/// Connection type for a step 1 list entry (unknown index falls back to Postgres).
fn step1_connection_type(index: usize) -> ConnectionType {
    STEP1_DB_TYPES
        .get(index)
        .and_then(|s| s.parse().ok())
        .unwrap_or_default()
}

/// Index of the Database field in the Postgres/MySQL forms.
const DATABASE_FIELD: usize = 4;
/// TLS mode dropdown and optional CA certificate path, right after Database.
const TLS_FIELD: usize = 5;
const CA_FIELD: usize = 6;
const TLS_MODES: [&str; 4] = ["disable", "prefer", "require", "verify-full"];

#[derive(Debug, PartialEq, Eq)]
pub struct PasswordStorageTypeError;

impl FromStr for PasswordStorageType {
    type Err = PasswordStorageTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "keyring" => Self::Keyring,
            _ => Self::DontSave,
        })
    }
}

#[derive(Default)]
pub struct Modal {
    pub fields: Vec<ModalField>,
    pub current_field: usize,
    pub is_open: bool,
    pub selected_button: usize,
    pub mode: Mode,
    pub test_result: TestResult,
    pub original_name: Option<String>,
    pub password_storage: PasswordStorageType,
    /// When `Some(field_index)`, that dropdown field's menu is open (tui-menu).
    pub dropdown_open: Option<usize>,
    /// tui-menu state when a dropdown is open (not Debug).
    pub menu_state: Option<MenuState<&'static str>>,
    /// Current step: `ChooseType` (step 1) or `ConnectionForm` (step 2).
    pub step: ConnectionModalStep,
    /// Step 1: selected index in database type list (0 = postgres, 1 = sqlite).
    pub step1_type_index: usize,
    /// Step 1: optional "Import from URL" input; if non-empty, step 2 is prefilled.
    pub step1_import_url: TextArea<'static>,
    /// Step 1: focus on URL input (true) or on type list (false).
    pub step1_focus_on_url: bool,
    /// Step 2: connection type (set when entering step 2).
    pub connection_type: Option<ConnectionType>,
}

impl std::fmt::Debug for Modal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Modal")
            .field("fields", &self.fields)
            .field("current_field", &self.current_field)
            .field("is_open", &self.is_open)
            .field("selected_button", &self.selected_button)
            .field("mode", &self.mode)
            .field("test_result", &self.test_result)
            .field("original_name", &self.original_name)
            .field("password_storage", &self.password_storage)
            .field("dropdown_open", &self.dropdown_open)
            .field("menu_state", &self.menu_state.is_some())
            .field("step", &self.step)
            .field("step1_type_index", &self.step1_type_index)
            .field("step1_import_url", &self.step1_import_url)
            .field("step1_focus_on_url", &self.step1_focus_on_url)
            .field("connection_type", &self.connection_type)
            .finish()
    }
}

/// Result of confirming an edit in the cell value modal (persisted to the DB, then UI).
#[derive(Debug, Clone)]
pub struct CellValueApply {
    pub schema_name: String,
    pub table_name: String,
    pub set_column: String,
    pub row_snapshot: Vec<String>,
    pub row_index: usize,
    pub col_index: usize,
    pub new_value: String,
    pub primary_key: Vec<(String, String)>,
    pub db_row_id: Option<DbRowId>,
}

#[derive(Debug, Clone)]
pub struct CellValueModal {
    pub is_open: bool,
    pub column_name: String,
    schema_name: String,
    table_name: String,
    input: TextArea<'static>,
    /// When true, keyboard input goes to the textarea; when false, OK/Cancel row is focused.
    focus_editor: bool,
    /// 0 = OK, 1 = Cancel (only when `!focus_editor`)
    selected_button: usize,
    row_snapshot: Vec<String>,
    row_index: usize,
    col_index: usize,
    primary_key: Vec<(String, String)>,
    db_row_id: Option<DbRowId>,
}

#[derive(Default, Debug, Clone)]
pub struct SqlQuerySelectionModal {
    pub is_open: bool,
    pub selected_index: usize,
    pub statements: Vec<String>,
    submitted: bool,
}

impl Modal {
    #[must_use]
    pub fn new(_connection: Connection, mode: Mode) -> Self {
        Self {
            fields: Vec::new(),
            current_field: 0,
            is_open: false,
            selected_button: 0,
            mode,
            test_result: TestResult::NotTested,
            original_name: None,
            password_storage: PasswordStorageType::default(),
            dropdown_open: None,
            menu_state: None,
            step: ConnectionModalStep::default(),
            step1_type_index: 0,
            step1_import_url: Self::new_url_textarea(),
            step1_focus_on_url: false,
            connection_type: None,
        }
    }

    fn new_url_textarea() -> TextArea<'static> {
        let mut ta = TextArea::default();
        ta.set_cursor_line_style(Style::default());
        ta.set_cursor_style(Style::default());
        ta.set_style(Style::default().fg(Color::White));
        ta.set_max_histories(0);
        ta
    }

    fn set_url_focus(&mut self, focused: bool) {
        self.step1_focus_on_url = focused;
        if focused {
            self.step1_import_url.set_style(theme::focus_field());
            self.step1_import_url
                .set_cursor_style(theme::focus_cursor());
        } else {
            self.step1_import_url
                .set_style(Style::default().fg(Color::White));
            self.step1_import_url.set_cursor_style(Style::default());
        }
    }

    /// Build step 2 fields from step 1 state. If `step1_import_url` is set, parse and prefill; else use selected type with empty/default values.
    fn enter_step2(&mut self) {
        let url_text = self
            .step1_import_url
            .lines()
            .first()
            .cloned()
            .unwrap_or_default();
        let (connection_type, prefilled) = if url_text.trim().is_empty() {
            (step1_connection_type(self.step1_type_index), None)
        } else if let Some(parsed) = parse_connection_string(&url_text) {
            (parsed.connection_type, Some(parsed.url))
        } else {
            (step1_connection_type(self.step1_type_index), None)
        };

        self.connection_type = Some(connection_type);
        self.step = ConnectionModalStep::ConnectionForm;
        self.fields.clear();
        self.current_field = 0;
        self.dropdown_open = None;
        self.menu_state = None;

        match connection_type {
            ConnectionType::Postgres | ConnectionType::Mysql => {
                let (host, port, user, database) = parse_server_url(
                    connection_type,
                    prefilled.as_deref().unwrap_or_default(),
                );
                let name = ModalField::new("Name");
                let mut host_f = ModalField::new("Host");
                host_f.set_value(host);
                let mut port_f = ModalField::new("Port");
                port_f.set_value(port);
                let mut user_f = ModalField::new("User");
                user_f.set_value(user);
                let mut database_f = ModalField::new("Database");
                database_f.set_value(database);
                let tls = TlsConfig::from_url(
                    prefilled.as_deref().unwrap_or_default(),
                );
                let mut tls_f = ModalField::new("TLS");
                tls_f.set_options(TLS_MODES.to_vec());
                tls_f.set_value(tls.mode.label());
                let mut ca_f = ModalField::new("CA Cert");
                ca_f.set_value(tls.ca_path.unwrap_or_default());
                let mut env = ModalField::new("Environment");
                env.set_options(vec!["dev", "staging", "prod"]);
                env.set_value("dev");
                let mut metadata = ModalField::new("Metadata");
                metadata.set_value("{}");
                let mut password = ModalField::new("Password");
                password.set_masked();
                self.fields = vec![
                    name, host_f, port_f, user_f, database_f, tls_f, ca_f, env,
                    metadata, password,
                ];
            }
            ConnectionType::Sqlite => {
                let path = prefilled.unwrap_or_default();
                let name = ModalField::new("Name");
                let mut path_f = ModalField::new("Path");
                path_f.set_value(path);
                let mut env = ModalField::new("Environment");
                env.set_options(vec!["dev", "staging", "prod"]);
                env.set_value("dev");
                let mut metadata = ModalField::new("Metadata");
                metadata.set_value("{}");
                self.fields = vec![name, path_f, env, metadata];
            }
        }

        for (i, f) in self.fields.iter_mut().enumerate() {
            f.set_focus(i == 0);
            f.clamp_to_options();
        }
    }

    /// Return to step 1 from step 2 (preserves step 1 state).
    fn back_to_step1(&mut self) {
        self.step = ConnectionModalStep::ChooseType;
        self.connection_type = None;
        self.fields.clear();
        self.current_field = 0;
        self.dropdown_open = None;
        self.menu_state = None;
    }

    pub const fn toggle_password_storage(&mut self) {
        self.password_storage = match self.password_storage {
            PasswordStorageType::Keyring => PasswordStorageType::DontSave,
            PasswordStorageType::DontSave => PasswordStorageType::Keyring,
        };
    }

    pub fn open(&mut self) {
        self.is_open = true;
        self.step = ConnectionModalStep::ChooseType;
        self.step1_type_index = 0;
        self.step1_import_url = Self::new_url_textarea();
        self.step1_focus_on_url = false; // already false from new_url_textarea default
        self.connection_type = None;
        self.fields.clear();
        self.current_field = 0;
        self.dropdown_open = None;
        self.menu_state = None;
    }

    pub fn open_for_edit(&mut self, connection: &Connection) {
        self.is_open = true;
        self.step = ConnectionModalStep::ConnectionForm;
        self.connection_type = Some(connection.r#type);
        self.mode = Mode::Edit;
        self.original_name = Some(connection.name.clone());
        self.fields.clear();
        self.current_field = 0;
        self.dropdown_open = None;
        self.menu_state = None;

        self.password_storage = connection
            .password_storage
            .as_ref()
            .map(|s| PasswordStorageType::from_str(s).unwrap_or_default())
            .unwrap_or_default();

        match connection.r#type {
            ConnectionType::Postgres | ConnectionType::Mysql => {
                let (host, port, user, database) =
                    parse_server_url(connection.r#type, &connection.url);
                let mut name = ModalField::new("Name");
                name.set_value(connection.name.clone());
                let mut host_f = ModalField::new("Host");
                host_f.set_value(host);
                let mut port_f = ModalField::new("Port");
                port_f.set_value(port);
                let mut user_f = ModalField::new("User");
                user_f.set_value(user);
                let mut database_f = ModalField::new("Database");
                database_f.set_value(database);
                let tls = TlsConfig::from_url(&connection.url);
                let mut tls_f = ModalField::new("TLS");
                tls_f.set_options(TLS_MODES.to_vec());
                tls_f.set_value(tls.mode.label());
                let mut ca_f = ModalField::new("CA Cert");
                ca_f.set_value(tls.ca_path.unwrap_or_default());
                let mut env = ModalField::new("Environment");
                env.set_options(vec!["dev", "staging", "prod"]);
                env.set_value(connection.environment.to_string());
                let mut metadata = ModalField::new("Metadata");
                metadata.set_value(connection.metadata.to_string());
                let mut password = ModalField::new("Password");
                password.set_masked();
                password
                    .set_value(connection.password.clone().unwrap_or_default());
                self.fields = vec![
                    name, host_f, port_f, user_f, database_f, tls_f, ca_f, env,
                    metadata, password,
                ];
            }
            ConnectionType::Sqlite => {
                let mut name = ModalField::new("Name");
                name.set_value(connection.name.clone());
                let mut path_f = ModalField::new("Path");
                path_f.set_value(connection.url.clone());
                let mut env = ModalField::new("Environment");
                env.set_options(vec!["dev", "staging", "prod"]);
                env.set_value(connection.environment.to_string());
                let mut metadata = ModalField::new("Metadata");
                metadata.set_value(connection.metadata.to_string());
                self.fields = vec![name, path_f, env, metadata];
            }
        }

        for (i, f) in self.fields.iter_mut().enumerate() {
            f.set_focus(i == 0);
            f.clamp_to_options();
        }
    }

    pub const fn close(&mut self) {
        self.is_open = false;
    }

    /// Build and open the tui-menu dropdown for the current field if it is a dropdown field.
    fn open_dropdown_if_focused(&mut self) {
        let Some(idx) = self.focused_field_index() else {
            return;
        };
        let Some(field) = self.fields.get(idx) else {
            return;
        };
        let Some(ref opts) = field.options else {
            return;
        };
        if opts.is_empty() {
            return;
        }
        // Order options with current value first so tui-menu highlights it after push().
        let current = field.value();
        let mut children: Vec<MenuItem<&'static str>> =
            opts.iter().map(|&o| MenuItem::item(o, o)).collect();
        if let Some(pos) = children.iter().position(|m| m.data == Some(current))
            && pos != 0
        {
            let item = children.remove(pos);
            children.insert(0, item);
        }
        let mut state = MenuState::new(vec![MenuItem::group("", children)]);
        state.activate();
        let _ = state.push();
        self.menu_state = Some(state);
        self.dropdown_open = Some(idx);
    }

    /// Close dropdown; if apply is true, set field value from selected menu item.
    fn close_dropdown(&mut self, apply: bool) {
        let field_idx = self.dropdown_open.take();
        let mut state = self.menu_state.take();
        if let (Some(idx), Some(ref mut s)) = (field_idx, state.as_mut()) {
            if apply && let Some(event) = s.drain_events().next() {
                let MenuEvent::Selected(value) = event;
                if let Some(field) = self.fields.get_mut(idx) {
                    field.set_value(value);
                }
            }
            s.reset();
        }
    }

    /// Number of buttons shown: New has Back, OK, Test, Cancel (4); Edit has OK, Test, Cancel (3).
    const fn button_count(&self) -> usize {
        match self.mode {
            Mode::Edit => 3,
            Mode::New => 4,
        }
    }

    /// Get total number of navigable items (fields + storage when visible + buttons).
    fn total_items(&self) -> usize {
        if self.step != ConnectionModalStep::ConnectionForm {
            return 0;
        }
        let storage = usize::from(self.is_password_storage_row_visible());
        self.visible_fields_count() + storage + self.button_count()
    }

    /// Check if `current_field` is on a button. Returns the button index within the current button list.
    fn is_on_button(&self) -> Option<usize> {
        if self.step != ConnectionModalStep::ConnectionForm {
            return None;
        }
        let storage = usize::from(self.is_password_storage_row_visible());
        let button_start = self.visible_fields_count() + storage;
        let count = self.button_count();
        if self.current_field >= button_start
            && self.current_field < button_start + count
        {
            Some(self.current_field - button_start)
        } else {
            None
        }
    }

    pub fn next_field(&mut self) {
        let total = self.total_items();
        if self.current_field < total - 1 {
            if let Some(i) = self.focused_field_index() {
                self.fields[i].set_focus(false);
            }
            self.current_field += 1;
            if let Some(i) = self.focused_field_index() {
                self.fields[i].set_focus(true);
            }
        }
    }

    pub fn prev_field(&mut self) {
        if self.current_field > 0 {
            if let Some(i) = self.focused_field_index() {
                self.fields[i].set_focus(false);
            }
            self.current_field -= 1;
            if let Some(i) = self.focused_field_index() {
                self.fields[i].set_focus(true);
            }
        }
    }

    fn forward_key_to_field(&mut self, key: KeyEvent) {
        if let Some(i) = self.focused_field_index() {
            self.fields[i].input_key(key);
        }
    }

    #[must_use]
    pub fn get_connection(&self) -> Option<Connection> {
        let connection_type = self.connection_type?;

        if !self.is_valid() {
            return None;
        }

        let password = if self.is_password_field_hidden() {
            None
        } else {
            Some(self.fields[self.password_field_index()].value().to_string())
        };

        let (environment_index, metadata_index) = match connection_type {
            ConnectionType::Postgres | ConnectionType::Mysql => (7, 8),
            ConnectionType::Sqlite => (2, 3),
        };
        let environment = self
            .fields
            .get(environment_index)
            .map(|f| f.value().parse().unwrap_or_default())
            .unwrap_or_default();
        let metadata_str = self
            .fields
            .get(metadata_index)
            .map_or("", |f| f.value().trim());
        let metadata_str = metadata_str.to_string();
        let metadata = if metadata_str.is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(&metadata_str).unwrap_or_else(|_| {
                serde_json::Value::Object(serde_json::Map::new())
            })
        };

        let (password, password_storage) = if self.is_sqlite() {
            (None, None)
        } else {
            (password, Some(self.password_storage.to_string()))
        };

        let url = match connection_type {
            ConnectionType::Postgres | ConnectionType::Mysql => {
                let host = self.fields.get(1).map_or_else(
                    || "localhost".to_string(),
                    |f| f.value().to_string(),
                );
                let port = self.fields.get(2).map_or_else(
                    || default_port(connection_type).to_string(),
                    |f| f.value().to_string(),
                );
                let user = self
                    .fields
                    .get(3)
                    .map_or(String::new(), |f| f.value().to_string());
                let database = self.fields.get(DATABASE_FIELD).map_or_else(
                    || default_database(connection_type).to_string(),
                    |f| f.value().to_string(),
                );
                let url = build_server_url(
                    connection_type,
                    &host,
                    &port,
                    &user,
                    &database,
                );
                let mode = self
                    .fields
                    .get(TLS_FIELD)
                    .map_or("disable", |f| f.value().trim());
                let ca = if self.is_ca_field_hidden() {
                    ""
                } else {
                    self.fields.get(CA_FIELD).map_or("", |f| f.value().trim())
                };
                with_tls_query(connection_type, &url, mode, ca)
            }
            ConnectionType::Sqlite => self
                .fields
                .get(1)
                .map(|f| f.value().to_string())
                .unwrap_or_default(),
        };

        Some(Connection {
            name: self.fields[0].value().to_string(),
            r#type: connection_type,
            url,
            environment,
            metadata,
            selected_database: None,
            schema: None,
            table: None,
            password: if self.is_sqlite() { None } else { password },
            password_storage,
            status: crate::db::connection::ConnectionStatus::default(),
        })
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        // Password field is optional when "ask every time" is selected;
        // the Database field is optional for MySQL.
        let end = if self.is_password_field_hidden() {
            self.password_field_index()
        } else {
            self.fields.len()
        };
        let is_server = self.connection_type != Some(ConnectionType::Sqlite);
        let optional_db = (self.connection_type == Some(ConnectionType::Mysql))
            .then_some(DATABASE_FIELD);
        let optional_ca = is_server.then_some(CA_FIELD);
        !self.fields.iter().take(end).enumerate().any(|(i, f)| {
            Some(i) != optional_db
                && Some(i) != optional_ca
                && f.value().trim().is_empty()
        })
    }

    /// Handle key events for UI navigation only
    /// Returns an enum indicating what action was triggered
    #[allow(clippy::too_many_lines)]
    pub fn handle_key_events_ui(&mut self, key: KeyEvent) -> ModalAction {
        if self.step == ConnectionModalStep::ChooseType {
            return self.handle_step1_key(key);
        }

        // Step 2: when a tui-menu dropdown is open, forward to the menu
        if self.menu_state.is_some() {
            return self.handle_menu_key(key);
        }

        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => {
                self.close();
                ModalAction::Cancel
            }
            (_, KeyCode::BackTab | KeyCode::Up) => {
                if self.is_on_button().is_some() {
                    let storage =
                        usize::from(self.is_password_storage_row_visible());
                    self.current_field = self
                        .visible_fields_count()
                        .saturating_add(storage)
                        .saturating_sub(1);
                } else {
                    self.prev_field();
                }
                ModalAction::None
            }
            (_, KeyCode::Tab | KeyCode::Down) => {
                self.next_field();
                ModalAction::None
            }
            (_, KeyCode::Enter) => {
                if let Some(button_idx) = self.is_on_button() {
                    // New: [Back=0, OK=1, Test=2, Cancel=3]
                    // Edit: [OK=0, Test=1, Cancel=2]
                    let (is_back, is_ok, is_test, is_cancel) = match self.mode {
                        Mode::New => (
                            button_idx == 0,
                            button_idx == 1,
                            button_idx == 2,
                            button_idx == 3,
                        ),
                        Mode::Edit => (
                            false,
                            button_idx == 0,
                            button_idx == 1,
                            button_idx == 2,
                        ),
                    };
                    if is_back {
                        self.back_to_step1();
                        ModalAction::None
                    } else if is_ok && self.is_valid() {
                        ModalAction::Save
                    } else if is_test {
                        ModalAction::Test
                    } else if is_cancel {
                        self.close();
                        ModalAction::Cancel
                    } else {
                        ModalAction::None
                    }
                } else if let Some(idx) = self.focused_field_index() {
                    if self.fields.get(idx).is_some_and(ModalField::is_dropdown)
                    {
                        self.open_dropdown_if_focused();
                        ModalAction::None
                    } else if self.is_valid() {
                        ModalAction::Save
                    } else {
                        ModalAction::None
                    }
                } else if self.is_valid() {
                    ModalAction::Save
                } else {
                    ModalAction::None
                }
            }
            (_, KeyCode::Char(c)) => {
                if self.is_password_storage_row_visible()
                    && self.current_field == self.visible_fields_count()
                    && c == ' '
                {
                    self.toggle_password_storage();
                    return ModalAction::None;
                }
                self.forward_key_to_field(key);
                ModalAction::None
            }
            (
                _,
                KeyCode::Backspace
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Delete,
            ) => {
                if self.current_field < self.visible_fields_count() {
                    self.forward_key_to_field(key);
                } else if let Some(button_idx) = self.is_on_button() {
                    let storage =
                        usize::from(self.is_password_storage_row_visible());
                    let button_start = self.visible_fields_count() + storage;
                    let count = self.button_count();
                    if key.code == KeyCode::Left {
                        let new_button_idx = (button_idx + count - 1) % count;
                        self.current_field = button_start + new_button_idx;
                    } else if key.code == KeyCode::Right {
                        let new_button_idx = (button_idx + 1) % count;
                        self.current_field = button_start + new_button_idx;
                    }
                }

                ModalAction::None
            }
            _ => ModalAction::None,
        }
    }

    /// Step 1: type list (Up/Down) and Import from URL input; Enter = go to step 2.
    fn handle_step1_key(&mut self, key: KeyEvent) -> ModalAction {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => {
                self.close();
                ModalAction::Cancel
            }
            (_, KeyCode::BackTab | KeyCode::Up) => {
                if self.step1_focus_on_url {
                    self.set_url_focus(false);
                } else {
                    self.step1_type_index = self
                        .step1_type_index
                        .saturating_sub(1)
                        .min(STEP1_DB_TYPES.len().saturating_sub(1));
                }
                ModalAction::None
            }
            (_, KeyCode::Tab | KeyCode::Down) => {
                if self.step1_focus_on_url {
                    // Already on URL input; Down does nothing
                } else if self.step1_type_index + 1 < STEP1_DB_TYPES.len() {
                    self.step1_type_index += 1;
                } else {
                    self.set_url_focus(true);
                }
                ModalAction::None
            }
            (_, KeyCode::Enter) => {
                self.enter_step2();
                ModalAction::None
            }
            _ if self.step1_focus_on_url => {
                self.step1_import_url.input(key);
                ModalAction::None
            }
            _ => ModalAction::None,
        }
    }

    /// Handle keys when tui-menu dropdown is open: Escape to close; Up/Down/j/k to navigate; Enter to select.
    fn handle_menu_key(&mut self, key: KeyEvent) -> ModalAction {
        let Some(ref mut state) = self.menu_state else {
            return ModalAction::None;
        };
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => {
                self.close_dropdown(false);
                ModalAction::None
            }
            (_, KeyCode::Up | KeyCode::Char('k' | 'K')) => {
                state.up();
                ModalAction::None
            }
            (_, KeyCode::Down | KeyCode::Char('j' | 'J')) => {
                state.down();
                ModalAction::None
            }
            (_, KeyCode::Enter) => {
                state.select();
                if let Some(event) = state.drain_events().next() {
                    let MenuEvent::Selected(value) = event;
                    if let Some(field_idx) = self.dropdown_open
                        && let Some(field) = self.fields.get_mut(field_idx)
                    {
                        field.set_value(value);
                    }
                }
                self.close_dropdown(false);
                ModalAction::None
            }
            _ => ModalAction::None,
        }
    }

    /// Insert pasted text into the focused text field (step-1 URL, form field, etc.).
    pub fn handle_paste(&mut self, text: &str) {
        if self.step == ConnectionModalStep::ChooseType {
            if self.step1_focus_on_url {
                self.step1_import_url.insert_str(text);
            }
            return;
        }
        if self.menu_state.is_some() {
            return;
        }
        if let Some(i) = self.focused_field_index()
            && let Some(field) = self.fields.get_mut(i)
            && field.options.is_none()
        {
            field.input.insert_str(text);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalAction {
    None,
    Save,
    Test,
    Cancel,
}

/// Stateful widget for the connection modal (required because modal holds tui-menu state that is not Clone).
pub struct ConnectionModalWidget;

impl StatefulWidget for ConnectionModalWidget {
    type State = Modal;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        state.render_into(area, buf);
    }
}

impl Modal {
    /// Render the connection modal into the buffer (used by `ConnectionModalWidget`).
    pub fn render_into(&self, area: Rect, buf: &mut Buffer) {
        if !self.is_open {
            return;
        }

        let (modal_width, field_height, modal_height) =
            if self.step == ConnectionModalStep::ChooseType {
                let h = 7; // section labels + list + separator + URL label + URL input
                (STEP1_MODAL_WIDTH, h, 1 + h + 1 + 2)
            } else {
                let fh = self.fields_section_height();
                (CONNECTION_MODAL_WIDTH, fh, 1 + fh + 1 + 1 + 2)
            };

        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area =
            Rect::new(x, y, modal_width, modal_height.min(area.height));

        let title = match (self.mode, self.step) {
            (Mode::Edit, _) => "Edit Connection".to_string(),
            (
                Mode::New,
                ConnectionModalStep::ChooseType
                | ConnectionModalStep::ConnectionForm,
            ) => "New Connection".to_string(),
        };

        let block = Block::default()
            .title(title)
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(theme::modal_connection_border())
            .style(Style::default().bg(Color::Black));
        Clear.render(modal_area, buf);
        block.render(modal_area, buf);

        let inner_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(field_height),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .margin(1)
            .split(modal_area);

        if self.step == ConnectionModalStep::ChooseType {
            self.render_step1(inner_layout[1], buf);
            Paragraph::new("Enter to continue")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center)
                .render(inner_layout[2], buf);
        } else {
            self.render_test_result(inner_layout[2], buf);
            self.render_buttons(inner_layout[3], buf);
            self.render_fields(inner_layout[1], buf);
        }
    }

    /// Render step 1: two separate options — (1) select database type and continue, or (2) import from URL.
    fn render_step1(&self, area: Rect, buf: &mut Buffer) {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

        // Section 1: Select database type (list only — Enter advances with this type)
        let type_label_row = layout[0];
        Paragraph::new("Select database type:")
            .style(Style::default().fg(Color::Cyan))
            .alignment(Alignment::Left)
            .render(type_label_row, buf);

        for (i, &db_type) in STEP1_DB_TYPES.iter().enumerate() {
            let row = layout[1 + i];
            let is_selected =
                !self.step1_focus_on_url && self.step1_type_index == i;
            let style = if is_selected {
                Style::default().fg(Color::Yellow).bg(Color::DarkGray)
            } else {
                Style::default().fg(Color::White)
            };
            let prefix = if is_selected { "> " } else { "  " };
            Paragraph::new(format!("{prefix}{db_type}"))
                .style(style)
                .alignment(Alignment::Left)
                .render(row, buf);
        }

        // Separator so list and URL box are clearly separate
        let sep_row = layout[1 + STEP1_DB_TYPES.len()];
        Paragraph::new("────────────────────────")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Left)
            .render(sep_row, buf);

        // Section 2: Or import from URL (optional — paste URL to prefill next step)
        let url_label_row = layout[2 + STEP1_DB_TYPES.len()];
        Paragraph::new("Or import from URL:")
            .style(Style::default().fg(Color::Cyan))
            .alignment(Alignment::Left)
            .render(url_label_row, buf);

        Widget::render(
            &self.step1_import_url,
            layout[3 + STEP1_DB_TYPES.len()],
            buf,
        );
    }

    /// Height for fields section; dropdown is drawn as overlay and does not expand layout.
    fn fields_section_height(&self) -> u16 {
        let rows = u16::try_from(self.visible_fields_count())
            .unwrap_or(u16::MAX)
            + u16::from(self.is_password_storage_row_visible());
        rows + 2 // rows + padding (no fixed cap — TLS/CA fields need the space)
    }
}

impl Modal {
    /// Get the index of the password field (last field for Postgres; unused for `SQLite`).
    const fn password_field_index(&self) -> usize {
        self.fields.len().saturating_sub(1)
    }

    /// True when connection type is `SQLite` (password not supported).
    fn is_sqlite(&self) -> bool {
        self.connection_type == Some(ConnectionType::Sqlite)
    }

    /// Check if password field should be hidden (Ask every time, or `SQLite` which has no passwords).
    fn is_password_field_hidden(&self) -> bool {
        self.password_storage == PasswordStorageType::DontSave
            || self.is_sqlite()
    }

    /// CA Cert is only meaningful when TLS is not `disable`.
    fn is_ca_field_hidden(&self) -> bool {
        match self.connection_type {
            Some(ConnectionType::Postgres | ConnectionType::Mysql) => self
                .fields
                .get(TLS_FIELD)
                .is_none_or(|f| f.value() == "disable"),
            _ => true,
        }
    }

    fn is_field_hidden(&self, index: usize) -> bool {
        (index == CA_FIELD && self.is_ca_field_hidden())
            || (index == self.password_field_index()
                && self.is_password_field_hidden())
    }

    /// Nav index → `fields` index, skipping hidden rows.
    fn visible_to_field(&self, visible: usize) -> Option<usize> {
        self.fields
            .iter()
            .enumerate()
            .filter(|&(i, _)| !self.is_field_hidden(i))
            .nth(visible)
            .map(|(i, _)| i)
    }

    /// `current_field` when it points at a form field (not storage/buttons).
    fn focused_field_index(&self) -> Option<usize> {
        if self.current_field < self.visible_fields_count() {
            self.visible_to_field(self.current_field)
        } else {
            None
        }
    }

    /// Show password storage row only for Postgres (`SQLite` has no passwords).
    fn is_password_storage_row_visible(&self) -> bool {
        !self.is_sqlite()
    }

    /// Visible form fields (password / CA Cert may be omitted).
    fn visible_fields_count(&self) -> usize {
        (0..self.fields.len())
            .filter(|&i| !self.is_field_hidden(i))
            .count()
    }

    fn render_fields(&self, area: Rect, buf: &mut Buffer) {
        // Fixed one row per field (+ storage row only for Postgres); dropdown list is overlay.
        let storage_row = usize::from(self.is_password_storage_row_visible());
        let num_rows = self.visible_fields_count() + storage_row;
        let field_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints((0..num_rows).map(|_| Constraint::Length(1)))
            .split(area);

        let highlighted_value = self
            .menu_state
            .as_ref()
            .and_then(|s| s.highlight())
            .and_then(|m| m.data);

        let mut overlay: Option<(Rect, &[&'static str], usize)> = None;
        let mut visible_index = 0;

        for (i, field) in self.fields.iter().enumerate() {
            if self.is_field_hidden(i) {
                continue;
            }

            let row_area = field_layout[visible_index];
            let is_dropdown_open =
                self.dropdown_open == Some(i) && field.is_dropdown();

            if is_dropdown_open {
                let opts = field.options.as_deref().unwrap_or(&[]);
                let hi = highlighted_value
                    .and_then(|v| opts.iter().position(|o| *o == v))
                    .unwrap_or(0)
                    .min(opts.len().saturating_sub(1));
                Self::render_trigger_row(row_area, buf, field, true);
                if !opts.is_empty() {
                    overlay = Some((row_area, opts, hi));
                }
            } else if field.is_dropdown() {
                // Dropdown: render value with arrow as Paragraph
                let label = format!("{}:", field.label);
                let value_str = field.value();
                let value = if value_str.is_empty() {
                    " ".repeat(18)
                } else {
                    format!("{value_str} ▼")
                };
                let text = format!("{label:<12} {value}");
                let style = if field.is_focused {
                    Style::default().fg(Color::Yellow).bg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                };
                Paragraph::new(text)
                    .style(style)
                    .alignment(Alignment::Left)
                    .render(row_area, buf);
            } else {
                // Free-text: split row into label + textarea
                let label = format!("{:<12} ", format!("{}:", field.label));
                let label_width = u16::try_from(label.len()).unwrap_or(14);
                let chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(label_width),
                        Constraint::Min(1),
                    ])
                    .split(row_area);
                let label_style = if field.is_focused {
                    Style::default().fg(Color::Yellow).bg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                };
                Paragraph::new(label)
                    .style(label_style)
                    .render(chunks[0], buf);
                // Render via reference to avoid cloning the textarea
                Widget::render(&field.input, chunks[1], buf);
            }
            visible_index += 1;
        }

        if self.is_password_storage_row_visible() {
            let checkbox_text = match self.password_storage {
                PasswordStorageType::Keyring => "[ ] Ask every time",
                PasswordStorageType::DontSave => "[x] Ask every time",
            };
            let storage_style =
                if self.current_field == self.visible_fields_count() {
                    Style::default().fg(Color::Yellow).bg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::Cyan)
                };
            Paragraph::new(checkbox_text)
                .style(storage_style)
                .alignment(Alignment::Left)
                .render(field_layout[self.visible_fields_count()], buf);
        }

        // Draw dropdown list as overlay so it hovers over content below without shifting layout.
        if let Some((trigger_rect, options, highlighted_index)) = overlay {
            Self::render_dropdown_overlay(
                buf,
                trigger_rect,
                options,
                highlighted_index,
            );
        }
    }

    fn render_trigger_row(
        area: Rect,
        buf: &mut Buffer,
        field: &ModalField,
        is_open: bool,
    ) {
        let trigger_style = if field.is_focused {
            Style::default().fg(Color::Yellow).bg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };
        let arrow = if is_open { " ▲" } else { " ▼" };
        let v = field.value();
        let value_display = if v.is_empty() {
            " ".to_string()
        } else {
            format!("{v}{arrow}")
        };
        let label = format!("{}:", field.label);
        let trigger_text = format!("{label:<12} {value_display}");
        Paragraph::new(trigger_text)
            .style(trigger_style)
            .alignment(Alignment::Left)
            .render(area, buf);
    }

    /// Render dropdown list as a floating overlay below the trigger row (does not affect layout).
    fn render_dropdown_overlay(
        buf: &mut Buffer,
        trigger_rect: Rect,
        options: &[&'static str],
        highlighted_index: usize,
    ) {
        if options.is_empty() {
            return;
        }
        // Height: top border + one row per option + bottom border
        let list_height = 2 + u16::try_from(options.len()).unwrap_or(u16::MAX);
        let list_width = trigger_rect.width.max(
            options
                .iter()
                .map(|o| u16::try_from(o.len()).unwrap_or(0) + 2)
                .max()
                .unwrap_or(10),
        );

        let overlay_rect = Rect {
            x: trigger_rect.x,
            y: trigger_rect.y + 1,
            width: list_width,
            height: list_height,
        };

        Clear.render(overlay_rect, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::modal_default_border())
            .style(Style::default().bg(Color::Black));
        let inner = block.inner(overlay_rect);
        block.render(overlay_rect, buf);

        for (i, opt) in options.iter().enumerate() {
            let row_area = Rect {
                x: inner.x,
                y: inner.y + u16::try_from(i).unwrap_or(0),
                width: inner.width,
                height: 1,
            };
            let is_highlighted = i == highlighted_index;
            let style = if is_highlighted {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White).bg(Color::Black)
            };
            let line = Line::from(vec![Span::styled(format!(" {opt}"), style)]);
            Paragraph::new(line).render(row_area, buf);
        }
    }

    fn render_buttons(&self, area: Rect, buf: &mut Buffer) {
        let selected_button = self.is_on_button().unwrap_or(999);
        let buttons = Buttons {
            buttons: match self.mode {
                Mode::New => vec!["Back", "OK", "Test", "Cancel"],
                Mode::Edit => vec!["OK", "Test", "Cancel"],
            },
            selected: selected_button,
        };
        buttons.render(area, buf);
    }

    fn render_test_result(&self, area: Rect, buf: &mut Buffer) {
        let (text, style) = match &self.test_result {
            TestResult::NotTested => ("", Style::default()),
            TestResult::Testing => {
                ("Testing connection...", Style::default().fg(Color::Yellow))
            }
            TestResult::Success => {
                ("✓ Connection successful", theme::success())
            }
            TestResult::Failed(msg) => {
                (msg.as_str(), Style::default().fg(Color::Red))
            }
        };

        Paragraph::new(text)
            .style(style)
            .alignment(Alignment::Center)
            .render(area, buf);
    }
}

impl SqlQuerySelectionModal {
    #[must_use]
    pub const fn new(statements: Vec<String>) -> Self {
        Self {
            is_open: true,
            selected_index: 0,
            statements,
            submitted: false,
        }
    }

    pub const fn close(&mut self) {
        self.is_open = false;
    }

    pub const fn next(&mut self) {
        if self.statements.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.statements.len();
    }

    pub const fn prev(&mut self) {
        if self.statements.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + self.statements.len() - 1)
            % self.statements.len();
    }

    #[must_use]
    pub fn selected_statement(&self) -> Option<String> {
        self.statements.get(self.selected_index).cloned()
    }

    pub const fn submitted(&self) -> bool {
        self.submitted
    }

    pub const fn handle_key_events(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => {
                self.submitted = false;
                self.close();
            }
            (_, KeyCode::Up | KeyCode::Char('k' | 'K')) => {
                self.prev();
            }
            (_, KeyCode::Down | KeyCode::Char('j' | 'J')) => {
                self.next();
            }
            (_, KeyCode::Enter) => {
                self.submitted = true;
                self.close();
            }
            _ => {}
        }
    }
}

impl Widget for SqlQuerySelectionModal {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.is_open {
            return;
        }

        let width = 76u16.min(area.width.saturating_sub(2));
        let height = 12u16.min(area.height.saturating_sub(2));
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        let modal_area = Rect::new(x, y, width, height);

        let block = Block::default()
            .title("Select SQL Statement")
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(theme::modal_default_border())
            .style(Style::default().bg(Color::Black));
        Clear.render(modal_area, buf);
        let inner = block.inner(modal_area);
        block.render(modal_area, buf);
        let max_rows = inner.height.saturating_sub(1) as usize;
        let start = self
            .selected_index
            .saturating_sub(max_rows.saturating_sub(1));
        let end = (start + max_rows).min(self.statements.len());

        for (row, (idx, stmt)) in self.statements[start..end]
            .iter()
            .enumerate()
            .map(|(row, stmt)| (row, (start + row, stmt)))
        {
            let is_selected = idx == self.selected_index;
            let style = if is_selected {
                Style::default().fg(Color::Yellow).bg(Color::DarkGray)
            } else {
                Style::default().fg(Color::White)
            };
            let preview = stmt
                .lines()
                .next()
                .map(str::trim)
                .unwrap_or_default()
                .chars()
                .take((inner.width.saturating_sub(6)) as usize)
                .collect::<String>();
            let label = format!(
                "{} {}. {}",
                if is_selected { ">" } else { " " },
                idx + 1,
                preview
            );
            let row_area = Rect {
                x: inner.x,
                y: inner.y + u16::try_from(row).unwrap_or(0),
                width: inner.width,
                height: 1,
            };
            Paragraph::new(label).style(style).render(row_area, buf);
        }
    }
}

impl CellValueModal {
    fn new_cell_textarea(lines: Vec<String>) -> TextArea<'static> {
        let mut input = TextArea::new(lines);
        input.set_cursor_line_style(Style::default());
        input.set_cursor_style(Style::default());
        input.set_style(Style::default().fg(Color::White));
        input.set_max_histories(0);
        input
    }

    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        column_name: String,
        cell_value: &str,
        row_index: usize,
        col_index: usize,
        row_snapshot: Vec<String>,
        schema_name: String,
        table_name: String,
        primary_key: Vec<(String, String)>,
        db_row_id: Option<DbRowId>,
    ) -> Self {
        let lines: Vec<String> = if cell_value.is_empty() {
            vec![String::new()]
        } else {
            cell_value.lines().map(String::from).collect()
        };
        let mut s = Self {
            is_open: true,
            column_name,
            schema_name,
            table_name,
            input: Self::new_cell_textarea(lines),
            focus_editor: true,
            selected_button: 0,
            row_snapshot,
            row_index,
            col_index,
            primary_key,
            db_row_id,
        };
        s.set_editor_focused(true);
        s
    }

    pub const fn close(&mut self) {
        self.is_open = false;
    }

    fn set_editor_focused(&mut self, focused: bool) {
        self.focus_editor = focused;
        if focused {
            self.input.set_style(Style::default().fg(Color::White));
            self.input.set_cursor_style(theme::focus_cursor());
        } else {
            self.input.set_style(Style::default().fg(Color::DarkGray));
            self.input.set_cursor_style(Style::default());
        }
    }

    pub fn handle_key_events(&mut self, key: KeyEvent) -> ModalAction {
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => {
                self.close();
                ModalAction::Cancel
            }
            (_, KeyCode::Tab) => {
                if self.focus_editor {
                    self.selected_button = 0;
                    self.set_editor_focused(false);
                } else {
                    self.set_editor_focused(true);
                }
                ModalAction::None
            }
            (_, KeyCode::BackTab) => {
                self.set_editor_focused(true);
                ModalAction::None
            }
            (_, KeyCode::Left) => {
                if !self.focus_editor && self.selected_button > 0 {
                    self.selected_button = 0;
                } else if self.focus_editor {
                    self.input.input(key);
                }
                ModalAction::None
            }
            (_, KeyCode::Right) => {
                if !self.focus_editor && self.selected_button < 1 {
                    self.selected_button = 1;
                } else if self.focus_editor {
                    self.input.input(key);
                }
                ModalAction::None
            }
            (_, KeyCode::Enter) => {
                if self.focus_editor {
                    self.input.input(key);
                    ModalAction::None
                } else if self.selected_button == 0 {
                    self.close();
                    ModalAction::Save
                } else {
                    self.close();
                    ModalAction::Cancel
                }
            }
            _ if self.focus_editor => {
                self.input.input(key);
                ModalAction::None
            }
            _ => ModalAction::None,
        }
    }

    pub fn handle_paste(&mut self, text: &str) {
        if self.focus_editor {
            self.input.insert_str(text);
        }
    }

    /// Snapshot for persisting the edit after OK ([`ModalAction::Save`]).
    #[must_use]
    pub fn build_apply(&self) -> CellValueApply {
        CellValueApply {
            schema_name: self.schema_name.clone(),
            table_name: self.table_name.clone(),
            set_column: self.column_name.clone(),
            row_snapshot: self.row_snapshot.clone(),
            row_index: self.row_index,
            col_index: self.col_index,
            new_value: self.input.lines().join("\n"),
            primary_key: self.primary_key.clone(),
            db_row_id: self.db_row_id.clone(),
        }
    }
}

impl Widget for CellValueModal {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.is_open {
            return;
        }

        let max_width = 80u16;
        let name_w = self.column_name.chars().count();
        let content_max_line = self
            .input
            .lines()
            .iter()
            .map(|s| UnicodeWidthStr::width(s.as_str()))
            .max()
            .unwrap_or(0);
        let modal_width = u16::try_from(
            (name_w.max(content_max_line).saturating_add(4))
                .max(40)
                .min(max_width as usize),
        )
        .unwrap_or(max_width);

        let line_count = self.input.lines().len().max(1);
        let textarea_lines =
            u16::try_from(line_count.min(12)).unwrap_or(12).max(3);
        let modal_height =
            (2u16.saturating_add(textarea_lines).saturating_add(1))
                .min(area.height.saturating_sub(4))
                .max(8);

        let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
        let modal_area = Rect::new(x, y, modal_width, modal_height);

        let block = Block::default()
            .title(self.column_name.clone())
            .title_alignment(Alignment::Center)
            .borders(Borders::ALL)
            .border_style(theme::modal_default_border())
            .style(Style::default().bg(Color::Black));
        Clear.render(modal_area, buf);
        block.render(modal_area, buf);

        let inner_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(textarea_lines),
                Constraint::Length(1),
            ])
            .margin(1)
            .split(modal_area);

        let content_layout = *inner_layout.first().unwrap_or(&Rect::ZERO);
        Widget::render(&self.input, content_layout, buf);

        let buttons = Buttons {
            buttons: vec!["OK", "Cancel"],
            selected: if self.focus_editor {
                0
            } else {
                self.selected_button
            },
        };
        let button_layout = *inner_layout.get(1).unwrap_or(&Rect::ZERO);
        buttons.render(button_layout, buf);
    }
}

/// Manager for handling multiple modals in the application
#[derive(Default, Debug)]
pub struct ModalManager {
    connection_modal: Option<Modal>,
    confirmation_modal: Option<ConfirmDialog>,
    confirmation_connection: Option<Connection>,
    sql_execution_confirmation_modal: Option<ConfirmDialog>,
    sql_execution_statement: Option<String>,
    sql_query_selection_modal: Option<SqlQuerySelectionModal>,
    cell_value_modal: Option<CellValueModal>,
    cell_value_apply: Option<CellValueApply>,
    password_modal: Option<TextPromptModal>,
    password_connection: Option<Connection>,
    jump_to_row_modal: Option<TextPromptModal>,
    active_modal_type: Option<ModalType>,
}

impl ModalManager {
    /// Create a new modal manager
    #[must_use]
    pub const fn new() -> Self {
        Self {
            connection_modal: None,
            confirmation_modal: None,
            confirmation_connection: None,
            sql_execution_confirmation_modal: None,
            sql_execution_statement: None,
            sql_query_selection_modal: None,
            cell_value_modal: None,
            cell_value_apply: None,
            password_modal: None,
            password_connection: None,
            jump_to_row_modal: None,
            active_modal_type: None,
        }
    }

    /// Check if any modal is currently open
    #[must_use]
    pub fn is_any_modal_open(&self) -> bool {
        self.connection_modal.as_ref().is_some_and(|m| m.is_open)
            || self.confirmation_modal.as_ref().is_some_and(|m| m.is_open)
            || self
                .sql_execution_confirmation_modal
                .as_ref()
                .is_some_and(|m| m.is_open)
            || self
                .sql_query_selection_modal
                .as_ref()
                .is_some_and(|m| m.is_open)
            || self.cell_value_modal.as_ref().is_some_and(|m| m.is_open)
            || self.password_modal.as_ref().is_some_and(|m| m.is_open)
            || self.jump_to_row_modal.as_ref().is_some_and(|m| m.is_open)
    }

    /// Open a new connection modal
    pub fn open_new_connection_modal(&mut self) {
        let mut modal = Modal::new(Connection::default(), Mode::New);
        modal.open();
        self.connection_modal = Some(modal);
        self.active_modal_type = Some(ModalType::Connection);
    }

    /// Open an edit connection modal
    pub fn open_edit_connection_modal(
        &mut self,
        connection: &Connection,
        password: String,
    ) {
        let mut connection_with_password = connection.clone();
        connection_with_password.password = Some(password);

        let mut modal =
            Modal::new(connection_with_password.clone(), Mode::Edit);
        modal.open_for_edit(&connection_with_password);
        self.connection_modal = Some(modal);
        self.active_modal_type = Some(ModalType::Connection);
    }

    /// Open a confirmation modal
    pub fn open_confirmation_modal(
        &mut self,
        message: String,
        connection: Connection,
    ) {
        let modal = ConfirmDialog::new(
            "Confirm Delete",
            message,
            theme::modal_danger_border(),
            0,
        );
        self.confirmation_modal = Some(modal);
        self.confirmation_connection = Some(connection);
        self.active_modal_type = Some(ModalType::Confirmation);
    }

    pub fn open_sql_execution_confirmation_modal(&mut self, statement: String) {
        let preview = statement
            .lines()
            .take(3)
            .collect::<Vec<_>>()
            .join("\n")
            .chars()
            .take(180)
            .collect::<String>();
        let message = format!(
            "This statement may modify data.\n\nExecute anyway?\n\n{preview}"
        );
        let modal = ConfirmDialog::new(
            "Confirm SQL Execution",
            message,
            theme::modal_confirm_border(),
            1,
        );
        self.sql_execution_confirmation_modal = Some(modal);
        self.sql_execution_statement = Some(statement);
        self.active_modal_type = Some(ModalType::SqlExecutionConfirmation);
    }

    pub fn open_sql_query_selection_modal(&mut self, statements: Vec<String>) {
        let modal = SqlQuerySelectionModal::new(statements);
        self.sql_query_selection_modal = Some(modal);
        self.active_modal_type = Some(ModalType::SqlQuerySelection);
    }

    /// Open a cell value display modal
    #[allow(clippy::too_many_arguments)]
    pub fn open_cell_value_modal(
        &mut self,
        column_name: String,
        cell_value: &str,
        row_index: usize,
        col_index: usize,
        row_snapshot: Vec<String>,
        schema_name: String,
        table_name: String,
        primary_key: Vec<(String, String)>,
        db_row_id: Option<DbRowId>,
    ) {
        let modal = CellValueModal::new(
            column_name,
            cell_value,
            row_index,
            col_index,
            row_snapshot,
            schema_name,
            table_name,
            primary_key,
            db_row_id,
        );
        self.cell_value_modal = Some(modal);
        self.active_modal_type = Some(ModalType::CellValue);
    }

    /// Open a password input modal
    pub fn open_password_modal(
        &mut self,
        connection: Connection,
        prompt: String,
    ) {
        let modal = TextPromptModal::new("Enter Password", 50, 8)
            .with_prompt(prompt)
            .masked();
        self.password_modal = Some(modal);
        self.password_connection = Some(connection);
        self.active_modal_type = Some(ModalType::Password);
    }

    pub fn open_jump_to_row_modal(&mut self) {
        let modal = TextPromptModal::new(" Jump to row ", 36, 7)
            .with_placeholder("Row number")
            .with_validation(PromptValidation::PositiveInteger)
            .with_buttons("Go", "Cancel");
        self.jump_to_row_modal = Some(modal);
        self.active_modal_type = Some(ModalType::JumpToRow);
    }

    /// Row number if jump modal closed with Go.
    #[must_use]
    pub fn was_jump_to_row_confirmed(&self) -> Option<u64> {
        if let Some(modal) = &self.jump_to_row_modal
            && !modal.is_open
            && modal.submitted
        {
            return modal.parsed_positive_int();
        }
        None
    }

    #[must_use]
    pub const fn get_jump_to_row_modal(&self) -> Option<&TextPromptModal> {
        self.jump_to_row_modal.as_ref()
    }

    /// Connection associated with the currently open (or just-closed) password modal.
    #[must_use]
    pub const fn password_connection(&self) -> Option<&Connection> {
        self.password_connection.as_ref()
    }

    /// Close the currently active modal
    pub const fn close_active_modal(&mut self) {
        match self.active_modal_type {
            Some(ModalType::Connection) => {
                if let Some(modal) = &mut self.connection_modal {
                    modal.close();
                }
            }
            Some(ModalType::Confirmation) => {
                if let Some(modal) = &mut self.confirmation_modal {
                    modal.close();
                }
            }
            Some(ModalType::CellValue) => {
                if let Some(modal) = &mut self.cell_value_modal {
                    modal.close();
                }
            }
            Some(ModalType::SqlExecutionConfirmation) => {
                if let Some(modal) = &mut self.sql_execution_confirmation_modal
                {
                    modal.close();
                }
            }
            Some(ModalType::SqlQuerySelection) => {
                if let Some(modal) = &mut self.sql_query_selection_modal {
                    modal.close();
                }
            }
            Some(ModalType::Password) => {
                if let Some(modal) = &mut self.password_modal {
                    modal.close();
                }
            }
            Some(ModalType::JumpToRow) => {
                if let Some(modal) = &mut self.jump_to_row_modal {
                    modal.close();
                }
            }
            None => {}
        }
        self.active_modal_type = None;
    }

    /// Handle key events for the currently active modal (UI only)
    /// Returns the action that was triggered
    pub fn handle_key_events_ui(&mut self, key: KeyEvent) -> ModalAction {
        match self.active_modal_type {
            Some(ModalType::Connection) => {
                if let Some(modal) = &mut self.connection_modal {
                    let action = modal.handle_key_events_ui(key);
                    // If modal was closed, clear the active type
                    if !modal.is_open {
                        self.active_modal_type = None;
                    }
                    action
                } else {
                    ModalAction::None
                }
            }
            Some(ModalType::Confirmation) => {
                if let Some(modal) = &mut self.confirmation_modal {
                    let action = modal.handle_key_events(key);
                    if !modal.is_open {
                        self.active_modal_type = None;
                    }
                    Self::dialog_to_modal_action(action)
                } else {
                    ModalAction::None
                }
            }
            Some(ModalType::CellValue) => {
                if let Some(modal) = &mut self.cell_value_modal {
                    let action = modal.handle_key_events(key);
                    if action == ModalAction::Save {
                        self.cell_value_apply = Some(modal.build_apply());
                    }
                    if !modal.is_open {
                        self.active_modal_type = None;
                    }
                    action
                } else {
                    ModalAction::None
                }
            }
            Some(ModalType::SqlExecutionConfirmation) => {
                if let Some(modal) = &mut self.sql_execution_confirmation_modal
                {
                    let action = modal.handle_key_events(key);
                    if !modal.is_open {
                        self.active_modal_type = None;
                    }
                    Self::dialog_to_modal_action(action)
                } else {
                    ModalAction::None
                }
            }
            Some(ModalType::SqlQuerySelection) => {
                if let Some(modal) = &mut self.sql_query_selection_modal {
                    modal.handle_key_events(key);
                    if !modal.is_open {
                        self.active_modal_type = None;
                    }
                    if modal.submitted() {
                        ModalAction::Save
                    } else {
                        ModalAction::Cancel
                    }
                } else {
                    ModalAction::None
                }
            }
            Some(ModalType::Password) => {
                if let Some(modal) = &mut self.password_modal {
                    let action = modal.handle_key_events(key);
                    // If modal was closed, clear the active type
                    if !modal.is_open {
                        self.active_modal_type = None;
                    }
                    Self::dialog_to_modal_action(action)
                } else {
                    ModalAction::None
                }
            }
            Some(ModalType::JumpToRow) => {
                if let Some(modal) = &mut self.jump_to_row_modal {
                    let action = modal.handle_key_events(key);
                    if !modal.is_open {
                        self.active_modal_type = None;
                    }
                    Self::dialog_to_modal_action(action)
                } else {
                    ModalAction::None
                }
            }
            None => ModalAction::None,
        }
    }

    const fn dialog_to_modal_action(action: DialogAction) -> ModalAction {
        match action {
            DialogAction::Submit => ModalAction::Save,
            DialogAction::Cancel => ModalAction::Cancel,
            DialogAction::None => ModalAction::None,
        }
    }

    /// Get a reference to the connection modal
    #[must_use]
    #[allow(dead_code)]
    pub const fn get_connection_modal(&self) -> Option<&Modal> {
        self.connection_modal.as_ref()
    }

    /// Get a mutable reference to the connection modal
    pub const fn get_connection_modal_mut(&mut self) -> Option<&mut Modal> {
        self.connection_modal.as_mut()
    }

    /// Get a reference to the confirmation modal
    #[must_use]
    pub const fn get_confirmation_modal(&self) -> Option<&ConfirmDialog> {
        self.confirmation_modal.as_ref()
    }

    /// Check if the connection modal was just closed and needs a refresh
    #[must_use]
    pub fn was_connection_modal_closed(&self) -> bool {
        self.connection_modal.as_ref().is_some_and(|m| !m.is_open)
    }

    /// Check if the confirmation modal was just closed and confirmed
    #[must_use]
    pub fn was_confirmation_modal_confirmed(&self) -> Option<Connection> {
        if let Some(modal) = &self.confirmation_modal
            && !modal.is_open
            && modal.is_confirmed()
        {
            return self.confirmation_connection.clone();
        }

        None
    }

    /// Clear any closed modals from memory
    pub fn cleanup_closed_modals(&mut self) {
        if let Some(modal) = &self.connection_modal
            && !modal.is_open
        {
            self.connection_modal = None;
        }

        if let Some(modal) = &self.confirmation_modal
            && !modal.is_open
        {
            self.confirmation_modal = None;
            self.confirmation_connection = None;
        }

        if let Some(modal) = &self.cell_value_modal
            && !modal.is_open
        {
            self.cell_value_modal = None;
        }

        if let Some(modal) = &self.sql_execution_confirmation_modal
            && !modal.is_open
        {
            self.sql_execution_confirmation_modal = None;
        }

        if let Some(modal) = &self.sql_query_selection_modal
            && !modal.is_open
        {
            self.sql_query_selection_modal = None;
        }

        if let Some(modal) = &self.password_modal
            && !modal.is_open
        {
            self.password_modal = None;
            self.password_connection = None;
        }

        if let Some(modal) = &self.jump_to_row_modal
            && !modal.is_open
        {
            self.jump_to_row_modal = None;
        }
    }

    /// Get a reference to the password modal
    #[must_use]
    pub const fn get_password_modal(&self) -> Option<&TextPromptModal> {
        self.password_modal.as_ref()
    }

    /// Get a mutable reference to the password modal
    pub const fn get_password_modal_mut(
        &mut self,
    ) -> Option<&mut TextPromptModal> {
        self.password_modal.as_mut()
    }

    /// Get a reference to the cell value modal
    #[must_use]
    pub const fn get_cell_value_modal(&self) -> Option<&CellValueModal> {
        self.cell_value_modal.as_ref()
    }

    /// Take a pending cell edit after the modal closed with OK ([`ModalAction::Save`]).
    #[must_use]
    pub const fn take_cell_value_apply(&mut self) -> Option<CellValueApply> {
        self.cell_value_apply.take()
    }

    /// Check if SQL execution confirmation modal was just closed and confirmed.
    #[must_use]
    pub fn was_sql_execution_confirmed(&self) -> Option<String> {
        if let Some(modal) = &self.sql_execution_confirmation_modal
            && !modal.is_open
            && modal.is_confirmed()
        {
            return self.sql_execution_statement.clone();
        }
        None
    }

    /// Get a reference to the SQL execution confirmation modal.
    #[must_use]
    pub const fn get_sql_execution_confirmation_modal(
        &self,
    ) -> Option<&ConfirmDialog> {
        self.sql_execution_confirmation_modal.as_ref()
    }

    /// Check if SQL query selection modal was closed via Enter and return selected statement.
    #[must_use]
    pub fn was_sql_query_selected(&self) -> Option<String> {
        if let Some(modal) = &self.sql_query_selection_modal
            && !modal.is_open
            && modal.submitted()
        {
            return modal.selected_statement();
        }
        None
    }

    /// Get SQL query selection modal for rendering.
    #[must_use]
    pub const fn get_sql_query_selection_modal(
        &self,
    ) -> Option<&SqlQuerySelectionModal> {
        self.sql_query_selection_modal.as_ref()
    }

    /// Route bracketed paste to the active modal's focused input.
    pub fn handle_paste(&mut self, text: &str) -> bool {
        if !self.is_any_modal_open() {
            return false;
        }
        match self.active_modal_type {
            Some(ModalType::Connection) => {
                if let Some(modal) = &mut self.connection_modal {
                    modal.handle_paste(text);
                }
                true
            }
            Some(ModalType::CellValue) => {
                if let Some(modal) = &mut self.cell_value_modal {
                    modal.handle_paste(text);
                }
                true
            }
            Some(ModalType::Password) => {
                if let Some(modal) = &mut self.password_modal {
                    modal.handle_paste(text);
                }
                true
            }
            Some(ModalType::JumpToRow) => {
                if let Some(modal) = &mut self.jump_to_row_modal {
                    modal.handle_paste(text);
                }
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tls_form_tests {
    use super::Modal;
    use crate::db::{
        connection::{Connection, ConnectionType},
        tls::TlsConfig,
    };

    #[test]
    fn tls_fields_round_trip_through_url() {
        for (t, key) in [
            (ConnectionType::Postgres, "sslmode"),
            (ConnectionType::Mysql, "ssl-mode"),
        ] {
            let ca = "/tmp/my certs/a&b=c.pem";
            let conn = Connection {
                name: "n".into(),
                r#type: t,
                url: super::with_tls_query(
                    t,
                    &format!("{t}://u@h:1/db"),
                    "verify-full",
                    ca,
                ),
                password: Some("p".into()),
                ..Connection::default()
            };
            assert!(conn.url.contains(&format!("{key}=verify-full")));

            let mut modal = Modal::default();
            modal.open_for_edit(&conn);
            let saved = modal.get_connection().expect("valid form");
            let tls = TlsConfig::from_url(&saved.url);
            assert_eq!(tls.mode.label(), "verify-full");
            assert_eq!(tls.ca_path.as_deref(), Some(ca));
            // Same host/port/user/db survive the trip.
            assert!(saved.url.starts_with(&format!("{t}://u@h:1/db?")));
        }
    }

    #[test]
    fn disable_and_empty_ca_add_no_query() {
        let mut modal = Modal::default();
        modal.open_for_edit(&Connection {
            name: "n".into(),
            url: "postgres://u@h:1/db".into(),
            password: Some("p".into()),
            ..Connection::default()
        });
        let saved = modal.get_connection().expect("valid form");
        assert_eq!(saved.url, "postgres://u@h:1/db");
    }

    #[test]
    fn ca_field_hidden_when_tls_disabled() {
        let mut modal = Modal::default();
        modal.open_for_edit(&Connection {
            name: "n".into(),
            url: "postgres://u@h:1/db".into(),
            password: Some("p".into()),
            ..Connection::default()
        });
        assert!(modal.is_ca_field_hidden());
        // name..tls + env + metadata + password = 9 (no CA)
        assert_eq!(modal.visible_fields_count(), 9);

        modal.fields[super::TLS_FIELD].set_value("require");
        assert!(!modal.is_ca_field_hidden());
        assert_eq!(modal.visible_fields_count(), 10);

        modal.fields[super::TLS_FIELD].set_value("disable");
        assert!(modal.is_ca_field_hidden());
        assert_eq!(modal.visible_fields_count(), 9);
    }
}
