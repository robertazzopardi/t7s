use alloc::string::ToString;
use core::{
    fmt,
    fmt::{Display, Formatter},
    str::FromStr,
};
use std::env::var;

use ratatui::style::{Color, Style};
use serde::{Deserialize, Serialize};
use url::{Url, form_urlencoded::Serializer};

use crate::{
    db::{
        Database, TableData, mysql::Mysql, postgres::Postgres, sqlite::Sqlite,
        tls::TlsConfig,
    },
    ui::theme,
};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionType {
    #[default]
    Postgres,
    Sqlite,
    /// MySQL and MariaDB (same wire protocol).
    Mysql,
}

impl ConnectionType {
    /// `pg_stat_activity` has no equivalent elsewhere, so the activity view is Postgres-only.
    #[must_use]
    pub const fn has_activity_view(self) -> bool {
        matches!(self, Self::Postgres)
    }
}

impl Display for ConnectionType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postgres => write!(f, "postgres"),
            Self::Sqlite => write!(f, "sqlite"),
            Self::Mysql => write!(f, "mysql"),
        }
    }
}

impl FromStr for ConnectionType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_lowercase();
        Ok(match lower.as_str() {
            "postgres" | "postgresql" => Self::Postgres,
            "sqlite" => Self::Sqlite,
            "mysql" | "mariadb" => Self::Mysql,
            _ => return Err(()),
        })
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    #[default]
    Dev,
    Staging,
    Prod,
}

impl Display for Environment {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dev => write!(f, "dev"),
            Self::Staging => write!(f, "staging"),
            Self::Prod => write!(f, "prod"),
        }
    }
}

impl FromStr for Environment {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_lowercase();
        Ok(match lower.as_str() {
            "dev" => Self::Dev,
            "staging" => Self::Staging,
            "prod" => Self::Prod,
            _ => return Err(()),
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct Connection {
    /// Connection name
    pub name: String,
    /// postgres or sqlite
    pub r#type: ConnectionType,
    /// Full DSN (e.g. postgres://..., or path for sqlite)
    pub url: String,
    /// dev, staging, prod
    pub environment: Environment,
    /// Extra fields stored as JSON.
    pub metadata: serde_json::Value,
    /// Runtime UI state
    pub selected_database: Option<String>,
    /// Runtime UI state
    pub schema: Option<String>,
    pub table: Option<String>,
    /// Password (not persisted; from keyring or prompt)
    pub password: Option<String>,
    /// Where to store password: `keyring` or `dont_save`.
    pub password_storage: Option<String>,
    /// Runtime-only health status from the last `p` ping check (not persisted).
    pub status: ConnectionStatus,
}

/// Result of the `p` reachability check (TCP connect / file exists), not a login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionStatus {
    #[default]
    Unknown,
    Up,
    Down,
}

impl Display for ConnectionStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unknown => "",
            Self::Up => "● up",
            Self::Down => "● down",
        })
    }
}

impl Display for Connection {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.r#type {
            ConnectionType::Postgres | ConnectionType::Mysql => {
                let (host, port, user, database_from_url) =
                    parse_server_url(self.r#type, &self.url);
                let database = self
                    .selected_database
                    .as_deref()
                    .unwrap_or(&database_from_url);

                write!(
                    f,
                    " Name: {}\n Host: {}\n Port: {}\n User: {}\n TLS: {}\n Database: {}\n Schema: {}\n Table: {}",
                    self.name,
                    host,
                    port,
                    user,
                    TlsConfig::from_url(&self.url).mode,
                    database,
                    self.schema.clone().unwrap_or_default(),
                    self.table.clone().unwrap_or_default(),
                )
            }
            ConnectionType::Sqlite => {
                write!(
                    f,
                    " Name: {}\n Database: {}\n Table: {}",
                    self.name,
                    self.url,
                    self.table.clone().unwrap_or_default(),
                )
            }
        }
    }
}

