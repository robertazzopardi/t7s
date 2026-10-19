use core::{error::Error, fmt::Write};
use std::collections::HashMap;

use color_eyre::Result;
use rusqlite::{Connection as SqliteConnection, params, types};
use rusqlite_migration::{M, Migrations};
use serde_json::{Map, Value as SerdeJsonValue};

use crate::db::{
    Column, Database, DatabaseInfo, DbRowId, Schema, Table, TableData,
    TableDataPage, TableRow, cell_clears_to_sql_null,
    connection::{Connection, ConnectionStatus, ConnectionType, Environment},
    get_db_path,
    query_log::report_sql,
    should_omit_for_insert_default, sole_pk, sqlite_rowid_pk_omit,
};

/// `prepare` that reports the SQL text (placeholders only) to the query log.
fn prep<'c>(
    conn: &'c SqliteConnection,
    sql: &str,
) -> rusqlite::Result<rusqlite::Statement<'c>> {
    report_sql(sql);
    conn.prepare(sql)
}

/// `execute` that reports the SQL text (placeholders only) to the query log.
fn exec<P: rusqlite::Params>(
    conn: &SqliteConnection,
    sql: &str,
    params: P,
) -> rusqlite::Result<usize> {
    report_sql(sql);
    conn.execute(sql, params)
}

fn sqlite_quote_ident(ident: &str) -> String {
    format!(r#""{}""#, ident.replace('"', "\"\""))
}

/// Build `PRAGMA <pragma>('<table>')` with `'` escaped in the table name.
fn pragma_for_table(pragma: &str, table_name: &str) -> String {
    format!("PRAGMA {pragma}('{}')", table_name.replace('\'', "''"))
}

fn sqlite_table_decltypes(
    conn: &SqliteConnection,
    table_name: &str,
) -> Result<HashMap<String, String>, rusqlite::Error> {
    let mut stmt = prep(conn, &pragma_for_table("table_info", table_name))?;
    let mut m = HashMap::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        let decl: String = row.get::<_, Option<String>>(2)?.unwrap_or_default();
        m.insert(name, decl);
    }
    Ok(m)
}

fn sqlite_resolve_decl<'a>(
    decls: &'a HashMap<String, String>,
    col: &str,
) -> &'a str {
    decls
        .get(col)
        .map(String::as_str)
        .or_else(|| {
            decls
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(col))
                .map(|(_, v)| v.as_str())
        })
        .unwrap_or("")
}

/// Target for `SQLite` [`CAST`](https://www.sqlite.org/lang_expr.html#castexpr): `INTEGER`, `REAL`, `TEXT`, or `BLOB`.
fn sqlite_cast_keyword(decl: &str) -> &'static str {
    let d = decl.trim();
    if d.is_empty() {
        return "TEXT";
    }
    let u = d.to_ascii_uppercase();
    if u.contains("INT") || u == "BOOLEAN" || u == "BOOL" {
        return "INTEGER";
    }
    if u.contains("BLOB") {
        return "BLOB";
    }
    if u.contains("REAL")
        || u.contains("FLOA")
        || u.contains("DOUB")
        || u.contains("NUM")
        || u.contains("DEC")
    {
        return "REAL";
    }
    if u.contains("CHAR") || u.contains("CLOB") || u.contains("TEXT") {
        return "TEXT";
    }
    "TEXT"
}

pub struct Sqlite {
    pub name: String,
    pub path: String,
}

impl Sqlite {
    fn open_conn(&self) -> Result<SqliteConnection, Box<dyn Error>> {
        Ok(SqliteConnection::open(&self.path)?)
    }
}

impl TableData for Sqlite {
    fn title() -> &'static str {
        "Sqlite"
    }

    fn ref_array(&self) -> Vec<String> {
        vec![self.name.clone(), self.path.clone()]
    }

    fn num_columns(&self) -> usize {
        self.ref_array().len()
    }

    fn cols() -> Vec<&'static str> {
        vec!["Name", "Path"]
    }
}

#[async_trait::async_trait]
impl Database for Sqlite {
    async fn test(&self) -> bool {
        self.open_conn().is_ok()
    }

