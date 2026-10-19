use core::error::Error;
use std::fs::create_dir_all;

use directories::BaseDirs;
use k9tui::widgets::table::TableData;

pub mod connection;
pub mod mysql;
pub mod postgres;
pub mod query_log;
pub mod sqlite;
pub mod tls;

use std::path::PathBuf;

use color_eyre::{Result, eyre};

/// Stable-enough row locator for `UPDATE` when the table has no primary key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbRowId {
    Sqlite(i64),
    PostgresCtid(String),
}

/// Row locators for batch delete after user confirmation.
#[derive(Debug, Clone)]
pub struct RowDeleteSpec {
    pub primary_key: Vec<(String, String)>,
    pub row_id_fallback: Option<DbRowId>,
}

/// One page of table rows for the explorer, including optional per-row DB locators.
#[derive(Debug, Default)]
pub struct TableDataPage {
    pub rows: Vec<Vec<String>>,
    pub column_names: Vec<String>,
    pub row_ids: Vec<Option<DbRowId>>,
}

#[async_trait::async_trait]
pub trait Database: Send + Sync {
    async fn test(&self) -> bool;
    async fn execute_sql(
        &self,
        sql: &str,
    ) -> Result<Vec<TableRow>, Box<dyn Error>>;

    async fn get_schemas(&self) -> Result<Vec<Schema>, Box<dyn Error>>;

    async fn get_tables(
        &self,
        schema_name: &str,
    ) -> Result<Vec<Table>, Box<dyn Error>>;