impl TableData for Connection {
    fn title() -> &'static str {
        "Connection"
    }

    fn ref_array(&self) -> Vec<String> {
        vec![
            self.name.clone(),
            self.r#type.to_string(),
            truncate_display_url(
                &redact_password_in_url(self.url.as_str()),
                40,
            ),
            self.environment.to_string(),
            self.auth_badge().to_string(),
            self.status.to_string(),
        ]
    }

    fn num_columns(&self) -> usize {
        self.ref_array().len()
    }

    fn cols() -> Vec<&'static str> {
        vec!["Name", "Type", "Url", "Env", "Auth", "Status"]
    }

    fn cell_style(&self, column: usize) -> Option<Style> {
        if column == Self::ENV_COLUMN {
            Some(theme::env_style(self.environment))
        } else if column == Self::STATUS_COLUMN {
            match self.status {
                ConnectionStatus::Up => Some(Style::default().fg(Color::Green)),
                ConnectionStatus::Down => Some(Style::default().fg(Color::Red)),
                ConnectionStatus::Unknown => None,
            }
        } else {
            None
        }
    }
}

/// Redact password in a URL for display (e.g. <postgres://user:xxx@host/db>)
fn redact_password_in_url(url: &str) -> String {
    Url::parse(url).map_or_else(
        |_| url.to_string(),
        |mut parsed| {
            if parsed.password().is_some()
                && !parsed.password().unwrap_or_default().is_empty()
            {
                parsed
                    .set_password(Some("***"))
                    .map_or_else(|()| url.to_string(), |()| parsed.to_string())
            } else {
                url.to_string()
            }
        },
    )
}

pub fn shorten_home_path(path: &str) -> String {
    if let Ok(home) = var("HOME")
        && let Some(rest) = path.strip_prefix(home.as_str())
    {
        return format!("~{rest}");
    }
    path.to_string()
}

