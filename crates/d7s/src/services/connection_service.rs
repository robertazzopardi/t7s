use core::time::Duration;

use color_eyre::{Result, eyre::eyre};
use tokio::{fs::metadata, net::TcpStream, time::timeout};

use crate::db::{
    connection::{Connection, ConnectionType, parse_server_url},
    sqlite::{
        delete_connection, get_connections, save_connection, update_connection,
    },
};

/// Service for managing database connections (CRUD operations)
pub struct ConnectionService;

/// Per-connection bound for the list health check.
const PING_TIMEOUT: Duration = Duration::from_secs(3);

impl ConnectionService {
    /// Get all connections from the database
    pub fn get_all() -> Result<Vec<Connection>> {
        get_connections()
    }

    /// Create a new connection
    pub fn create(connection: &Connection) -> Result<()> {
        save_connection(connection).map_err(|e| eyre!("{}", e))?;
        Ok(())
    }

    /// Update an existing connection (handles renames)
    pub fn update(old_name: &str, connection: &Connection) -> Result<()> {
        // update_connection handles renaming automatically via WHERE clause
        update_connection(old_name, connection).map_err(|e| eyre!("{}", e))?;
        Ok(())
    }

    /// Delete a connection by name
    pub fn delete(name: &str) -> Result<()> {
        delete_connection(name).map_err(|e| eyre!("{}", e))?;
        Ok(())
    }

    /// Validate a connection (check required fields are present)
    pub fn validate(connection: &Connection) -> Result<(), String> {
        if connection.name.trim().is_empty() {
            return Err("Connection name is required".to_string());
        }
        if connection.url.trim().is_empty() {
            return Err("Connection url is required".to_string());
        }
        Ok(())
    }

    /// Test a connection by attempting to connect (postgres or sqlite)
    pub async fn test(connection: &Connection) -> bool {
        connection.to_database().test().await
    }

    /// Credential-free reachability probe for the connection-list health
    /// check. Postgres: a bounded TCP connect to host:port (no login, so
    /// saved/prompted passwords don't matter and no failed-auth noise hits the
    /// server log). `SQLite`: the database file exists (opening would create it).
    pub async fn ping(connection: &Connection) -> bool {
        match connection.r#type {
            ConnectionType::Postgres | ConnectionType::Mysql => {
                let (host, port, _, _) =
                    parse_server_url(connection.r#type, &connection.url);
                let Ok(port) = port.parse::<u16>() else {
                    return false;
                };
                timeout(PING_TIMEOUT, TcpStream::connect((host.as_str(), port)))
                    .await
                    .is_ok_and(|r| r.is_ok())
            }
            ConnectionType::Sqlite => {
                metadata(&connection.url).await.is_ok_and(|m| m.is_file())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    fn pg(url: &str) -> Connection {
        Connection {
            r#type: ConnectionType::Postgres,
            url: url.into(),
            ..Connection::default()
        }
    }

    #[tokio::test]
    async fn ping_postgres_is_a_credential_free_tcp_connect() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(
            ConnectionService::ping(&pg(&format!(
                "postgres://nobody@127.0.0.1:{port}/db"
            )))
            .await
        );
        drop(listener);
        assert!(
            !ConnectionService::ping(&pg(&format!(
                "postgres://nobody@127.0.0.1:{port}/db"
            )))
            .await
        );
    }
}
