use core::time::Duration;

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use k9tui::{
    theme,
    widgets::{navigation::TableNavigationHandler, table::TableDataState},
};
use ratatui::{
    style::{Color, Style},
    widgets::{Block, Borders, TableState},
};
use ratatui_textarea::TextArea;

use crate::{
    app::App,
    app_state::{AppState, DatabaseExplorerState},
    db::connection::ConnectionType,
    error::Result,
    services::{ConnectionService, PasswordService, PreferencesService},
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

const fn should_clear_status_on_key(key: KeyEvent) -> bool {
    !is_navigation_key(key.code)
        && !matches!(key.code, KeyCode::Char('y' | 'Y'))
}

impl App<'_> {
    /// Reads the crossterm events and updates the state of [`App`].
    ///
    /// If your application needs to perform work in between handling events, you can use the
    /// [`event::poll`] function to check if there are any events available with a timeout.
    #[allow(clippy::future_not_send)]
    pub async fn handle_crossterm_events(&mut self) -> Result<()> {
        // Short timeout (rather than a blocking read) so the run loop keeps
        // spinning to pick up watch-mode ticks between keypresses.
        if !event::poll(Duration::from_millis(200))? {
            return Ok(());
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if should_clear_status_on_key(key) {
                    self.clear_status();
                }
                // Nav keys leave the "Esc again to discard" status on screen
                // (excluded from should_clear_status_on_key above), so they
                // must not silently clear the pending flag it describes.
                // Esc is excluded here because its own handler below reads
                // this flag to decide first-press vs. second-press.
                if should_clear_status_on_key(key)
                    && !matches!(key.code, KeyCode::Esc)
                {
                    self.draft_discard_pending = false;
                }
                self.on_key_event(key).await?;
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
    #[allow(clippy::future_not_send, clippy::too_many_lines)]
    pub async fn on_key_event(&mut self, key: KeyEvent) -> Result<()> {
        // Help view: only toggle/close and quit
        if self.show_help {
            match (key.modifiers, key.code) {
                (_, KeyCode::Char('?') | KeyCode::Esc) => {
                    self.show_help = false;
                }
                (_, KeyCode::Char('q'))
                | (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => {
                    self.quit();
                }
                _ => {
                    TableNavigationHandler::navigate_table(
                        &self.help_table.model,
                        &mut self.help_table.view,
                        key.code,
                    );
                }
            }
            return Ok(());
        }

        // Describe view: only toggle/close, quit, and scroll
        if self.show_describe {
            match (key.modifiers, key.code) {
                (_, KeyCode::Char('d') | KeyCode::Esc) => {
                    self.show_describe = false;
                }
                (_, KeyCode::Char('q'))
                | (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => {
                    self.quit();
                }
                _ => {
                    TableNavigationHandler::navigate_table(
                        &self.describe_table.model,
                        &mut self.describe_table.view,
                        key.code,
                    );
                }
            }
            return Ok(());
        }

        // Query log view: Esc / q / L return; Ctrl+C quits
        if self.show_query_log {
            match (key.modifiers, key.code) {
                (_, KeyCode::Esc | KeyCode::Char('q' | 'L')) => {
                    self.show_query_log = false;
                }
                (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => {
                    self.quit();
                }
                _ => {
                    TableNavigationHandler::navigate_table(
                        &self.query_log_table.model,
                        &mut self.query_log_table.view,
                        key.code,
                    );
                }
            }
            return Ok(());
        }

        // `:` command bar shares the search bar's slot
        if self.command_mode && self.search_filter.is_some() {
            return self.handle_command_key(key).await;
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
        }

        // Handle modal events
        if self.modal_manager.is_any_modal_open() {
            return self.handle_modal_events(key).await;
        }

        // Handle application shortcuts (q, n, d, e, t, s, Esc, Enter)
        if self.handle_table_data_hotkeys(key).await? {
            return Ok(());
        }
        if self.handle_hotkeys(key).await? {
            return Ok(());
        }

        // Paged table data: j/k at the window edge load the next/previous page
        if self.state == AppState::DatabaseConnected
            && matches!(
                self.database_explorer.state,
                DatabaseExplorerState::TableData(..)
            )
            && self.try_step_virtual_table_page(key).await?
        {
            return Ok(());
        }

        // Handle navigation keys (j/k/h/l, 0/$, g/G, /)
        self.handle_navigation_keys(key);

        Ok(())
    }

    /// Handle application shortcuts (q, n, d, e, E, t, Esc, Enter)
    /// Returns true if the key was handled and should stop processing
    #[allow(clippy::too_many_lines, clippy::future_not_send)]
    async fn handle_hotkeys(&mut self, key: KeyEvent) -> Result<bool> {
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q'))
            | (KeyModifiers::CONTROL, KeyCode::Char('c' | 'C')) => {
                self.quit();
                Ok(true)
            }
            (_, KeyCode::Char('y')) => {
                self.copy();
                Ok(true)
            }
            (_, KeyCode::Char('Y')) => {
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::TableData(_, _)
                        | DatabaseExplorerState::SqlResults(_)
                ) {
                    self.copy_row_tsv();
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            (_, KeyCode::Char('?')) => {
                self.open_help();
                Ok(true)
            }
            (_, KeyCode::Char(c @ '1'..='5')) => {
                if self.state == AppState::DatabaseConnected {
                    let idx = usize::from(c as u8 - b'1');
                    if let Some((schema, table)) =
                        self.database_explorer.recent_tables.get(idx).cloned()
                    {
                        self.load_table_data(&schema, &table);
                    }
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            (_, KeyCode::Char('o')) => {
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::Connections
                ) {
                    self.connect_to_database()?;
                }
                Ok(true)
            }
            (_, KeyCode::Char('O')) => {
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::Connections
                ) {
                    if let Some(name) = PreferencesService::last_connection() {
                        let before = self.state;
                        self.connect_to_named_connection(&name)?;
                        if self.state == AppState::DatabaseConnected
                            && before != AppState::DatabaseConnected
                        {
                            self.set_status(format!("Reconnected to {name}"));
                        }
                    }
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            (_, KeyCode::Char('n')) => {
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::Connections
                ) {
                    self.modal_manager.open_new_connection_modal();
                }
                Ok(true)
            }
            (_, KeyCode::Char('p')) => {
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::Connections
                ) {
                    self.check_connections_health();
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            (_, KeyCode::Char('D')) => {
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::Connections
                ) {
                    self.handle_delete_connection();
                    return Ok(true);
                }
                Ok(false)
            }
            (_, KeyCode::Char('e')) => {
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::Connections
                ) {
                    self.handle_edit_connection();
                } else if self.state == AppState::DatabaseConnected {
                    self.request_editor();
                }
                Ok(true)
            }
            (_, KeyCode::Char('d')) => {
                let rows = self.build_describe_rows().await;
                if rows.is_empty() {
                    Ok(false)
                } else {
                    self.describe_table = TableDataState::new(rows);
                    self.show_describe = true;
                    Ok(true)
                }
            }
            (_, KeyCode::Char('t')) => {
                if self.state == AppState::DatabaseConnected {
                    self.handle_toggle_table_view().await?;
                }
                Ok(true)
            }
            (_, KeyCode::Char('E')) => {
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
                        return Ok(true);
                    }
                    let options = statements
                        .into_iter()
                        .map(|s| s.text)
                        .collect::<Vec<_>>();
                    self.modal_manager.open_sql_query_selection_modal(options);
                    return Ok(true);
                }
                Ok(false)
            }
            (_, KeyCode::Char('w'))
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::SqlResults(_)
                ) =>
            {
                self.toggle_watch();
                Ok(true)
            }
            (_, KeyCode::Char('L'))
                if self.state == AppState::DatabaseConnected =>
            {
                self.open_query_log_view();
                Ok(true)
            }
            (_, KeyCode::Char('A'))
                if self.state == AppState::DatabaseConnected
                    && self
                        .database_explorer
                        .connection
                        .r#type
                        .has_activity_view() =>
            {
                self.open_activity_view().await;
                Ok(true)
            }
            (KeyModifiers::CONTROL, KeyCode::Char('s' | 'S'))
            | (_, KeyCode::Char('x'))
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::SqlResults(_)
                ) =>
            {
                self.export_sql_results_tsv();
                Ok(true)
            }
            (_, KeyCode::Char(':')) => {
                self.open_command_bar();
                Ok(true)
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
                Ok(true)
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
                        self.set_status(
                            "Draft row pending — Esc again to discard, s to commit",
                        );
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
                        self.go_back_in_database().await;
                    }
                } else if self.state == AppState::ConnectionList {
                    self.clear_filter();
                }

                Ok(true)
            }
            (_, KeyCode::Enter) => {
                if matches!(
                    self.database_explorer.state,
                    DatabaseExplorerState::Connections
                ) {
                    self.connect_to_database()?;
                } else {
                    self.handle_database_navigation().await?;
                }
                Ok(true)
            }
            _ => Ok(false),
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
                self.handle_database_table_navigation(KeyCode::Down);
            }
            (_, KeyCode::Char('k') | KeyCode::Up) => {
                self.handle_database_table_navigation(KeyCode::Up);
            }
            (_, KeyCode::Char('h' | 'b') | KeyCode::Left) => {
                self.handle_database_table_navigation(KeyCode::Left);
            }
            (_, KeyCode::Char('l' | 'w') | KeyCode::Right) => {
                self.handle_database_table_navigation(KeyCode::Right);
            }
            // Jump to edges
            (_, KeyCode::Char('0')) => {
                self.handle_database_table_navigation(KeyCode::Char('0'));
            }
            (_, KeyCode::Char('$')) => {
                self.handle_database_table_navigation(KeyCode::Char('$'));
            }
            (_, KeyCode::Char('g')) => {
                self.handle_database_table_navigation(KeyCode::Char('g'));
            }
            (_, KeyCode::Char('G')) => {
                self.handle_database_table_navigation(KeyCode::Char('G'));
            }
            (_, KeyCode::Char('/'))
                if !self.modal_manager.is_any_modal_open() =>
            {
                let mut search_bar = TextArea::default();
                search_bar.set_cursor_line_style(Style::default());
                search_bar.set_placeholder_text("/");
                search_bar.set_style(Style::default().fg(Color::White));
                search_bar.set_max_histories(0);
                search_bar.set_block(
                    Block::default()
                        .border_style(theme::border())
                        .borders(Borders::ALL)
                        .title(" Filter (current view) "),
                );
                self.search_filter = Some(search_bar);
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
    async fn handle_toggle_table_view(&mut self) -> Result<()> {
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
        Ok(())
    }

    /// Handle modal events
    pub async fn handle_modal_events(&mut self, key: KeyEvent) -> Result<()> {
        let action = self.modal_manager.handle_key_events_ui(key);

        match action {
            ModalAction::Save => {
                if let Some(apply) = self.modal_manager.take_cell_value_apply()
                {
                    if self.apply_cell_value_edit(apply).await? {
                        self.cell_edit = None;
                    }
                    self.modal_manager.cleanup_closed_modals();
                    return Ok(());
                }
                if let Some(row) =
                    self.modal_manager.was_jump_to_row_confirmed()
                    && matches!(key.code, KeyCode::Enter)
                {
                    self.jump_to_table_row(row);
                    self.modal_manager.cleanup_closed_modals();
                    return Ok(());
                }
                if self.handle_password_modal_save()? {
                    return Ok(());
                }
                if let Some(statement) =
                    self.modal_manager.was_sql_query_selected()
                    && matches!(key.code, KeyCode::Enter)
                {
                    self.prepare_sql_statement_execution(statement).await;
                    self.modal_manager.cleanup_closed_modals();
                    return Ok(());
                }
                if self.pending_row_deletes.is_some()
                    && self
                        .modal_manager
                        .was_sql_execution_confirmed()
                        .is_some()
                    && matches!(key.code, KeyCode::Enter)
                {
                    self.execute_pending_row_deletes().await?;
                    self.modal_manager.cleanup_closed_modals();
                    return Ok(());
                }
                if let Some(statement) =
                    self.modal_manager.was_sql_execution_confirmed()
                    && matches!(key.code, KeyCode::Enter)
                {
                    self.execute_sql_statement_now(statement).await;
                    self.modal_manager.cleanup_closed_modals();
                    return Ok(());
                }
                if self
                    .modal_manager
                    .get_connection_modal()
                    .is_some_and(|m| m.is_open)
                {
                    self.handle_connection_modal_save().await;
                }
            }
            ModalAction::Test => {
                self.handle_connection_modal_test().await;
            }
            ModalAction::Cancel => {
                if self.modal_manager.was_connection_modal_closed() {
                    self.refresh_connections().await;
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
                self.refresh_connections().await;
            }
        }

        // Clean up closed modals
        self.modal_manager.cleanup_closed_modals();

        Ok(())
    }

    /// Handle password modal save action
    fn handle_password_modal_save(&mut self) -> Result<bool> {
        let Some(connection) =
            self.modal_manager.password_connection().cloned()
        else {
            return Ok(false);
        };

        let password = {
            let Some(password_modal) =
                self.modal_manager.get_password_modal_mut()
            else {
                return Ok(false);
            };
            password_modal.input_value()
        };

        self.spawn_connect(connection, Some(password))?;
        Ok(true)
    }

    /// Handle connection modal save action
    async fn handle_connection_modal_save(&mut self) {
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
        self.refresh_connections().await;
    }

    /// Handle connection modal test action
    async fn handle_connection_modal_test(&mut self) {
        let Some(modal) = self.modal_manager.get_connection_modal_mut() else {
            return;
        };

        let Some(connection) = modal.get_connection() else {
            modal.test_result =
                TestResult::Failed("Please fill in all fields".to_string());
            return;
        };

        modal.test_result = TestResult::Testing;
        let success = ConnectionService::test(&connection).await;
        modal.test_result = if success {
            TestResult::Success
        } else {
            TestResult::Failed("Connection failed".to_string())
        };
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::{app::App, app_state};

    #[tokio::test]
    async fn p_is_only_handled_on_the_connection_list() {
        let p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE);
        let mut app = App::default();
        assert!(app.handle_hotkeys(p).await.unwrap());

        // Anywhere else the key must fall through, not be swallowed.
        app.database_explorer.state =
            app_state::DatabaseExplorerState::Tables("s".into());
        assert!(!app.handle_hotkeys(p).await.unwrap());
    }
}