fn ellipsize_tail(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if max_chars == 0 {
        return String::new();
    }
    if count <= max_chars {
        return s.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let skip = count.saturating_sub(keep);
    format!("…{}", s.chars().skip(skip).collect::<String>())
}

fn truncate_display_url(url: &str, max_chars: usize) -> String {
    ellipsize_tail(&shorten_home_path(url), max_chars)
}

#[cfg(test)]
mod tests {
    use super::{
        Connection, ConnectionStatus, TableData, truncate_display_url,
    };

    #[test]
    fn truncate_keeps_url_tail() {
        let long = "/var/tmp/very/long/path/to/database/file/d7s.db";
        let out = truncate_display_url(long, 20);
        assert!(out.starts_with('…'));
        assert!(out.ends_with("d7s.db"));
        assert!(out.chars().count() <= 20);
    }

    #[test]
    fn mysql_urls_parse_with_mysql_defaults() {
        use super::{
            ConnectionType, build_server_url, parse_connection_string,
            parse_server_url,
        };
        let parsed =
            parse_connection_string("mariadb://bob@db.example:3307/shop")
                .unwrap();
        assert_eq!(parsed.connection_type, ConnectionType::Mysql);
        assert_eq!(
            parse_server_url(ConnectionType::Mysql, &parsed.url),
            (
                "db.example".into(),
                "3307".into(),
                "bob".into(),
                "shop".into()
            )
        );
        // No port / database: MySQL defaults (3306, no database selected).
        assert_eq!(
            parse_server_url(ConnectionType::Mysql, "mysql://bob@localhost"),
            (
                "localhost".into(),
                "3306".into(),
                "bob".into(),
                String::new()
            )
        );
        assert_eq!(
            build_server_url(ConnectionType::Mysql, "h", "3306", "u", ""),
            "mysql://u@h:3306/"
        );
    }

    #[test]
    fn status_labels_and_column() {
        assert_eq!(ConnectionStatus::Unknown.to_string(), "");
        assert_eq!(ConnectionStatus::Up.to_string(), "● up");
        assert_eq!(ConnectionStatus::Down.to_string(), "● down");

        let c = Connection {
            status: ConnectionStatus::Down,
            ..Connection::default()
        };
        assert_eq!(
            Connection::cols().get(Connection::STATUS_COLUMN),
            Some(&"Status")
        );
        assert_eq!(
            c.ref_array()
                .get(Connection::STATUS_COLUMN)
                .map(String::as_str),
            Some("● down")
        );
        assert!(c.cell_style(Connection::STATUS_COLUMN).is_some());
        assert!(
            Connection::default()
                .cell_style(Connection::STATUS_COLUMN)
                .is_none()
        );
    }
}

impl Connection {
    /// Convert this connection to a Postgres instance for testing/connecting.
    /// Parses `url` and uses `password` for authentication.
    /// Uses `selected_database` if set (when connected to a specific database), otherwise parses from URL.
    #[must_use]
    pub fn to_postgres(&self) -> Box<dyn Database> {
        let (host, port, user, database) =
            self.server_parts(ConnectionType::Postgres);
        Box::new(Postgres {
            name: self.name.clone(),
            host: Some(host),
            port: Some(port),
            user,
            database,
            password: self.password.clone().unwrap_or_default(),
            tls: TlsConfig::from_url(&self.url),
        })
    }

    /// (host, port, user, database) from the URL; `selected_database` wins over the URL's.
    fn server_parts(
        &self,
        connection_type: ConnectionType,
    ) -> (String, String, String, String) {
        let (host, port, user, database_from_url) =
            parse_server_url(connection_type, &self.url);
        let database =
            self.selected_database.clone().unwrap_or(database_from_url);
        (host, port, user, database)
    }

    #[must_use]
    pub fn to_mysql(&self) -> Box<dyn Database> {
        let (host, port, user, database) =
            self.server_parts(ConnectionType::Mysql);
        Box::new(Mysql {
            name: self.name.clone(),
            host: Some(host),
            port: Some(port),
            user,
            database,
            password: self.password.clone().unwrap_or_default(),
            tls: TlsConfig::from_url(&self.url),
        })
    }

    /// Backend for this connection's type.
    #[must_use]
    pub fn to_database(&self) -> Box<dyn Database> {
        match self.r#type {
            ConnectionType::Postgres => self.to_postgres(),
            ConnectionType::Sqlite => self.to_sqlite(),
            ConnectionType::Mysql => self.to_mysql(),
        }
    }

    #[must_use]
    pub fn to_sqlite(&self) -> Box<dyn Database> {
        Box::new(Sqlite {
            name: self.name.clone(),
            path: self.url.clone(),
        })
    }

    /// Check if this connection is configured to ask for password every time
    #[must_use]
    pub fn should_ask_every_time(&self) -> bool {
        self.password_storage
            .as_ref()
            .is_some_and(|s| s.eq_ignore_ascii_case("dont_save"))
    }

    /// Check if this connection is configured to use keyring storage
    #[must_use]
    pub fn uses_keyring(&self) -> bool {
        self.password_storage
            .as_ref()
            .is_some_and(|s| s.eq_ignore_ascii_case("keyring"))
    }

    /// Short auth label for the connections table (not raw metadata JSON).
    #[must_use]
    pub fn auth_badge(&self) -> &'static str {
        if self.r#type == ConnectionType::Sqlite {
            ""
        } else if self.should_ask_every_time() {
            "ask"
        } else if self.uses_keyring() {
            "keyring"
        } else {
            ""
        }
    }

    /// User part of the connection (for prompts). Parsed from URL for postgres.
    #[must_use]
    pub fn user_display(&self) -> String {
        if self.r#type != ConnectionType::Sqlite
            && let Ok(u) = Url::parse(&self.url)
        {
            return u.username().to_string();
        }
        self.name.clone()
    }

    /// k9s-style stacked "Label: value" lines for the top bar.
    #[must_use]
    pub fn summary_stack(&self) -> String {
        let mut lines = vec![format!("Connection: {}", self.name)];
        match self.r#type {
            ConnectionType::Postgres | ConnectionType::Mysql => {
                let (host, _, user, database_from_url) =
                    parse_server_url(self.r#type, &self.url);
                let database = self
                    .selected_database
                    .as_deref()
                    .unwrap_or(&database_from_url);
                lines.push(format!("Host: {host}"));
                lines.push(format!("User: {user}"));
                lines.push(format!(
                    "TLS: {}",
                    TlsConfig::from_url(&self.url).mode
                ));
                lines.push(format!("Database: {database}"));
                if let Some(schema) =
                    self.schema.as_deref().filter(|s| !s.is_empty())
                {
                    lines.push(format!("Schema: {schema}"));
                }
            }
            ConnectionType::Sqlite => {
                lines.push(format!("Path: {}", shorten_home_path(&self.url)));
            }
        }
        if let Some(table) = self.table.as_deref().filter(|t| !t.is_empty()) {
            lines.push(format!("Table: {table}"));
        }
        lines.push(format!("Env: {}", self.environment));
        lines.join("\n")
    }

    /// Environment column index in the connections table.
    pub const ENV_COLUMN: usize = 3;
    /// Status column index in the connections table.
    pub const STATUS_COLUMN: usize = 5;
}

