use std::sync::Arc;

use tokio::task::spawn_blocking;

use crate::{
    app::App,
    app_state::{AppState, DatabaseExplorerState},
    bg::{BgEvent, spawn_keyed},
    database_explorer_state::DatabaseExplorer,
    db::{
        Database,
        connection::{
            Connection, ConnectionStatus, ConnectionType, parse_server_url,
        },
        query_log::QueryLog,
    },
    error::Result,
    in_flight::{connection_key, ping_key},
    services::{ConnectionService, PasswordService, PreferencesService},
    ui::widgets::hotkeys::{CONNECTION_HOTKEYS, database_hotkeys},
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

    /// Connect to the selected database (spawns work; does not await).
    pub fn connect_to_database(&mut self) {
        let Some(connection) = self.get_selected_connection().cloned() else {
            return;
        };

        if connection.r#type == ConnectionType::Sqlite {
            self.spawn_connect(connection, None);
            return;
        }

        if connection.should_ask_every_time() {
            if let Some(password) = self
                .password_service
                .get_session_password(&connection)
                .cloned()
            {
                self.spawn_connect(connection, Some(password));
                return;
            }
            let prompt = format!(
                "Enter password for user '{}':",
                connection.user_display()
            );
            self.modal_manager.open_password_modal(connection, prompt);
            return;
        }

        self.spawn_connect(connection, None);
    }

    /// Spawn connect: optional password, or keyring lookup in the background task.
    pub(crate) fn spawn_connect(
        &mut self,
        connection: Connection,
        password: Option<String>,
    ) {
        let key = connection_key(&connection.name);
        if !self.in_flight.try_begin(key.clone()) {
            self.set_status(format!(
                "Already connecting to {}",
                connection.name
            ));
            return;
        }

        self.latest_connect_key = Some(key.clone());
        self.set_status("Connecting…");
        let query_log = self.query_log.clone();
        let tx = self.bg_tx.clone();
        let name = connection.name.clone();
        let conn_type = connection.r#type;
        let resolve_keyring =
            password.is_none() && conn_type != ConnectionType::Sqlite;

        spawn_keyed(tx.clone(), key.clone(), async move {
            let result = if conn_type == ConnectionType::Sqlite {
                build_sqlite_database(query_log, connection).await
            } else {
                let password = match password {
                    Some(p) => Some(p),
                    None if resolve_keyring => spawn_blocking({
                        let name = name.clone();
                        move || PasswordService::get_from_keyring(&name).ok()
                    })
                    .await
                    .ok()
                    .flatten(),
                    None => None,
                };

                if let Some(p) = password {
                    build_server_database(query_log, connection, p).await
                } else {
                    let prompt = format!(
                        "Password not found for user '{}'.\nPlease enter password:",
                        connection.user_display()
                    );
                    let _ = tx.send(BgEvent::ConnectPasswordRequired {
                        key,
                        connection,
                        prompt,
                    });
                    return;
                }
            };

            match result {
                Ok((database, connection)) => {
                    let _ = tx.send(BgEvent::Connected {
                        key,
                        connection,
                        database,
                    });
                }
                Err(message) => {
                    let _ = tx.send(BgEvent::ConnectFailed { key, message });
                }
            }
        });
    }

    pub(crate) fn apply_connected(
        &mut self,
        connection: Connection,
        database: Arc<dyn Database>,
    ) {
        let connection_name = connection.name.clone();
        let connection_env = connection.environment;
        let conn_type = connection.r#type;
        self.hotkeys = database_hotkeys(conn_type);
        self.database_explorer =
            DatabaseExplorer::new(connection, Some(database));
        self.state = AppState::DatabaseConnected;
        let _ = PreferencesService::set_last_connection(&connection_name);
        let msg = self.connect_status_message(&connection_name, connection_env);
        self.set_status(msg);

        if conn_type == ConnectionType::Sqlite {
            self.load_tables("sqlite_schema");
        } else {
            self.load_databases();
        }
    }

    pub(crate) fn apply_connection_status(
        &mut self,
        name: &str,
        status: ConnectionStatus,
    ) {
        let connections = &mut self.database_explorer.connections;
        for item in &mut connections.original {
            if item.name == name {
                item.status = status;
            }
        }
        for item in &mut connections.table.model.items {
            if item.name == name {
                item.status = status;
            }
        }
    }

    /// Connect to a named connection from the connection list (CLI `-c`).
    pub fn connect_to_named_connection(&mut self, name: &str) {
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
            return;
        };
        self.database_explorer
            .connections
            .table
            .view
            .state
            .select(Some(idx));
        self.connect_to_database();
    }

    /// Ping every saved connection concurrently; does not block the UI loop.
    pub fn check_connections_health(&mut self) {
        self.spawn_connections_health_check();
        self.set_status("Checking connections…");
    }

    fn spawn_connections_health_check(&mut self) {
        let connections = self.database_explorer.connections.original.clone();
        for connection in connections {
            let key = ping_key(&connection.name);
            if !self.in_flight.try_begin(key.clone()) {
                continue;
            }
            let tx = self.bg_tx.clone();
            let name = connection.name.clone();
            spawn_keyed(tx.clone(), key.clone(), async move {
                let up = ConnectionService::ping(&connection).await;
                let status = if up {
                    ConnectionStatus::Up
                } else {
                    ConnectionStatus::Down
                };
                let _ =
                    tx.send(BgEvent::ConnectionStatus { key, name, status });
            });
        }
    }

    /// Run the connection-list health probe when any connections are present.
    /// Used on launch and after the list is refreshed (edit/delete/Esc back).
    pub fn ping_connections_if_any(&mut self) {
        if self.database_explorer.connections.original.is_empty() {
            return;
        }
        self.spawn_connections_health_check();
    }

    /// Disconnect from the current database
    pub fn disconnect_from_database(&mut self) {
        self.database_explorer.state = DatabaseExplorerState::Connections;
        self.state = AppState::ConnectionList;

        self.hotkeys = CONNECTION_HOTKEYS.to_vec();
    }
}