    async fn execute_sql(
        &self,
        sql: &str,
    ) -> Result<Vec<TableRow>, Box<dyn Error>> {
        // rusqlite is synchronous, so we just run it in the async context
        let client = self.open_conn()?;

        let mut stmt = client.prepare(sql)?;

        let column_names: Vec<String> = stmt
            .column_names()
            .iter()
            .map(|s| (*s).to_string())
            .collect();

        // Statements without result columns (INSERT/UPDATE/DELETE/DDL) are
        // executed once on the prepared statement; never re-run.
        if stmt.column_count() == 0 {
            let affected_rows = stmt.execute([])?;
            return Ok(vec![TableRow {
                values: vec![format!("Affected rows: {affected_rows}")],
                column_names: vec!["Result".to_string()],
            }]);
        }

        let mut result = Vec::new();
        let mut rows_iter = stmt.query([])?;
        while let Some(row) = rows_iter.next()? {
            let mut values = Vec::new();
            for i in 0..column_names.len() {
                values.push(convert_sqlite_value_to_string(row, i));
            }
            result.push(TableRow {
                values,
                column_names: column_names.clone(),
            });
        }

        Ok(result)
    }

    async fn get_schemas(&self) -> Result<Vec<Schema>, Box<dyn Error>> {
        Ok(vec![Schema {
            name: "sqlite_schema".to_string(),
            owner: String::new(),
        }])
    }