/// Result of parsing a connection string. Used to prefill the connection form.
#[derive(Debug, Clone)]
pub struct ParsedConnection {
    pub connection_type: ConnectionType,
    /// Full URL (Postgres) or file path (`SQLite`).
    pub url: String,
}

/// Detect connection type from a string and return it with the URL/path.
/// Postgres: string starting with `postgres://` or `postgresql://`.
/// `SQLite`: anything else (file path or `sqlite:` URI).
#[must_use]
pub fn parse_connection_string(s: &str) -> Option<ParsedConnection> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_lowercase();
    let connection_type = if lower.starts_with("postgresql://")
        || lower.starts_with("postgres://")
    {
        ConnectionType::Postgres
    } else if lower.starts_with("mysql://") || lower.starts_with("mariadb://") {
        ConnectionType::Mysql
    } else {
        ConnectionType::Sqlite
    };
    Some(ParsedConnection {
        connection_type,
        url: s.to_string(),
    })
}

/// Build a Postgres URL from host, port, user, and database.
/// Password is intentionally excluded — it is stored only in the keyring.
#[must_use]
pub fn build_server_url(
    connection_type: ConnectionType,
    host: &str,
    port: &str,
    user: &str,
    database: &str,
) -> String {
    let auth = if user.is_empty() {
        String::new()
    } else {
        format!("{user}@")
    };
    format!("{connection_type}://{auth}{host}:{port}/{database}")
}

/// Append `sslmode=` (Postgres) / `ssl-mode=` (MySQL) and `sslrootcert=` when TLS is enabled.
#[must_use]
pub fn with_tls_query(
    connection_type: ConnectionType,
    url: &str,
    mode: &str,
    ca_path: &str,
) -> String {
    if mode.is_empty() || mode == "disable" {
        return url.to_string();
    }
    let key = if connection_type == ConnectionType::Mysql {
        "ssl-mode"
    } else {
        "sslmode"
    };
    let mut q = Serializer::new(String::new());
    q.append_pair(key, mode);
    if !ca_path.is_empty() {
        q.append_pair("sslrootcert", ca_path);
    }
    format!("{url}?{}", q.finish())
}

/// Default TCP port for a server-based connection type.
#[must_use]
pub const fn default_port(connection_type: ConnectionType) -> &'static str {
    match connection_type {
        ConnectionType::Mysql => "3306",
        ConnectionType::Postgres | ConnectionType::Sqlite => "5432",
    }
}

/// Default database for a new connection form (MySQL: none selected).
#[must_use]
pub const fn default_database(connection_type: ConnectionType) -> &'static str {
    match connection_type {
        ConnectionType::Mysql => "",
        ConnectionType::Postgres | ConnectionType::Sqlite => "postgres",
    }
}

/// Parse a postgres/mysql URL into (host, port, user, database), using the type's defaults.
pub fn parse_server_url(
    connection_type: ConnectionType,
    url_str: &str,
) -> (String, String, String, String) {
    let default_host = "localhost".to_string();
    let default_port = default_port(connection_type).to_string();
    let default_user = String::new();
    let default_db = default_database(connection_type).to_string();

    let Ok(url) = Url::parse(url_str) else {
        return (default_host, default_port, default_user, default_db);
    };
    let host = url.host_str().map_or(default_host, ToString::to_string);
    let port = url.port().map_or(default_port, |p| p.to_string());
    let user = url.username().to_string();
    let database = url
        .path()
        .strip_prefix('/')
        .map_or(default_db, ToString::to_string);
    (host, port, user, database)
}