    async fn get_columns(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<Column>, Box<dyn Error>>;

    /// Returns up to `limit` rows starting at `offset` (0-based), plus column names and row locators.
    async fn get_table_data_page(
        &self,
        schema_name: &str,
        table_name: &str,
        offset: u64,
        limit: u32,
    ) -> Result<TableDataPage, Box<dyn Error>>;

    /// Ordered primary-key column names (composite keys preserve order).
    async fn get_primary_key_columns(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<String>, Box<dyn Error>>;

    /// Update a single cell. Uses `primary_key` for the `WHERE` clause when non-empty; otherwise
    /// `row_id_fallback` (`rowid` / `ctid`) when present.
    async fn update_table_cell(
        &self,
        schema_name: &str,
        table_name: &str,
        set_column: &str,
        new_value: &str,
        primary_key: &[(String, String)],
        row_id_fallback: Option<DbRowId>,
    ) -> Result<u64, Box<dyn Error>>;

    /// Insert one row. `values` align with [`Database::get_columns`] order for this table.
    /// Empty string with a nullable column becomes SQL `NULL`. Same `WHERE` identity as update.
    async fn insert_table_row(
        &self,
        schema_name: &str,
        table_name: &str,
        values: &[String],
    ) -> Result<u64, Box<dyn Error>>;

    /// Delete one row; same `WHERE` construction as [`Database::update_table_cell`].
    async fn delete_table_row(
        &self,
        schema_name: &str,
        table_name: &str,
        primary_key: &[(String, String)],
        row_id_fallback: Option<DbRowId>,
    ) -> Result<u64, Box<dyn Error>>;

    /// Total row count for the table (for paging UI). May be expensive on huge tables.
    async fn get_table_row_count(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<u64, Box<dyn Error>>;

    /// Index names defined on the table, for the describe view. Cheap metadata lookup only.
    async fn get_table_index_names(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<String>, Box<dyn Error>>;

    /// On-disk size of the table, formatted for display, when cheaply available
    /// (e.g. Postgres `pg_total_relation_size`). `None` when not supported.
    async fn get_table_size(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Option<String>, Box<dyn Error>>;

    async fn get_databases(&self) -> Result<Vec<DatabaseInfo>, Box<dyn Error>>;
}

/// Database information
#[derive(Debug, Clone)]
pub struct DatabaseInfo {
    pub name: String,
}

/// Schema information
#[derive(Debug, Clone)]
pub struct Schema {
    pub name: String,
    pub owner: String,
}

/// `true` when a cell is empty/NULL in the grid and the column should be **omitted** from
/// `INSERT` so the server applies a default, identity, or `NULL` (nullable + no default).
///
/// `sqlite_rowid_pk_omit`: this column is the **sole** `INTEGER PRIMARY KEY` in `SQLite` (rowid).
#[must_use]
pub fn should_omit_for_insert_default(
    col: &Column,
    raw: &str,
    engine_sqlite: bool,
    sqlite_rowid_pk_omit: bool,
) -> bool {
    let trimmed = raw.trim();
    let empty = trimmed.is_empty();
    let explicit_null = !empty && trimmed.eq_ignore_ascii_case("null");
    if !empty && !explicit_null {
        // Prefill from `N` may leave the literal default in the cell; still omit so
        // identity/expression defaults are not sent as text.
        if literal_default_display(col).as_deref() == Some(trimmed) {
            return column_has_insert_default(
                col,
                engine_sqlite,
                sqlite_rowid_pk_omit,
            );
        }
        return false;
    }
    if explicit_null {
        return false;
    }
    column_has_insert_default(col, engine_sqlite, sqlite_rowid_pk_omit)
}

fn column_has_insert_default(
    col: &Column,
    engine_sqlite: bool,
    sqlite_rowid_pk_omit: bool,
) -> bool {
    if let Some(d) = &col.default_value {
        let t = d.trim();
        if !t.is_empty() && t.to_uppercase() != "NULL" {
            return true;
        }
    }
    if engine_sqlite && sqlite_rowid_pk_omit {
        return true;
    }
    if !engine_sqlite {
        let u = col.data_type.to_lowercase();
        if u.contains("serial") || u.contains("identity") {
            return true;
        }
        if col
            .default_value
            .as_deref()
            .is_some_and(|d| d.contains("nextval"))
        {
            return true;
        }
    }
    false
}

/// Blank / `"null"` on a nullable column should write SQL `NULL` (not `''` / `0`).
#[must_use]
pub fn cell_clears_to_sql_null(new_value: &str, column_nullable: bool) -> bool {
    let t = new_value.trim();
    column_nullable && (t.is_empty() || t.eq_ignore_ascii_case("null"))
}

/// Unquoted literal from a schema DEFAULT for grid prefill (`N`), if simple enough.
///
/// Expression defaults (`CURRENT_TIMESTAMP`, `nextval`, …) return `None` so the cell
/// stays empty and [`should_omit_for_insert_default`] still omits on INSERT.
#[must_use]
pub fn literal_default_display(col: &Column) -> Option<String> {
    let raw = col.default_value.as_deref()?.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("NULL") {
        return None;
    }
    if is_generated_default(raw) {
        return None;
    }
    // Postgres often appends `::typename`.
    let d = raw.split("::").next().unwrap_or(raw).trim();
    if d.len() >= 2 && d.starts_with('\'') && d.ends_with('\'') {
        return Some(d[1..d.len() - 1].replace("''", "'"));
    }
    if d.eq_ignore_ascii_case("true") || d.eq_ignore_ascii_case("false") {
        return Some(d.to_string());
    }
    if d.parse::<i64>().is_ok() || d.parse::<f64>().is_ok() {
        return Some(d.to_string());
    }
    None
}

/// Cell value for a new `N` (defaults) draft row.
///
/// Literal defaults are shown; expression/generated defaults stay empty so
/// INSERT omits the column. Only bare nullable columns (no DEFAULT) show `NULL`.
#[must_use]
pub fn draft_cell_from_column_default(col: &Column) -> String {
    if let Some(v) = literal_default_display(col) {
        return v;
    }
    if col.default_value.as_deref().is_some_and(|d| {
        let t = d.trim();
        !t.is_empty() && !t.eq_ignore_ascii_case("NULL")
    }) {
        // Expression / identity DEFAULT — leave empty for omit-on-insert.
        return String::new();
    }
    if col.is_nullable {
        return "NULL".to_string();
    }
    String::new()
}

/// Table information
#[derive(Debug, Clone)]
pub struct Table {
    pub name: String,
    pub schema: String,
    pub size: Option<String>,
}

/// Column information
#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub default_value: Option<String>,
    pub description: Option<String>,
}

/// `SQLite` sole `INTEGER PRIMARY KEY` column (rowid alias) omitted on INSERT when empty.
#[must_use]
pub fn sqlite_rowid_pk_omit(col: &Column, pk_first: Option<&str>) -> bool {
    pk_first.is_some_and(|first_pk| {
        col.name == first_pk || col.name.eq_ignore_ascii_case(first_pk)
    }) && col.data_type.to_lowercase().contains("int")
}

/// The PK column name when the primary key is a single column.
#[must_use]
pub const fn sole_pk(pk_names: &[String]) -> Option<&str> {
    match pk_names {
        [only] => Some(only.as_str()),
        _ => None,
    }
}

/// Default expressions meaning the DB generates the value (serial, PG identity, MySQL `auto_increment`).
fn is_generated_default(d: &str) -> bool {
    let d = d.to_lowercase();
    d.contains("nextval") || d.contains("identity") || d == "auto_increment"
}

/// Generated / identity / rowid PK columns are not edited in the grid.
#[must_use]
pub fn column_is_editable(col: &Column, is_sqlite_rowid_pk: bool) -> bool {
    if is_sqlite_rowid_pk {
        return false;
    }
    let u = col.data_type.to_lowercase();
    if u.contains("serial") || u.contains("identity") {
        return false;
    }
    !col.default_value
        .as_deref()
        .is_some_and(is_generated_default)
}

/// Table row data
#[derive(Debug, Clone)]
pub struct TableRow {
    pub values: Vec<String>,
    pub column_names: Vec<String>,
}

impl TableData for DatabaseInfo {
    fn title() -> &'static str {
        "Databases"
    }

    fn ref_array(&self) -> Vec<String> {
        vec![self.name.clone()]
    }

    fn num_columns(&self) -> usize {
        self.ref_array().len()
    }

    fn cols() -> Vec<&'static str> {
        vec!["Name"]
    }
}

impl TableData for Schema {
    fn title() -> &'static str {
        "Schemas"
    }

    fn ref_array(&self) -> Vec<String> {
        vec![self.name.clone(), self.owner.clone()]
    }

    fn num_columns(&self) -> usize {
        self.ref_array().len()
    }

    fn cols() -> Vec<&'static str> {
        vec!["Name", "Owner", "Description"]
    }
}

