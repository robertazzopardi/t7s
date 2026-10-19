use std::env::var;

use rusqlite::{Connection as SqliteConnection, params};
use serde::{Deserialize, Serialize};

use crate::{
    db::get_db_path, error::Result, virtual_table::VIRTUAL_TABLE_PAGE_SIZE,
};

const KEY_RECENT_TABLES: &str = "recent_tables";
const KEY_SQL_HISTORY: &str = "sql_history";
const KEY_LAST_CONNECTION: &str = "last_connection";
const KEY_PAGE_SIZE: &str = "page_size";
const SQL_HISTORY_CAP: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RecentTableEntry {
    schema: String,
    table: String,
}

/// Thin key/value store over the same `SQLite` file as connections.
pub struct PreferencesService;

impl PreferencesService {
    pub fn get(key: &str) -> Result<Option<String>> {
        let conn = SqliteConnection::open(get_db_path()?)?;
        let mut stmt =
            conn.prepare("SELECT value FROM preferences WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            return Ok(row.get(0)?);
        }
        Ok(None)
    }

    pub fn set(key: &str, value: &str) -> Result<()> {
        let conn = SqliteConnection::open(get_db_path()?)?;
        conn.execute(
            "INSERT INTO preferences (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn load_recent_tables() -> Vec<(String, String)> {
        Self::get(KEY_RECENT_TABLES)
            .ok()
            .flatten()
            .and_then(|json| {
                serde_json::from_str::<Vec<RecentTableEntry>>(&json).ok()
            })
            .map(|entries| {
                entries.into_iter().map(|e| (e.schema, e.table)).collect()
            })
            .unwrap_or_default()
    }

    pub fn save_recent_tables(tables: &[(String, String)]) -> Result<()> {
        let entries: Vec<RecentTableEntry> = tables
            .iter()
            .take(5)
            .map(|(schema, table)| RecentTableEntry {
                schema: schema.clone(),
                table: table.clone(),
            })
            .collect();
        Self::set(KEY_RECENT_TABLES, &serde_json::to_string(&entries)?)
    }

    pub fn last_sql() -> Option<String> {
        Self::load_sql_history().into_iter().next()
    }

    pub fn load_sql_history() -> Vec<String> {
        Self::get(KEY_SQL_HISTORY)
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn push_sql_history(query: &str) -> Result<()> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let mut history = Self::load_sql_history();
        history.retain(|s| s != trimmed);
        history.insert(0, trimmed.to_string());
        history.truncate(SQL_HISTORY_CAP);
        Self::set(KEY_SQL_HISTORY, &serde_json::to_string(&history)?)
    }

    pub fn last_connection() -> Option<String> {
        Self::get(KEY_LAST_CONNECTION).ok().flatten()
    }

    pub fn set_last_connection(name: &str) -> Result<()> {
        Self::set(KEY_LAST_CONNECTION, name)
    }

    /// `D7S_PAGE_SIZE` env wins, then stored pref, then default 200.
    pub fn effective_page_size() -> u32 {
        if let Ok(env) = var("D7S_PAGE_SIZE")
            && let Ok(n) = env.parse::<u32>()
            && n > 0
        {
            return n;
        }
        Self::get(KEY_PAGE_SIZE)
            .ok()
            .flatten()
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(VIRTUAL_TABLE_PAGE_SIZE)
    }

    pub fn set_page_size(size: u32) -> Result<()> {
        Self::set(KEY_PAGE_SIZE, &size.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_tables_json_round_trip() {
        let tables = [
            ("public".to_string(), "users".to_string()),
            ("sqlite_schema".to_string(), "items".to_string()),
        ];
        let entries: Vec<RecentTableEntry> = tables
            .iter()
            .map(|(schema, table)| RecentTableEntry {
                schema: schema.clone(),
                table: table.clone(),
            })
            .collect();
        let json = serde_json::to_string(&entries).unwrap();
        let back: Vec<RecentTableEntry> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back.first().expect("entry").schema, "public");
        assert_eq!(back.get(1).expect("entry").table, "items");
    }
}
