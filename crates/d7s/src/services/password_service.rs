use std::collections::HashMap;

use crate::{
    auth::Keyring,
    db::connection::{Connection, ConnectionType},
    error::Result,
};

/// Service for managing passwords across keyring and session storage
pub struct PasswordService {
    /// Session password storage (in-memory only, cleared when app exits)
    /// Key format: "{user}@{host}:{port}/{database}"
    session_passwords: HashMap<String, String>,
}

impl Default for PasswordService {
    fn default() -> Self {
        Self::new()
    }
}

impl PasswordService {
    /// Create a new password service
    pub fn new() -> Self {
        Self {
            session_passwords: HashMap::new(),
        }
    }

    /// Generate a unique key for a connection to use in session password storage
    fn connection_key(connection: &Connection) -> String {
        connection.name.clone()
    }

    // Keyring operations

    /// Get password from keyring for a connection
    pub fn get_from_keyring(connection_name: &str) -> Result<String> {
        let keyring = Keyring::new(connection_name)?;
        Ok(keyring.get_password()?)
    }

    /// Save password to keyring for a connection
    pub fn save_to_keyring(
        connection_name: &str,
        password: &str,
    ) -> Result<()> {
        let keyring = Keyring::new(connection_name)?;
        keyring.set_password(password)?;
        Ok(())
    }

    /// Delete password from keyring for a connection
    pub fn delete_from_keyring(connection_name: &str) -> Result<()> {
        let keyring = Keyring::new(connection_name)?;
        keyring.delete_password()?;
        Ok(())
    }

    // Session operations

    /// Get password from session storage for a connection
    pub fn get_session_password(
        &self,
        connection: &Connection,
    ) -> Option<&String> {
        let key = Self::connection_key(connection);
        self.session_passwords.get(&key)
    }

    /// Store password in session memory for a connection
    pub fn store_session_password(
        &mut self,
        connection: &Connection,
        password: String,
    ) {
        if connection.should_ask_every_time() {
            let key = Self::connection_key(connection);
            self.session_passwords.insert(key, password);
        }
    }

    /// Remove password from session storage for a connection
    pub fn remove_session_password(&mut self, connection: &Connection) {
        if connection.should_ask_every_time() {
            let key = Self::connection_key(connection);
            self.session_passwords.remove(&key);
        }
    }

    // High-level API

    /// Get password for a connection from the appropriate source
    /// Returns Some(password) if found in session or keyring, None if needs prompting.
    /// `SQLite` connections have no password; returns None so caller connects without password.
    pub fn get_password(&self, connection: &Connection) -> Option<String> {
        if connection.r#type == ConnectionType::Sqlite {
            return None;
        }
        if connection.should_ask_every_time() {
            self.get_session_password(connection).cloned()
        } else {
            Self::get_from_keyring(&connection.name).ok()
        }
    }

    /// Get password for connection, returning empty string if "ask every time" and not in session.
    /// `SQLite` connections have no password; returns empty string.
    pub fn get_connection_password(connection: &Connection) -> String {
        if connection.r#type == ConnectionType::Sqlite {
            return String::new();
        }
        if connection.should_ask_every_time() {
            String::new()
        } else {
            Self::get_from_keyring(&connection.name).unwrap_or_default()
        }
    }
}