async fn build_sqlite_database(
    query_log: QueryLog,
    connection: Connection,
) -> Result<(Arc<dyn Database>, Connection), String> {
    let db: Arc<dyn Database> =
        Arc::from(query_log.wrap(connection.to_sqlite()));
    if db.test().await {
        Ok((db, connection))
    } else {
        Err(format!(
            "Failed to connect to database: {}",
            connection.name
        ))
    }
}

async fn build_server_database(
    query_log: QueryLog,
    connection: Connection,
    password: String,
) -> Result<(Arc<dyn Database>, Connection), String> {
    let mut connection_with_password = connection.clone();
    connection_with_password.password = Some(password);

    let default_db = if connection.r#type == ConnectionType::Postgres {
        Some("postgres".to_string())
    } else {
        Some(parse_server_url(connection.r#type, &connection.url).3)
            .filter(|d| !d.is_empty())
    };

    let mut temp_connection = connection_with_password.clone();
    temp_connection.selected_database.clone_from(&default_db);
    let db: Arc<dyn Database> =
        Arc::from(query_log.wrap(temp_connection.to_database()));

    if db.test().await {
        connection_with_password.selected_database = default_db;
        Ok((db, connection_with_password))
    } else {
        Err(format!(
            "Failed to connect to database: {}",
            connection.name
        ))
    }
}

#[cfg(test)]
mod tests {
    use core::time::Duration;
    use std::{fs::write, time::Instant};

    use tokio::time::sleep;

    use super::*;
    use crate::{
        db::query_log::tests::temp_sqlite, filtered_data::FilteredData,
        in_flight::connection_key as conn_key,
    };

    fn sqlite_row(name: &str, path: &str) -> Connection {
        Connection {
            name: name.into(),
            r#type: ConnectionType::Sqlite,
            url: path.into(),
            ..Connection::default()
        }
    }

    async fn drain_connect(app: &mut App<'_>) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            app.drain_bg_events();
            if !app.in_flight.contains(&conn_key("c1")) {
                return;
            }
            sleep(Duration::from_millis(1)).await;
        }
    }

    #[tokio::test]
    async fn connect_registers_in_flight_without_awaiting() {
        let path = temp_sqlite("connect-inflight");
        write(&path.path, b"").unwrap();
        let mut app = App::default();
        app.database_explorer.connections =
            FilteredData::new(vec![sqlite_row("c1", &path.path)]);
        app.database_explorer
            .connections
            .table
            .view
            .state
            .select(Some(0));

        app.connect_to_database();
        assert!(app.in_flight.contains(&conn_key("c1")));
        assert_eq!(app.status_line.message(), "Connecting…");

        drain_connect(&mut app).await;
        assert_eq!(app.state, AppState::DatabaseConnected);
    }

    #[tokio::test]
    async fn superseded_connect_result_is_dropped() {
        let path = temp_sqlite("connect-superseded");
        write(&path.path, b"").unwrap();
        let mut app = App::default();
        app.spawn_connect(sqlite_row("c1", &path.path), None);
        // The user picked another connection before c1 finished.
        app.latest_connect_key = Some(conn_key("c2"));
        drain_connect(&mut app).await;
        assert!(!app.in_flight.contains(&conn_key("c1")));
        assert_eq!(app.state, AppState::ConnectionList);
    }

    #[tokio::test]
    async fn pending_health_ping_does_not_block_connect() {
        let path = temp_sqlite("connect-during-ping");
        write(&path.path, b"").unwrap();
        let mut app = App::default();
        app.database_explorer.connections =
            FilteredData::new(vec![sqlite_row("c1", &path.path)]);
        app.database_explorer
            .connections
            .table
            .view
            .state
            .select(Some(0));
        assert!(app.in_flight.try_begin(ping_key("c1")));

        app.connect_to_database();
        assert_eq!(app.status_line.message(), "Connecting…");
        drain_connect(&mut app).await;
        assert_eq!(app.state, AppState::DatabaseConnected);
    }

    #[tokio::test]
    async fn duplicate_connect_same_name_refused() {
        let path = temp_sqlite("connect-dup");
        write(&path.path, b"").unwrap();
        let mut app = App::default();
        app.database_explorer.connections =
            FilteredData::new(vec![sqlite_row("c1", &path.path)]);
        app.database_explorer
            .connections
            .table
            .view
            .state
            .select(Some(0));

        app.connect_to_database();
        app.connect_to_database();
        assert_eq!(app.status_line.message(), "Already connecting to c1");
        drain_connect(&mut app).await;
    }
}
