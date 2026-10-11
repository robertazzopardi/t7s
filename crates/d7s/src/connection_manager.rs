use color_eyre::Result;

use crate::{
    app::App,
    app_state::{AppState, DatabaseExplorerState},
    database_explorer_state::DatabaseExplorer,
    db::connection::{Connection, ConnectionStatus, ConnectionType},
    services::{ConnectionService, PreferencesService},
    ui::widgets::hotkeys::{CONNECTION_HOTKEYS, DATABASE_HOTKEYS},
};

impl App<'_> {
    /// Get the currently selected connection from the connection list
    pub fn get_selected_connection(&self) -> Option<&Connection> {
        let model = &self.database_explorer.connections.table.model;
        let state = &self.database_explorer.connections.table.view.state;
        state
            .selected()
            .filter(|&idx| idx < model.items.len())
            .and_then(|idx| model.items.get(idx))
    }

    /// Connect to the selected database
    pub async fn connect_to_database(&mut self) -> Result<()> {
        let Some(connection) = self.get_selected_connection() else {
            return Ok(());
        };

        // SQLite does not use passwords; connect directly without prompting
        if connection.r#type == ConnectionType::Sqlite {
            return self.connect_sqlite_direct(connection.clone()).await;
        }

        // Try to get password from service (checks session first, then keyring)
        if let Some(password) = self.password_service.get_password(connection) {
            self.connect_with_password(connection.clone(), password)
                .await?;
        } else {
            // Need to prompt for password
            let prompt = if connection.should_ask_every_time() {
                format!(
                    "Enter password for user '{}':",
                    connection.user_display()
                )
            } else {
                format!(
                    "Password not found for user '{}'.\nPlease enter password:",
                    connection.user_display()
                )
            };
            self.modal_manager
                .open_password_modal(connection.clone(), prompt);
        }
        Ok(())
    }

    /// Connect to `SQLite` database (no password)
    async fn connect_sqlite_direct(
        &mut self,
        connection: Connection,
    ) -> Result<()> {
        let sqlite = self.query_log.wrap(connection.to_sqlite());
        if !sqlite.test().await {
            self.set_status(format!(
                "Failed to connect to database: {}",
                connection.name
            ));
            return Ok(());
        }

        let connection_name = connection.name.clone();
        let connection_env = connection.environment;
        self.database_explorer =
            DatabaseExplorer::new(connection, Some(sqlite));
        self.state = AppState::DatabaseConnected;
        self.hotkeys = DATABASE_HOTKEYS.to_vec();
        let _ = PreferencesService::set_last_connection(&connection_name);
        let msg = self.connect_status_message(&connection_name, connection_env);
        self.set_status(msg);

        // SQLite doesn't need the Databases/Schemas navigation steps
        // Load tables directly from the default sqlite_schema
        self.load_tables("sqlite_schema").await?;
        Ok(())
    }

    /// Connect to database with the provided password
    pub async fn connect_with_password(
        &mut self,
        connection: Connection,
        password: String,
    ) -> Result<()> {
        // Create connection with password
        let mut connection_with_password = connection.clone();
        connection_with_password.password = Some(password);

        // For PostgreSQL, connect to a default database first to list databases.
        // MySQL can connect without a database, so the URL's database is used as-is.
        let default_db = (connection.r#type == ConnectionType::Postgres)
            .then(|| "postgres".to_string());

        // Create a temporary connection to the default database
        let mut temp_connection = connection_with_password.clone();
        temp_connection.selected_database.clone_from(&default_db);
        let postgres = self.query_log.wrap(temp_connection.to_database());

        if postgres.test().await {
            // Connection successful; keep selected_database so explorer is on "postgres"
            connection_with_password.selected_database = default_db;
            let connection_name = connection.name.clone();
            let connection_env = connection.environment;
            self.database_explorer =
                DatabaseExplorer::new(connection_with_password, Some(postgres));
            self.state = AppState::DatabaseConnected;

            // Update hotkeys for database mode
            self.hotkeys = DATABASE_HOTKEYS.to_vec();
            let _ = PreferencesService::set_last_connection(&connection_name);
            let msg =
                self.connect_status_message(&connection_name, connection_env);
            self.set_status(msg);

            // Load databases after successful connection
            self.load_databases().await?;
        } else {
            self.set_status(format!(
                "Failed to connect to database: {}",
                connection.name
            ));
        }
        Ok(())
    }

    /// Connect to a named connection from the connection list (CLI `-c`).
    pub async fn connect_to_named_connection(
        &mut self,
        name: &str,
    ) -> Result<()> {
        let idx = self
            .database_explorer
            .connections
            .table
            .model
            .items
            .iter()
            .position(|c| c.name == name);
        let Some(idx) = idx else {
            self.set_status(format!("Connection not found: {name}"));
            return Ok(());
        };
        self.database_explorer
            .connections
            .table
            .view
            .state
            .select(Some(idx));
        self.connect_to_database().await
    }

    /// Ping every saved connection concurrently (credential-free reachability
    /// probe, see `ConnectionService::ping`) and update each row's status column.
    /// Bounded by a per-probe timeout, so the UI is blocked for a few seconds at
    /// most. Statuses are written to both the unfiltered list and the visible
    /// (possibly filtered) rows so clearing a search keeps them.
    pub async fn check_connections_health(&mut self) {
        let connections = &mut self.database_explorer.connections;
        let mut set = tokio::task::JoinSet::new();
        for connection in connections.original.clone() {
            set.spawn(async move {
                let up = ConnectionService::ping(&connection).await;
                (connection.name, up)
            });
        }

        // A probe task that panics is reported as Down rather than left "Checking".
        let mut up_names = std::collections::HashSet::new();
        while let Some(result) = set.join_next().await {
            if let Ok((name, true)) = result {
                up_names.insert(name);
            }
        }
        let apply = |items: &mut [Connection]| {
            for item in items {
                item.status = if up_names.contains(&item.name) {
                    ConnectionStatus::Up
                } else {
                    ConnectionStatus::Down
                };
            }
        };
        apply(&mut connections.original);
        apply(&mut connections.table.model.items);

        self.set_status("Connection health check complete.");
    }

    /// Disconnect from the current database
    pub fn disconnect_from_database(&mut self) {
        self.database_explorer.state = DatabaseExplorerState::Connections;
        self.state = AppState::ConnectionList;

        // Update hotkeys for connection mode
        self.hotkeys = CONNECTION_HOTKEYS.to_vec();
    }
}
