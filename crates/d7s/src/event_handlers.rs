use core::time::Duration;

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use k9tui::{
    bg::spawn_keyed,
    error::Result,
    event::poll_input,
    keymap::{Keymap, Nav},
    widgets::navigation::TableNavigationHandler,
};
use ratatui::widgets::TableState;

use crate::{
    app::App,
    app_state::{AppState, DatabaseExplorerState},
    bg::BgEvent,
    db::connection::ConnectionType,
    in_flight::connection_test_key,
    services::{
        connection_service::ConnectionService,
        password_service::PasswordService,
        preferences_service::PreferencesService,
    },
    sql::safety::split_statements,
    ui::widgets::connection_modal::{ModalAction, TestResult},
};

const fn is_navigation_key(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::Char(
            'j' | 'k' | 'h' | 'l' | 'g' | 'G' | '0' | '$' | 'b' | 'w'
        ) | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Left
            | KeyCode::Right
    )
}

fn should_clear_status_on_key(key: KeyEvent, keymap: &Keymap) -> bool {
    !is_navigation_key(key.code)
        && !keymap.is("copy", key.code)
        && !keymap.is("copy-row", key.code)
}

impl App<'_> {
    /// Reads the crossterm events and updates the state of [`App`].
    ///
    /// If your application needs to perform work in between handling events, you can use the
    /// [`event::poll`] function to check if there are any events available with a timeout.
    pub fn handle_crossterm_events(&mut self) -> Result<()> {
        // Short timeout (rather than a blocking read) so the run loop keeps
        // spinning to pick up watch-mode ticks between keypresses; wakes early
        // when a background result is queued so it is drawn right away.
        let bg_rx = &self.bg_rx;
        if !poll_input(Duration::from_millis(200), || !bg_rx.is_empty())? {
            return Ok(());
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if should_clear_status_on_key(key, &self.config.keymap) {
                    self.clear_status();
                }
                // Nav keys leave the "Esc again to discard" status on screen
                // (excluded from should_clear_status_on_key above), so they
                // must not silently clear the pending flag it describes.
                // Esc is excluded here because its own handler below reads
                // this flag to decide first-press vs. second-press.
                if should_clear_status_on_key(key, &self.config.keymap)
                    && !matches!(key.code, KeyCode::Esc)
                {
                    self.draft_discard_pending = false;
                }
                self.on_key_event(key)?;
            }
            // Ignore non-press key events
            // Terminal resize is handled automatically by ratatui
            Event::Paste(text) => self.handle_paste_text(&text),
            Event::Key(_)
            | Event::FocusGained
            | Event::FocusLost
            | Event::Mouse(_)
            | Event::Resize(_, _) => {}
        }

        Ok(())
    }

    pub(crate) fn handle_paste_text(&mut self, text: &str) {
        self.clear_status();
        // Modal owns paste while open (Ctrl+e leaves cell_edit set).
        if self.modal_manager.is_any_modal_open() {
            let _ = self.modal_manager.handle_paste(text);
            return;
        }
        if let Some(session) = self.cell_edit.as_mut() {
            session.buffer.push_str(text);
            self.sync_cell_edit_buffer_to_grid();
            return;
        }
        if !self.modal_manager.handle_paste(text)
            && let Some(textarea) = &mut self.search_filter
        {
            textarea.insert_str(text);
        }
    }

    /// Handles the key events and updates the state of [`App`].
    #[allow(clippy::too_many_lines)]
    pub fn on_key_event(&mut self, key: KeyEvent) -> Result<()> {
        // Help view: only toggle/close and quit
        if self.show_help {
            match (key.modifiers, key.code) {
                (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => {
                    self.quit();
                }
                (_, code)
                    if code == KeyCode::Esc
                        || self.config.keymap.is("help", code) =>
                {
                    self.show_help = false;
                }
                (_, code) if self.config.keymap.is("quit", code) => {
                    self.quit();
                }
                _ => {
                    if let Some(nav) = self.config.keymap.nav("help", key.code)
                    {
                        TableNavigationHandler::navigate_table(
                            &self.help_table.model,
                            &mut self.help_table.view,
                            nav,
                        );
                    }
                }
            }
            return Ok(());
        }

        // Describe view: only toggle/close, filter, quit, and scroll.
        // While the filter bar is open, keys go to it below.
        if self.show_describe && self.search_filter.is_none() {
            match (key.modifiers, key.code) {
                (_, KeyCode::Esc) if self.active_filter_query().is_some() => {
                    self.clear_filter();
                }
                (_, KeyCode::Char('d') | KeyCode::Esc) => {
                    self.show_describe = false;
                }
                (_, KeyCode::Char('/')) => self.open_filter_bar(),
                (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => {
                    self.quit();
                }
                (_, code) if self.config.keymap.is("quit", code) => {
                    self.quit();
                }
                _ => {
                    if let Some(nav) =
                        self.config.keymap.nav("describe", key.code)
                    {
                        self.describe_table.navigate(nav);
                    }
                }
            }
            return Ok(());
        }

        // Query log view: Esc and the quit and query-log keys return; Ctrl+C quits
        if self.show_query_log {
            match (key.modifiers, key.code) {
                (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => {
                    self.quit();
                }
                (_, code)
                    if code == KeyCode::Esc
                        || self.config.keymap.is("quit", code)
                        || self.config.keymap.is("query-log", code) =>
                {
                    self.show_query_log = false;
                }
                _ => {
                    if let Some(nav) =
                        self.config.keymap.nav("query-log", key.code)
                    {
                        TableNavigationHandler::navigate_table(
                            &self.query_log_table.model,
                            &mut self.query_log_table.view,
                            nav,
                        );
                    }
                }
            }
            return Ok(());
        }

        // `:` command bar shares the search bar's slot
        if self.command_mode && self.search_filter.is_some() {
            self.handle_command_key(key);
            return Ok(());
        }

        // Handle search filter input first
        if let Some(textarea) = &mut self.search_filter {
            if key.code == KeyCode::Esc {
                self.clear_filter();
                self.search_filter = None;
                return Ok(());
            }
            if key.code == KeyCode::Enter {
                self.apply_filter();
                self.search_filter = None;
                return Ok(());
            }

            if textarea.input(key) {
                let query = textarea
                    .lines()
                    .first()
                    .map_or_else(String::new, Clone::clone);
                self.apply_filter_with_query(&query);
                return Ok(());
            }
            // Cursor keys move the text cursor only; without this they also
            // scrolled the table's columns while typing.
            if matches!(
                key.code,
                KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End
            ) {
                return Ok(());
            }
            // The hotkeys below act on the explorer hidden behind describe.
            if self.show_describe {
                match (key.modifiers, key.code) {
                    (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => {
                        self.quit();
                    }
                    (_, KeyCode::Up) => self.describe_table.navigate(Nav::Up),
                    (_, KeyCode::Down) => {
                        self.describe_table.navigate(Nav::Down);
                    }
                    _ => {}
                }
                return Ok(());
            }
        }

        // Handle modal events
        if self.modal_manager.is_any_modal_open() {
            self.handle_modal_events(key);
            return Ok(());
        }
        self.pending_nav = None;

        // Handle application shortcuts (q, n, d, e, t, s, Esc, Enter)
        if self.handle_table_data_hotkeys(key)? {
            return Ok(());
        }
        if self.handle_hotkeys(key) {
            return Ok(());
        }

        // Paged table data: j/k at the window edge load the next/previous page
        if self.state == AppState::DatabaseConnected
            && matches!(
                self.database_explorer.state,
                DatabaseExplorerState::TableData(..)
            )
            && self.try_step_virtual_table_page(key)
        {
            return Ok(());
        }

        // Handle navigation keys (j/k/h/l, 0/$, g/G, /)
        self.handle_navigation_keys(key);

        Ok(())
    }

    /// Handle application shortcuts (q, n, d, e, E, t, Esc, Enter)
    /// Returns true if the key was handled and should stop processing
    #[allow(clippy::too_many_lines)]
    fn handle_hotkeys(&mut self, key: KeyEvent) -> bool {
        let on_connections = matches!(
            self.database_explorer.state,
            DatabaseExplorerState::Connections
        );
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => {
                self.quit();
                true
            }
            (KeyModifiers::CONTROL, KeyCode::Char('s' | 'S'))
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::SqlResults(_)
                ) =>
            {
                self.export_sql_results_tsv();
                true
            }
            (_, code) if self.config.keymap.is("quit", code) => {
                self.quit();
                true
            }
            (_, code) if self.config.keymap.is("copy", code) => {
                self.copy();
                true
            }
            (_, code) if self.config.keymap.is("copy-row", code) => {
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::TableData(_, _)
                        | DatabaseExplorerState::SqlResults(_)
                ) {
                    self.copy_row_tsv();
                    true
                } else {
                    false
                }
            }
            (_, code) if self.config.keymap.is("help", code) => {
                self.open_help();
                true
            }
            (_, KeyCode::Char(c @ '1'..='5')) => {
                if self.state == AppState::DatabaseConnected {
                    let idx = usize::from(c as u8 - b'1');
                    if let Some((schema, table)) =
                        self.database_explorer.recent_tables.get(idx).cloned()
                    {
                        self.load_table_data(&schema, &table);
                    }
                    true
                } else {
                    false
                }
            }
            (_, code)
                if on_connections
                    && self.config.keymap.is("open-connection", code) =>
            {
                self.connect_to_database();
                true
            }
            (_, code)
                if on_connections
                    && self.config.keymap.is("reconnect", code) =>
            {
                if let Some(name) = PreferencesService::last_connection() {
                    let before = self.state;
                    self.connect_to_named_connection(&name);
                    if self.state == AppState::DatabaseConnected
                        && before != AppState::DatabaseConnected
                    {
                        self.set_status(format!("Reconnected to {name}"));
                    }
                }
                true
            }
            (_, code)
                if on_connections
                    && self.config.keymap.is("new-connection", code) =>
            {
                self.modal_manager.open_new_connection_modal();
                true
            }
            (_, code)
                if on_connections && self.config.keymap.is("ping", code) =>
            {
                self.check_connections_health();
                true
            }
            (_, code)
                if on_connections
                    && self.config.keymap.is("delete-connection", code) =>
            {
                self.handle_delete_connection();
                true
            }
            (_, code)
                if on_connections
                    && self.config.keymap.is("edit-connection", code) =>
            {
                self.handle_edit_connection();
                true
            }
            (_, code)
                if self.state == AppState::DatabaseConnected
                    && self.config.keymap.is("sql-editor", code) =>
            {
                self.request_editor();
                true
            }
            (_, code) if self.config.keymap.is("describe", code) => {
                self.spawn_describe_rows()
            }
            (_, code)
                if self.state == AppState::DatabaseConnected
                    && self.config.keymap.is("table-structure", code) =>
            {
                self.handle_toggle_table_view();
                true
            }
            (_, code) if self.config.keymap.is("run-sql", code) => {
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::SqlResults(_)
                ) {
                    let statements = split_statements(
                        &self.database_explorer.sql_executor.sql_input(),
                    );
                    if statements.is_empty() {
                        self.set_status(
                            "No SQL statements found in editor file.",
                        );
                        return true;
                    }
                    let options = statements
                        .into_iter()
                        .map(|s| s.text)
                        .collect::<Vec<_>>();
                    self.modal_manager.open_sql_query_selection_modal(options);
                    return true;
                }
                false
            }
            (_, code)
                if self.config.keymap.is("watch", code)
                    && matches!(
                        self.database_explorer.state,
                        DatabaseExplorerState::SqlResults(_)
                    ) =>
            {
                self.toggle_watch();
                true
            }
            (_, code) if self.config.keymap.is("width", code) => {
                self.toggle_column_width_mode();
                true
            }
            (_, code)
                if self.config.keymap.is("query-log", code)
                    && self.state == AppState::DatabaseConnected =>
            {
                self.open_query_log_view();
                true
            }
            (_, code)
                if self.config.keymap.is("activity", code)
                    && self.state == AppState::DatabaseConnected
                    && self
                        .database_explorer
                        .connection
                        .r#type
                        .has_activity_view() =>
            {
                self.open_activity_view();
                true
            }
            (_, code)
                if self.config.keymap.is("export", code)
                    && matches!(
                        self.database_explorer.state,
                        DatabaseExplorerState::SqlResults(_)
                    ) =>
            {
                self.export_sql_results_tsv();
                true
            }
            (_, code) if self.config.keymap.is("command", code) => {
                self.open_command_bar();
                true
            }
            (_, KeyCode::Char('#'))
                if self.state == AppState::DatabaseConnected
                    && matches!(
                        self.database_explorer.state,
                        DatabaseExplorerState::TableData(_, _)
                    )
                    && !self.has_active_filter() =>
            {
                self.modal_manager.open_jump_to_row_modal();
                true
            }
            (_, KeyCode::Esc) => {
                // Cell-edit Esc is handled in `handle_cell_edit_key` (table data
                // path returns before this global hotkey table).
                if self.modal_manager.is_any_modal_open() {
                    self.modal_manager.close_active_modal();
                } else if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::TableData(_, _)
                ) && self.has_table_draft_rows()
                {
                    // Draft discard before column-deselect so n/c (which select
                    // a column) still use the two-step Esc story.
                    if self.draft_discard_pending {
                        if self.discard_table_draft() {
                            self.set_status("Draft discarded.");
                        }
                    } else {
                        self.draft_discard_pending = true;
                        self.set_status(format!(
                            "Draft row pending — Esc again to discard, {} to commit",
                            self.config.keymap.label("commit")
                        ));
                    }
                } else if self
                    .database_explorer
                    .current_table_state_mut()
                    .and_then(|s| s.selected_column())
                    .is_some()
                {
                    self.reset_table_selection_state();
                } else if self.state == AppState::DatabaseConnected {
                    let is_sql_executor = matches!(
                        self.database_explorer.state,
                        DatabaseExplorerState::SqlResults(_)
                    );

                    if is_sql_executor {
                        self.escape_from_or_return_to_sql_editor();
                    } else if self.has_active_filter() {
                        self.clear_filter();
                    } else {
                        self.go_back_in_database();
                    }
                } else if self.state == AppState::ConnectionList {
                    self.clear_filter();
                }

                true
            }
            (_, KeyCode::Enter) => {
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::Connections
                ) {
                    self.connect_to_database();
                } else {
                    self.handle_database_navigation();
                }
                true
            }
            _ => false,
        }
    }

    fn escape_from_or_return_to_sql_editor(&mut self) {
        self.database_explorer.sql_executor.deactivate();
        if let Some(previous_state) =
            self.database_explorer.previous_state.take()
        {
            self.database_explorer.state = previous_state;
        }
    }

    fn reset_table_selection_state(&mut self) {
        let explorer = &mut self.database_explorer;
        let Some(table_state) = explorer.current_table_state_mut() else {
            // NOTE do nothing to the table state if not state returned
            return;
        };

        let row = table_state.selected();
        *table_state = TableState::default();
        table_state.select(row);
    }

    /// Handle navigation keys (j/k/h/l, 0/$, g/G, /)
    fn handle_navigation_keys(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            // Vim keybindings for table navigation (Connections and DatabaseConnected use same handler)
            (_, KeyCode::Char('j') | KeyCode::Down) => {
                self.handle_database_table_navigation(Nav::Down);
            }
            (_, KeyCode::Char('k') | KeyCode::Up) => {
                self.handle_database_table_navigation(Nav::Up);
            }
            (_, KeyCode::Char('h' | 'b') | KeyCode::Left) => {
                self.handle_database_table_navigation(Nav::Left);
            }
            (_, KeyCode::Char('l' | 'w') | KeyCode::Right) => {
                self.handle_database_table_navigation(Nav::Right);
            }
            // Jump to edges
            (_, KeyCode::Char('0')) => {
                self.handle_database_table_navigation(Nav::FirstColumn);
            }
            (_, KeyCode::Char('$')) => {
                self.handle_database_table_navigation(Nav::LastColumn);
            }
            (_, KeyCode::Char('g')) => {
                self.handle_database_table_navigation(Nav::Top);
            }
            (_, KeyCode::Char('G')) => {
                self.handle_database_table_navigation(Nav::Bottom);
            }
            (_, code)
                if self.config.keymap.is("search", code)
                    && !self.modal_manager.is_any_modal_open() =>
            {
                self.open_filter_bar();
            }
            _ => {}
        }
    }

    /// Handle delete connection action
    fn handle_delete_connection(&mut self) {
        let Some(connection) = self.get_selected_connection() else {
            return;
        };
        let message = format!(
            "Are you sure you want to delete\nthe connection '{}'?\n\nThis action cannot be undone.",
            connection.name
        );
        self.modal_manager
            .open_confirmation_modal(message, connection.clone());
    }

    /// Handle edit connection action
    fn handle_edit_connection(&mut self) {
        let Some(connection) = self.get_selected_connection() else {
            return;
        };
        let password = PasswordService::get_connection_password(connection);
        let connection = connection.clone();
        self.modal_manager
            .open_edit_connection_modal(&connection, password);
    }

    /// Handle toggle between table data and columns view
    fn handle_toggle_table_view(&mut self) {
        let state = self.database_explorer.state.clone();

        match state {
            DatabaseExplorerState::TableData(schema_name, table_name) => {
                self.load_columns(&schema_name, &table_name);
            }
            DatabaseExplorerState::Columns(schema_name, table_name) => {
                self.load_table_data(&schema_name, &table_name);
            }
            DatabaseExplorerState::Connections
            | DatabaseExplorerState::Databases
            | DatabaseExplorerState::Schemas
            | DatabaseExplorerState::Tables(_)
            | DatabaseExplorerState::SqlResults(_) => {}
        }
    }

    /// Handle modal events
    pub fn handle_modal_events(&mut self, key: KeyEvent) {
        let action = self.modal_manager.handle_key_events_ui(key);

        match action {
            ModalAction::Save => {
                if let Some(apply) = self.modal_manager.take_cell_value_apply()
                {
                    if self.spawn_cell_value_edit(&apply) {
                        self.cell_edit = None;
                    }
                    self.modal_manager.cleanup_closed_modals();
                    return;
                }
                if let Some(row) =
                    self.modal_manager.was_jump_to_row_confirmed()
                    && matches!(key.code, KeyCode::Enter)
                {
                    self.jump_to_table_row(row);
                    self.modal_manager.cleanup_closed_modals();
                    return;
                }
                if self.handle_password_modal_save() {
                    return;
                }
                if let Some(statement) =
                    self.modal_manager.was_sql_query_selected()
                    && matches!(key.code, KeyCode::Enter)
                {
                    self.prepare_sql_statement_execution(statement);
                    self.modal_manager.cleanup_closed_modals();
                    return;
                }
                if self.pending_row_deletes.is_some()
                    && self
                        .modal_manager
                        .was_sql_execution_confirmed()
                        .is_some()
                    && matches!(key.code, KeyCode::Enter)
                {
                    self.spawn_pending_row_deletes();
                    self.modal_manager.cleanup_closed_modals();
                    return;
                }
                if let Some(statement) =
                    self.modal_manager.was_sql_execution_confirmed()
                    && matches!(key.code, KeyCode::Enter)
                {
                    self.execute_sql_statement_now(statement);
                    self.modal_manager.cleanup_closed_modals();
                    return;
                }
                if self
                    .modal_manager
                    .get_connection_modal()
                    .is_some_and(|m| m.is_open)
                {
                    self.handle_connection_modal_save();
                }
            }
            ModalAction::Test => {
                self.spawn_connection_modal_test();
            }
            ModalAction::Cancel => {
                if self.modal_manager.was_connection_modal_closed() {
                    self.refresh_connections();
                }
                self.pending_row_deletes = None;
            }
            ModalAction::None => {}
        }

        // Handle confirmation modal results
        if let Some(connection) =
            self.modal_manager.was_confirmation_modal_confirmed()
            && matches!(key.code, KeyCode::Enter)
        {
            // Delete from keyring only for server DBs (SQLite has no passwords)
            if connection.r#type != ConnectionType::Sqlite
                && !connection.should_ask_every_time()
            {
                let _ = PasswordService::delete_from_keyring(&connection.name);
            }

            // Delete connection using ConnectionService
            if let Err(e) = ConnectionService::delete(&connection.name) {
                self.set_status(format!("Failed to delete connection: {e}"));
            } else {
                self.refresh_connections();
            }
        }

        // Clean up closed modals
        self.modal_manager.cleanup_closed_modals();
    }

    /// Handle password modal save action
    fn handle_password_modal_save(&mut self) -> bool {
        let Some(connection) =
            self.modal_manager.password_connection().cloned()
        else {
            return false;
        };

        let password = {
            let Some(password_modal) =
                self.modal_manager.get_password_modal_mut()
            else {
                return false;
            };
            password_modal.input_value()
        };

        self.spawn_connect(connection, Some(password));
        true
    }

    /// Handle connection modal save action
    fn handle_connection_modal_save(&mut self) {
        let Some(modal) = self.modal_manager.get_connection_modal_mut() else {
            return;
        };

        let Some(connection) = modal.get_connection() else {
            return;
        };

        let original_name = modal.original_name.clone();

        if let Err(error_msg) = ConnectionService::validate(&connection) {
            modal.test_result = TestResult::Failed(error_msg);
            return;
        }

        if connection.uses_keyring()
            && let Some(password) = &connection.password
            && let Err(e) =
                PasswordService::save_to_keyring(&connection.name, password)
        {
            modal.test_result =
                TestResult::Failed(format!("Failed to save password: {e}"));
            return;
        }

        let save_result = original_name.as_ref().map_or_else(
            || ConnectionService::create(&connection),
            |orig_name| ConnectionService::update(orig_name, &connection),
        );

        match save_result {
            Ok(()) => {
                // If switching to "ask every time" on an existing connection,
                // delete the old keyring credential after the save succeeds.
                if let Some(orig_name) = &original_name
                    && connection.should_ask_every_time()
                {
                    let _ = PasswordService::delete_from_keyring(orig_name);
                }
                modal.close();
            }
            Err(e) => {
                modal.test_result = TestResult::Failed(e.to_string());
                return;
            }
        }
        // Drop modal borrow before await (status is runtime-only; refresh re-pings).
        self.refresh_connections();
    }

    /// Handle connection modal test action (off the UI thread).
    fn spawn_connection_modal_test(&mut self) {
        let Some(modal) = self.modal_manager.get_connection_modal_mut() else {
            return;
        };

        let Some(connection) = modal.get_connection() else {
            modal.test_result =
                TestResult::Failed("Please fill in all fields".to_string());
            return;
        };

        let key = connection_test_key(&connection.name);
        if !self.in_flight.try_begin(key.clone()) {
            return;
        }

        modal.test_result = TestResult::Testing;
        let tx = self.bg_tx.clone();
        spawn_keyed(tx.clone(), key.clone(), async move {
            let success = ConnectionService::test(&connection).await;
            let message = if success {
                String::new()
            } else {
                "Connection failed".into()
            };
            let _ = tx.send(BgEvent::ConnectionTestResult {
                key,
                success,
                message,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, buffer::Cell};

    use super::should_clear_status_on_key;
    use crate::{
        app::App,
        app_state::{self, AppState, DatabaseExplorerState},
        db::connection::Environment,
    };

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn connected_app() -> App<'static> {
        let mut app = App {
            state: AppState::DatabaseConnected,
            ..App::default()
        };
        app.database_explorer.state = DatabaseExplorerState::Tables("s".into());
        app
    }

    #[tokio::test]
    async fn edit_connection_remap_leaves_sql_editor_on_e() {
        let mut app = connected_app();
        assert!(app.config.keymap.set("edit-connection", KeyCode::Char('z')));
        assert!(app.handle_hotkeys(key('e')));
        assert!(app.open_editor_requested, "e still opens the editor");

        let mut list = App::default();
        assert!(
            list.config
                .keymap
                .set("edit-connection", KeyCode::Char('z'))
        );
        assert!(!list.handle_hotkeys(key('e')), "e is free on the list");
        assert!(!list.open_editor_requested);
        assert!(list.handle_hotkeys(key('z')), "z edits on the list");
    }

    #[tokio::test]
    async fn sql_editor_remap_leaves_edit_connection_on_e() {
        let mut app = connected_app();
        assert!(app.config.keymap.set("sql-editor", KeyCode::Char('z')));
        assert!(!app.handle_hotkeys(key('e')), "e is free when connected");
        assert!(!app.open_editor_requested);
        assert!(app.handle_hotkeys(key('z')));
        assert!(app.open_editor_requested);
    }

    #[tokio::test]
    async fn new_connection_remap_does_not_swallow_key_on_other_screens() {
        let mut app = connected_app();
        assert!(app.config.keymap.set("new-connection", KeyCode::Char('z')));
        assert!(!app.handle_hotkeys(key('z')));
    }

    #[tokio::test]
    async fn remapped_quit_follows_config() {
        let mut app = App {
            running: true,
            ..App::default()
        };
        assert!(app.config.keymap.set("quit", KeyCode::Char('z')));
        assert!(!app.handle_hotkeys(key('q')));
        assert!(app.running, "old key is released");
        assert!(app.handle_hotkeys(key('z')));
        assert!(!app.running, "new key fires");
    }

    #[tokio::test]
    async fn ctrl_c_still_quits_when_quit_is_remapped() {
        let mut app = App {
            running: true,
            ..App::default()
        };
        assert!(app.config.keymap.set("quit", KeyCode::Char('z')));
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.handle_hotkeys(ctrl_c));
        assert!(!app.running);
    }

    #[tokio::test]
    async fn ctrl_s_still_exports_when_s_is_remapped() {
        let mut app = App {
            running: true,
            ..App::default()
        };
        app.database_explorer.state =
            DatabaseExplorerState::SqlResults("select 1".into());
        assert!(app.config.keymap.set("quit", KeyCode::Char('s')));
        let ctrl_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(app.handle_hotkeys(ctrl_s));
        assert!(app.running, "Ctrl+s must not fire the bare-key action");
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(Cell::symbol)
            .collect();
        assert!(screen.contains("No results to export"), "{screen}");
    }

    #[tokio::test]
    async fn draft_pending_hint_names_the_commit_key() {
        let mut app = App::default();
        assert!(app.draft_pending_hint().contains("s to commit"));
        assert!(app.config.keymap.set("commit", KeyCode::Char('z')));
        assert!(app.draft_pending_hint().contains("z to commit"));
    }

    #[tokio::test]
    async fn connected_hint_names_the_help_key() {
        let mut app = App::default();
        assert!(app.config.keymap.set("help", KeyCode::F(1)));
        let hint = app.connect_status_message("db", Environment::default());
        assert!(hint.contains("press F1 for help"), "{hint}");
    }

    #[tokio::test]
    async fn empty_list_hint_names_the_new_connection_key() {
        let mut app = App::default();
        assert!(app.config.keymap.set("new-connection", KeyCode::Char('m')));
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(Cell::symbol)
            .collect();
        assert!(screen.contains("Press m to add"), "{screen}");
    }

    #[tokio::test]
    async fn watch_status_names_the_watch_key() {
        let mut app = App::default();
        assert!(app.config.keymap.set("watch", KeyCode::Char('z')));
        app.toggle_watch();
        assert!(
            app.status_line.message().contains("press z to stop"),
            "{}",
            app.status_line.message()
        );
    }

    #[tokio::test]
    async fn help_view_follows_remapped_keys() {
        let mut app = App {
            running: true,
            show_help: true,
            ..App::default()
        };
        assert!(app.config.keymap.set("help", KeyCode::Char('z')));
        assert!(app.config.keymap.set("quit", KeyCode::Char('x')));
        app.on_key_event(key('q')).unwrap();
        assert!(app.running, "a freed q must not quit");
        app.on_key_event(key('?')).unwrap();
        assert!(app.show_help, "a freed ? must not close help");
        app.on_key_event(key('z')).unwrap();
        assert!(!app.show_help, "the help key closes help");
        app.show_help = true;
        app.on_key_event(key('x')).unwrap();
        assert!(!app.running, "the quit key quits from help");
    }

    #[tokio::test]
    async fn describe_view_quits_with_remapped_key_only() {
        let mut app = App {
            running: true,
            show_describe: true,
            ..App::default()
        };
        assert!(app.config.keymap.set("quit", KeyCode::Char('x')));
        app.on_key_event(key('q')).unwrap();
        assert!(app.running, "a freed q must not quit");
        app.on_key_event(key('x')).unwrap();
        assert!(!app.running);
    }

    #[tokio::test]
    async fn query_log_closes_with_remapped_key() {
        let mut app = App {
            show_query_log: true,
            ..App::default()
        };
        assert!(app.config.keymap.set("query-log", KeyCode::Char('z')));
        app.on_key_event(key('L')).unwrap();
        assert!(app.show_query_log, "a freed L must not close the log");
        app.on_key_event(key('z')).unwrap();
        assert!(!app.show_query_log);
    }

    #[tokio::test]
    async fn remapped_search_opens_filter_bar() {
        let mut app = App::default();
        assert!(app.config.keymap.set("search", KeyCode::Char('z')));
        app.handle_navigation_keys(key('/'));
        assert!(app.search_filter.is_none(), "old key is released");
        app.handle_navigation_keys(key('z'));
        assert!(app.search_filter.is_some(), "new key fires");
    }

    #[tokio::test]
    async fn copy_status_survives_remapped_copy_key() {
        let mut app = App::default();
        assert!(!should_clear_status_on_key(key('y'), &app.config.keymap));
        assert!(app.config.keymap.set("copy", KeyCode::Char('z')));
        assert!(!should_clear_status_on_key(key('z'), &app.config.keymap));
        assert!(should_clear_status_on_key(key('y'), &app.config.keymap));
    }

    #[tokio::test]
    async fn p_is_only_handled_on_the_connection_list() {
        let p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE);
        let mut app = App::default();
        assert!(app.handle_hotkeys(p));

        // Anywhere else the key must fall through, not be swallowed.
        app.database_explorer.state =
            app_state::DatabaseExplorerState::Tables("s".into());
        assert!(!app.handle_hotkeys(p));
    }
}