    async fn get_tables(
        &self,
        schema_name: &str,
    ) -> Result<Vec<Table>, Box<dyn Error>> {
        let conn = self.open_conn()?;

        let mut sizes_by_name: HashMap<String, u64> = HashMap::new();
        let mut stat_stmt = prep(
            &conn,
            r#"SELECT name, SUM("pgsize") FROM "dbstat" GROUP BY name"#,
        )?;
        let mut stat_rows = stat_stmt.query([])?;
        while let Some(row) = stat_rows.next()? {
            let name: String = row.get(0)?;
            let bytes: i64 = row.get::<_, Option<i64>>(1)?.unwrap_or(0);
            sizes_by_name.insert(name, bytes.cast_unsigned());
        }

        let mut stmt = prep(
            &conn,
            &format!("SELECT name FROM {schema_name} WHERE type='table';"),
        )?;
        let tables = stmt
            .query_map([], |row| {
                let name: String = row.get(0)?;
                let size = sizes_by_name.get(&name).copied().unwrap_or(0);
                Ok(Table {
                    name,
                    schema: schema_name.to_string(),
                    size: Some(size.to_string()),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(tables)
    }

    async fn get_columns(
        &self,
        _schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<Column>, Box<dyn Error>> {
        let conn = self.open_conn()?;

        let mut stmt =
            prep(&conn, &pragma_for_table("table_info", table_name))?;
        let columns = stmt
            .query_map([], |row| {
                let name: String = row.get(1)?;
                let data_type: String = row.get(2)?;
                let notnull: i64 = row.get(3)?;
                let default_value: Option<String> = row.get(4)?;
                Ok(Column {
                    name,
                    data_type,
                    is_nullable: notnull == 0,
                    default_value,
                    description: None,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(columns)
    }

    async fn get_table_data_page(
        &self,
        schema_name: &str,
        table_name: &str,
        offset: u64,
        limit: u32,
    ) -> Result<TableDataPage, Box<dyn Error>> {
        let columns: Vec<String> = self
            .get_columns(schema_name, table_name)
            .await?
            .into_iter()
            .map(|col| col.name)
            .collect();

        let conn = self.open_conn()?;

        let column_count = columns.len();
        let limit_i = i64::from(limit);
        let offset_i = i64::try_from(offset).unwrap_or(i64::MAX);
        let tq = sqlite_quote_ident(table_name);
        let col_list = columns
            .iter()
            .map(|c| sqlite_quote_ident(c))
            .collect::<Vec<_>>()
            .join(", ");
        let query_rowid =
            format!("SELECT rowid, {col_list} FROM {tq} LIMIT ?1 OFFSET ?2");
        let (data, row_ids) = if let Ok(mut stmt) = prep(&conn, &query_rowid) {
            let mut row_ids = Vec::new();
            let data = stmt
                .query_map(params![limit_i, offset_i], |row| {
                    let rid = row.get::<_, i64>(0)?;
                    row_ids.push(Some(DbRowId::Sqlite(rid)));
                    let values = (0..column_count)
                        .map(|i| convert_sqlite_value_to_string(row, i + 1))
                        .collect();
                    Ok(values)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            (data, row_ids)
        } else {
            let mut stmt = prep(
                &conn,
                &format!("SELECT {col_list} FROM {tq} LIMIT ?1 OFFSET ?2"),
            )?;
            let data = stmt
                .query_map(params![limit_i, offset_i], |row| {
                    let values = (0..column_count)
                        .map(|i| convert_sqlite_value_to_string(row, i))
                        .collect();
                    Ok(values)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let row_ids = vec![None; data.len()];
            (data, row_ids)
        };

        Ok(TableDataPage {
            rows: data,
            column_names: columns,
            row_ids,
        })
    }

    async fn get_primary_key_columns(
        &self,
        _schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<String>, Box<dyn Error>> {
        let conn = self.open_conn()?;
        let mut stmt =
            prep(&conn, &pragma_for_table("table_info", table_name))?;
        let mut pk_cols: Vec<(i64, String)> = stmt
            .query_map([], |row| {
                let name: String = row.get(1)?;
                let pk: i64 = row.get(5)?;
                Ok((pk, name))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        pk_cols.retain(|(pk, _)| *pk > 0);
        pk_cols.sort_by_key(|(pk, _)| *pk);
        Ok(pk_cols.into_iter().map(|(_, n)| n).collect())
    }

    async fn update_table_cell(
        &self,
        _schema_name: &str,
        table_name: &str,
        set_column: &str,
        new_value: &str,
        primary_key: &[(String, String)],
        row_id_fallback: Option<DbRowId>,
    ) -> Result<u64, Box<dyn Error>> {
        let conn = self.open_conn()?;
        let decls = sqlite_table_decltypes(&conn, table_name)?;
        let set_kw =
            sqlite_cast_keyword(sqlite_resolve_decl(&decls, set_column));
        let tq = sqlite_quote_ident(table_name);
        let cq = sqlite_quote_ident(set_column);
        let nullable = self
            .get_columns("sqlite_schema", table_name)
            .await?
            .iter()
            .any(|c| c.name == set_column && c.is_nullable);
        let write_null = cell_clears_to_sql_null(new_value, nullable);

        if !primary_key.is_empty() {
            let mut sql = if write_null {
                format!("UPDATE {tq} SET {cq} = NULL WHERE ")
            } else {
                format!("UPDATE {tq} SET {cq} = CAST(?1 AS {set_kw}) WHERE ")
            };
            let param_base = if write_null { 1 } else { 2 };
            for (i, (k, _)) in primary_key.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let param_num = i + param_base;
                let pk_kw = sqlite_cast_keyword(sqlite_resolve_decl(&decls, k));
                let _ = write!(
                    sql,
                    "{} = CAST(?{param_num} AS {pk_kw})",
                    sqlite_quote_ident(k)
                );
            }
            let mut refs: Vec<&dyn rusqlite::ToSql> = Vec::new();
            if !write_null {
                refs.push(&new_value);
            }
            for (_, v) in primary_key {
                refs.push(v);
            }
            let n =
                u64::try_from(exec(&conn, &sql, refs.as_slice())?).unwrap_or(0);
            return Ok(n);
        }

        if let Some(DbRowId::Sqlite(rid)) = row_id_fallback {
            let n = if write_null {
                let sql =
                    format!("UPDATE {tq} SET {cq} = NULL WHERE rowid = ?1");
                u64::try_from(exec(&conn, &sql, params![rid])?).unwrap_or(0)
            } else {
                let sql = format!(
                    "UPDATE {tq} SET {cq} = CAST(?1 AS {set_kw}) WHERE rowid = ?2"
                );
                u64::try_from(exec(&conn, &sql, params![new_value, rid])?)
                    .unwrap_or(0)
            };
            return Ok(n);
        }

        Err(
            "Cannot update row: table has no usable primary key and rowid is not available"
                .into(),
        )
    }

    async fn insert_table_row(
        &self,
        _schema_name: &str,
        table_name: &str,
        values: &[String],
    ) -> Result<u64, Box<dyn Error>> {
        let columns = self.get_columns("sqlite_schema", table_name).await?;
        if values.len() != columns.len() {
            return Err("INSERT column count does not match table.".into());
        }
        let pk = self
            .get_primary_key_columns("sqlite_schema", table_name)
            .await
            .unwrap_or_default();
        let conn = self.open_conn()?;
        let decls = sqlite_table_decltypes(&conn, table_name)?;
        let tq = sqlite_quote_ident(table_name);
        let mut col_list: Vec<String> = Vec::new();
        let mut val_parts: Vec<String> = Vec::new();
        let mut refs: Vec<types::Value> = Vec::new();
        for (i, c) in columns.iter().enumerate() {
            let raw = values.get(i).map_or("", String::as_str);
            let sqlite_rowid_pk = sqlite_rowid_pk_omit(c, sole_pk(&pk));
            if should_omit_for_insert_default(c, raw, true, sqlite_rowid_pk) {
                continue;
            }
            if raw.trim().is_empty() || raw.eq_ignore_ascii_case("null") {
                if c.is_nullable {
                    col_list.push(sqlite_quote_ident(&c.name));
                    val_parts.push("NULL".to_string());
                } else {
                    return Err(format!(
                        "Column '{}' is NOT NULL and has no value or default in the form.",
                        c.name
                    )
                    .into());
                }
            } else {
                col_list.push(sqlite_quote_ident(&c.name));
                let kw =
                    sqlite_cast_keyword(sqlite_resolve_decl(&decls, &c.name));
                val_parts.push(format!("CAST(? AS {kw})"));
                refs.push(types::Value::Text(raw.to_string()));
            }
        }
        if col_list.is_empty() {
            let sql = format!("INSERT INTO {tq} DEFAULT VALUES");
            return Ok(u64::try_from(exec(&conn, &sql, [])?).unwrap_or(0));
        }
        let col_list = col_list.join(", ");
        let sql = format!(
            "INSERT INTO {tq} ({col_list}) VALUES ({})",
            val_parts.join(", ")
        );
        let mut pvec: Vec<&dyn rusqlite::ToSql> = Vec::new();
        for v in &refs {
            pvec.push(v);
        }
        let n = u64::try_from(exec(&conn, &sql, pvec.as_slice())?).unwrap_or(0);
        Ok(n)
    }

    async fn delete_table_row(
        &self,
        _schema_name: &str,
        table_name: &str,
        primary_key: &[(String, String)],
        row_id_fallback: Option<DbRowId>,
    ) -> Result<u64, Box<dyn Error>> {
        let conn = self.open_conn()?;
        let decls = sqlite_table_decltypes(&conn, table_name)?;
        let tq = sqlite_quote_ident(table_name);
        if !primary_key.is_empty() {
            let mut sql = format!("DELETE FROM {tq} WHERE ");
            for (i, (k, _)) in primary_key.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let param_num = i + 1;
                let pk_kw = sqlite_cast_keyword(sqlite_resolve_decl(&decls, k));
                let _ = write!(
                    sql,
                    "{} = CAST(?{param_num} AS {pk_kw})",
                    sqlite_quote_ident(k)
                );
            }
            let mut refs: Vec<&dyn rusqlite::ToSql> = Vec::new();
            for (_, v) in primary_key {
                refs.push(v);
            }
            let n =
                u64::try_from(exec(&conn, &sql, refs.as_slice())?).unwrap_or(0);
            return Ok(n);
        }
        if let Some(DbRowId::Sqlite(rid)) = row_id_fallback {
            let sql = format!("DELETE FROM {tq} WHERE rowid = ?1");
            let n =
                u64::try_from(exec(&conn, &sql, params![rid])?).unwrap_or(0);
            return Ok(n);
        }
        Err("Cannot delete row: no primary key and no rowid".into())
    }

    async fn get_table_row_count(
        &self,
        _schema_name: &str,
        table_name: &str,
    ) -> Result<u64, Box<dyn Error>> {
        let conn = self.open_conn()?;
        let sql = format!("SELECT COUNT(*) FROM {table_name}");
        report_sql(&sql);
        let count: i64 = conn.query_row(&sql, [], |row| row.get(0))?;
        Ok(count.cast_unsigned())
    }

    async fn get_table_index_names(
        &self,
        _schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<String>, Box<dyn Error>> {
        let conn = self.open_conn()?;
        let mut stmt =
            prep(&conn, &pragma_for_table("index_list", table_name))?;
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(names)
    }

    async fn get_table_size(
        &self,
        _schema_name: &str,
        _table_name: &str,
    ) -> Result<Option<String>, Box<dyn Error>> {
        // No cheap per-table size query in SQLite; skip rather than fabricate.
        Ok(None)
    }

    async fn get_databases(&self) -> Result<Vec<DatabaseInfo>, Box<dyn Error>> {
        // SQLite doesn't have multiple databases per connection
        // Return a single database with the path as the name
        Ok(vec![DatabaseInfo {
            name: self.path.clone(),
        }])
    }
}

/// Initialize the database with migrations.
///
/// Base schema: Name, Type, Url, Environment, Metadata (JSONB stored as TEXT).
///
/// # Errors
///
/// This function will return an error if the database cannot be opened or if migrations fail.
pub fn init_db() -> Result<()> {
    let db_path = get_db_path()?;
    let mut conn = SqliteConnection::open(db_path)?;

    migrations().to_latest(&mut conn)?;

    Ok(())
}

/// Append-only: never edit an applied migration, add a new one at the end.
fn migrations() -> Migrations<'static> {
    // Base schema: Name, Type, Url, Environment, Metadata (JSONB as TEXT).
    Migrations::new(vec![
        M::up(
            "CREATE TABLE IF NOT EXISTS connections (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                type TEXT NOT NULL CHECK( type IN ('postgres','sqlite') ),
                url TEXT NOT NULL,
                environment TEXT NOT NULL CHECK( environment IN ('local', 'dev','staging','prod') ),
                metadata TEXT
            );",
        )
        .down("DROP TABLE connections"),
        M::up(
            "CREATE TABLE IF NOT EXISTS preferences (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );",
        )
        .down("DROP TABLE preferences"),
        // Allow type 'mysql': SQLite can't alter a CHECK, so rebuild the table (ids kept).
        M::up(
            "CREATE TABLE connections_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                type TEXT NOT NULL CHECK( type IN ('postgres','sqlite','mysql') ),
                url TEXT NOT NULL,
                environment TEXT NOT NULL CHECK( environment IN ('local', 'dev','staging','prod') ),
                metadata TEXT
            );
            INSERT INTO connections_new SELECT id, name, type, url, environment, metadata FROM connections;
            DROP TABLE connections;
            ALTER TABLE connections_new RENAME TO connections;",
        )
        .down(
            "CREATE TABLE connections_old (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                type TEXT NOT NULL CHECK( type IN ('postgres','sqlite') ),
                url TEXT NOT NULL,
                environment TEXT NOT NULL CHECK( environment IN ('local', 'dev','staging','prod') ),
                metadata TEXT
            );
            INSERT INTO connections_old SELECT id, name, type, url, environment, metadata FROM connections WHERE type != 'mysql';
            DROP TABLE connections;
            ALTER TABLE connections_old RENAME TO connections;",
        ),
    ])
}

/// Build metadata JSON for storage (includes `password_storage` if set).
fn metadata_for_save(connection: &Connection) -> String {
    let mut obj = match &connection.metadata {
        SerdeJsonValue::Object(m) => m.clone(),
        SerdeJsonValue::Null
        | SerdeJsonValue::Bool(_)
        | SerdeJsonValue::Number(_)
        | SerdeJsonValue::String(_)
        | SerdeJsonValue::Array(_) => Map::new(),
    };
    if let Some(ps) = &connection.password_storage {
        obj.insert(
            "password_storage".to_string(),
            SerdeJsonValue::String(ps.clone()),
        );
    }
    SerdeJsonValue::Object(obj).to_string()
}

/// Parse metadata from DB and extract `password_storage`.
fn metadata_from_row(
    metadata_json: Option<&String>,
) -> (serde_json::Value, Option<String>) {
    let mut password_storage = None;
    let value = metadata_json
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| SerdeJsonValue::Object(Map::new()));

    if let Some(obj) = value.as_object()
        && let Some(ps) = obj.get("password_storage").and_then(|v| v.as_str())
    {
        password_storage = Some(ps.to_string());
    }
    (value, password_storage)
}

/// Save a connection to the database
///
/// # Errors
///
/// This function will return an error if the database cannot be opened or if the query fails.
pub fn save_connection(connection: &Connection) -> Result<(), Box<dyn Error>> {
    let db_path = get_db_path()?;
    let conn = SqliteConnection::open(db_path)?;

    let metadata = metadata_for_save(connection);

    conn.execute(
        "INSERT INTO connections (name, type, url, environment, metadata) VALUES (?, ?, ?, ?, ?)",
        params![
            connection.name,
            connection.r#type.to_string(),
            connection.url,
            connection.environment.to_string(),
            metadata,
        ],
    )?;

    Ok(())
}

/// Get all connections from the database
///
/// # Errors
///
/// This function will return an error if the database cannot be opened or if the query fails.
pub fn get_connections() -> Result<Vec<Connection>> {
    let db_path = get_db_path()?;
    let conn = SqliteConnection::open(db_path)?;

    let mut stmt = conn.prepare(
        "SELECT name, type, url, environment, metadata FROM connections ORDER BY name",
    )?;
    let connections = stmt
        .query_map([], |row| {
            let name: String = row.get(0)?;
            let type_str: String = row.get(1)?;
            let url: String = row.get(2)?;
            let env_str: String = row.get(3)?;
            let metadata_str: Option<String> = row.get(4)?;

            let r#type = type_str.parse().unwrap_or(ConnectionType::Postgres);
            let environment = env_str.parse().unwrap_or(Environment::Dev);
            let (metadata, password_storage) =
                metadata_from_row(metadata_str.as_ref());

            Ok(Connection {
                name,
                r#type,
                url,
                environment,
                metadata,
                selected_database: None,
                schema: None,
                table: None,
                password: None,
                password_storage,
                status: ConnectionStatus::default(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(connections)
}

/// Update a connection in the database
///
/// # Errors
///
/// This function will return an error if the database cannot be opened or if the query fails.
pub fn update_connection(
    old_name: &str,
    connection: &Connection,
) -> Result<(), Box<dyn Error>> {
    let db_path = get_db_path()?;
    let conn = SqliteConnection::open(db_path)?;

    let metadata = metadata_for_save(connection);

    conn.execute(
        "UPDATE connections SET name = ?, type = ?, url = ?, environment = ?, metadata = ? WHERE name = ?",
        params![
            connection.name,
            connection.r#type.to_string(),
            connection.url,
            connection.environment.to_string(),
            metadata,
            old_name,
        ],
    )?;

    Ok(())
}

/// Delete a connection from the database
///
/// # Errors
///
/// This function will return an error if the database cannot be opened or if the query fails.
pub fn delete_connection(connection_name: &str) -> Result<(), Box<dyn Error>> {
    let db_path = get_db_path()?;
    let conn = SqliteConnection::open(db_path)?;

    conn.execute(
        "DELETE FROM connections WHERE name = ?",
        params![connection_name],
    )?;

    Ok(())
}

/// Convert a `SQLite` value to a string representation
fn convert_sqlite_value_to_string(row: &rusqlite::Row, index: usize) -> String {
    // Try to get as different types and convert to string
    if let Ok(value) = row.get::<_, Option<String>>(index) {
        return value.unwrap_or_else(|| "NULL".to_string());
    }

    if let Ok(value) = row.get::<_, Option<i64>>(index) {
        return value.map_or_else(|| "NULL".to_string(), |v| v.to_string());
    }

    if let Ok(value) = row.get::<_, Option<f64>>(index) {
        return value.map_or_else(|| "NULL".to_string(), |v| v.to_string());
    }

    if let Ok(value) = row.get::<_, Option<Vec<u8>>>(index) {
        return value.map_or_else(
            || "NULL".to_string(),
            |v| format!("<{} bytes>", v.len()),
        );
    }

    // Fallback for unknown types
    "<unprintable>".to_string()
}

#[cfg(test)]
mod pragma_tests {
    use super::*;

    #[test]
    fn pragma_for_table_escapes_single_quotes() {
        assert_eq!(
            pragma_for_table("table_info", "it's"),
            "PRAGMA table_info('it''s')"
        );
    }

    #[test]
    fn decltypes_work_for_table_name_with_quote() {
        let conn = SqliteConnection::open_in_memory().unwrap();
        conn.execute_batch(r#"CREATE TABLE "it's" (id INTEGER, name TEXT);"#)
            .unwrap();
        let decls = sqlite_table_decltypes(&conn, "it's").unwrap();
        assert_eq!(decls.get("id").map(String::as_str), Some("INTEGER"));
        assert_eq!(decls.get("name").map(String::as_str), Some("TEXT"));
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::{env::temp_dir, fs::remove_file, process::id};

    use super::*;

    #[test]
    fn migration_allows_mysql_and_keeps_existing_rows() {
        let mut conn = SqliteConnection::open_in_memory().unwrap();
        // Existing install: schema as of the previous release (2 migrations).
        migrations().to_version(&mut conn, 2).unwrap();
        conn.execute(
            "INSERT INTO connections (name, type, url, environment, metadata) VALUES ('pg', 'postgres', 'postgres://u@h/db', 'dev', '{}')",
            [],
        )
        .unwrap();
        assert!(
            conn.execute(
                "INSERT INTO connections (name, type, url, environment) VALUES ('my', 'mysql', 'mysql://u@h/db', 'dev')",
                [],
            )
            .is_err(),
            "old schema rejects mysql"
        );

        migrations().to_latest(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO connections (name, type, url, environment) VALUES ('my', 'mysql', 'mysql://u@h/db', 'dev')",
            [],
        )
        .unwrap();
        let rows: Vec<(i64, String)> = conn
            .prepare("SELECT id, name FROM connections ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![(1, "pg".to_string()), (2, "my".to_string())]);
    }

    fn db() -> ((), Sqlite) {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let path = temp_dir().join(format!("d7s-test-{}-{n}.sqlite", id()));
        let _ = remove_file(&path);
        (
            (),
            Sqlite {
                name: "t".into(),
                path: path.to_string_lossy().to_string(),
            },
        )
    }

    fn first(r: &[TableRow]) -> String {
        r[0].values[0].clone()
    }

    #[tokio::test]
    async fn non_row_statements_run_exactly_once() {
        let (_d, db) = db();
        let r = db
            .execute_sql("CREATE TABLE t (id INTEGER, v TEXT)")
            .await
            .unwrap();
        assert_eq!(first(&r), "Affected rows: 0");

        let r = db
            .execute_sql("INSERT INTO t VALUES (1, 'a')")
            .await
            .unwrap();
        assert_eq!((r.len(), first(&r).as_str()), (1, "Affected rows: 1"));
        let r = db.execute_sql("SELECT count(*) FROM t").await.unwrap();
        assert_eq!(first(&r), "1");

        db.execute_sql("INSERT INTO t VALUES (2, 'b')")
            .await
            .unwrap();
        let r = db.execute_sql("UPDATE t SET v = 'z'").await.unwrap();
        assert_eq!(first(&r), "Affected rows: 2");
        let r = db.execute_sql("DELETE FROM t WHERE id = 1").await.unwrap();
        assert_eq!(first(&r), "Affected rows: 1");
        let r = db.execute_sql("SELECT count(*) FROM t").await.unwrap();
        assert_eq!(first(&r), "1");
    }

    #[tokio::test]
    async fn empty_select_is_empty_and_returning_runs_once() {
        let (_d, db) = db();
        db.execute_sql("CREATE TABLE t (id INTEGER)").await.unwrap();
        let r = db.execute_sql("SELECT * FROM t").await.unwrap();
        assert!(r.is_empty());

        let r = db
            .execute_sql("INSERT INTO t VALUES (7) RETURNING id")
            .await
            .unwrap();
        assert_eq!(r.len(), 1);
        let r = db.execute_sql("SELECT count(*) FROM t").await.unwrap();
        assert_eq!(first(&r), "1");
    }
}