impl TableData for Table {
    fn title() -> &'static str {
        "Tables"
    }

    fn ref_array(&self) -> Vec<String> {
        vec![
            self.name.clone(),
            self.schema.clone(),
            self.size.clone().unwrap_or_default(),
        ]
    }

    fn num_columns(&self) -> usize {
        self.ref_array().len()
    }

    fn cols() -> Vec<&'static str> {
        vec!["Name", "Schema", "Size"]
    }
}

impl TableData for Column {
    fn title() -> &'static str {
        "Columns"
    }

    fn ref_array(&self) -> Vec<String> {
        vec![
            self.name.clone(),
            self.data_type.clone(),
            if self.is_nullable {
                "YES".to_string()
            } else {
                "NO".to_string()
            },
            self.default_value.clone().unwrap_or_default(),
            self.description.clone().unwrap_or_default(),
        ]
    }

    fn num_columns(&self) -> usize {
        self.ref_array().len()
    }

    fn cols() -> Vec<&'static str> {
        vec!["Name", "Type", "Nullable", "Default", "Description"]
    }
}

impl TableData for TableRow {
    fn title() -> &'static str {
        "Table Data"
    }

    fn ref_array(&self) -> Vec<String> {
        self.values.clone()
    }

    fn num_columns(&self) -> usize {
        self.values.len()
    }

    fn cols() -> Vec<&'static str> {
        // This will be dynamically set based on the actual columns
        vec![]
    }
}

pub fn get_app_data_dir() -> Result<PathBuf> {
    let Some(path) = BaseDirs::new() else {
        return Err(eyre::eyre!(
            "Unable to find data directory for ratatui-template"
        ));
    };

    let mut path = PathBuf::from(path.data_dir());

    path.push("d7s");

    // Create directory if it doesn't exist
    create_dir_all(&path)?;

    Ok(path)
}

pub fn get_db_path() -> Result<PathBuf> {
    let mut path = get_app_data_dir()?;
    path.push("d7s.db");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, nullable: bool, default: Option<&str>) -> Column {
        Column {
            name: name.into(),
            data_type: "TEXT".into(),
            is_nullable: nullable,
            default_value: default.map(str::to_string),
            description: None,
        }
    }

    #[test]
    fn cell_clears_to_sql_null_only_when_nullable() {
        assert!(cell_clears_to_sql_null("", true));
        assert!(cell_clears_to_sql_null("NULL", true));
        assert!(cell_clears_to_sql_null(" null ", true));
        assert!(!cell_clears_to_sql_null("", false));
        assert!(!cell_clears_to_sql_null("x", true));
    }

    #[test]
    fn literal_default_display_unwraps_simple_defaults() {
        assert_eq!(
            literal_default_display(&col("n", true, Some("'x'"))).as_deref(),
            Some("x")
        );
        assert_eq!(
            literal_default_display(&col("n", true, Some("'x'::text")))
                .as_deref(),
            Some("x")
        );
        assert_eq!(
            literal_default_display(&col("n", true, Some("0"))).as_deref(),
            Some("0")
        );
        assert!(
            literal_default_display(&col("n", true, Some("CURRENT_TIMESTAMP")))
                .is_none()
        );
        assert_eq!(
            draft_cell_from_column_default(&col("n", true, None)),
            "NULL"
        );
        assert_eq!(draft_cell_from_column_default(&col("n", false, None)), "");
        // Nullable + expression DEFAULT must stay empty (not "NULL") so INSERT omits.
        let ts = col("ts", true, Some("CURRENT_TIMESTAMP"));
        assert_eq!(draft_cell_from_column_default(&ts), "");
        assert!(should_omit_for_insert_default(&ts, "", false, false));
        assert!(
            !should_omit_for_insert_default(&ts, "NULL", false, false),
            "explicit NULL must not omit"
        );
    }

    #[test]
    fn omit_when_cell_still_shows_literal_default() {
        let c = col("n", false, Some("'x'"));
        assert!(should_omit_for_insert_default(&c, "x", false, false));
        assert!(!should_omit_for_insert_default(&c, "y", false, false));
    }
}
