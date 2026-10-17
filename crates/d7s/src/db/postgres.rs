use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, hash_map::Entry},
    fmt::Write,
    sync::{Mutex, OnceLock},
};

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use tokio_postgres::{
    Row,
    config::SslMode,
    types::{FromSql, ToSql, Type},
};
use uuid::Uuid;

use crate::db::{
    Column, Database, DatabaseInfo, DbRowId, Schema, Table, TableData,
    TableDataPage, TableRow,
    query_log::report_sql,
    should_omit_for_insert_default,
    tls::{TlsConfig, TlsMode},
};

/// Connect (plain or TLS per `tls`) and spawn the connection driver task.
async fn connect_pg(
    config: &str,
    tls: &TlsConfig,
) -> Result<tokio_postgres::Client, Box<dyn std::error::Error>> {
    let mut config: tokio_postgres::Config = config.parse()?;
    // With `SslMode::Disable` the connector is never used.
    config.ssl_mode(match tls.mode {
        TlsMode::Disable => SslMode::Disable,
        TlsMode::Prefer => SslMode::Prefer,
        TlsMode::Require | TlsMode::VerifyFull => SslMode::Require,
    });
    let connector =
        tokio_postgres_rustls::MakeRustlsConnect::new(tls.client_config()?);
    let (client, conn) = config.connect(connector).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("Database connection error: {e}");
        }
    });
    Ok(client)
}

// Thin wrappers that report the SQL text (as sent, `$n` placeholders, no
// bound values) to the session query log before running it.
async fn pg_query(
    c: &tokio_postgres::Client,
    sql: &str,
    p: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, tokio_postgres::Error> {
    report_sql(sql);
    c.query(sql, p).await
}

async fn pg_query_one(
    c: &tokio_postgres::Client,
    sql: &str,
    p: &[&(dyn ToSql + Sync)],
) -> Result<Row, tokio_postgres::Error> {
    report_sql(sql);
    c.query_one(sql, p).await
}

async fn pg_execute(
    c: &tokio_postgres::Client,
    sql: &str,
    p: &[&(dyn ToSql + Sync)],
) -> Result<u64, tokio_postgres::Error> {
    report_sql(sql);
    c.execute(sql, p).await
}

/// Cache key: one physical Postgres database table (server + db + schema + table).
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct PostgresTableIdentity {
    host: String,
    port: String,
    database: String,
    schema: String,
    table: String,
}

/// Column order and which columns are user-defined types (cast to `::text` when selecting).
#[derive(Debug, Clone)]
struct CachedTableColumnInfo {
    ordered_columns: Vec<String>,
    udt_columns: HashSet<String>,
}

static TABLE_COLUMN_CACHE: OnceLock<
    Mutex<HashMap<PostgresTableIdentity, CachedTableColumnInfo>>,
> = OnceLock::new();

fn table_column_cache()
-> &'static Mutex<HashMap<PostgresTableIdentity, CachedTableColumnInfo>> {
    TABLE_COLUMN_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Escape a `PostgreSQL` identifier (double-quoted).
fn pg_quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// `pg_catalog.format_type` string for a column, for `CAST($n AS …)` in updates.
fn pg_resolve_format_type(
    types: &HashMap<String, String>,
    col: &str,
) -> String {
    types
        .get(col)
        .cloned()
        .or_else(|| {
            types
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(col))
                .map(|(_, v)| v.clone())
        })
        .unwrap_or_else(|| "text".to_string())
}

async fn pg_column_format_types(
    client: &tokio_postgres::Client,
    schema_name: &str,
    table_name: &str,
) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    let q = r"
        SELECT a.attname::text AS col,
               pg_catalog.format_type(a.atttypid, a.atttypmod) AS fmt
        FROM pg_catalog.pg_attribute a
        INNER JOIN pg_catalog.pg_class c ON a.attrelid = c.oid
        INNER JOIN pg_catalog.pg_namespace n ON c.relnamespace = n.oid
        WHERE n.nspname = $1
          AND c.relname = $2
          AND a.attnum > 0
          AND NOT a.attisdropped
    ";
    let rows = pg_query(client, q, &[&schema_name, &table_name]).await?;
    let mut m = HashMap::with_capacity(rows.len());
    for row in rows {
        let col: String = row.get(0);
        let fmt: String = row.get(1);
        m.insert(col, fmt);
    }
    Ok(m)
}

/// Base element type for a one-dimensional `format_type` array (e.g. `text[]` → `text`).
/// Returns `None` for non-arrays or multidimensional arrays (not handled here).
fn pg_array_element_base_type(format_type: &str) -> Option<&str> {
    let ft = format_type.trim();
    if !ft.ends_with("[]") {
        return None;
    }
    let base = ft[..ft.len() - 2].trim();
    if base.ends_with("[]") {
        return None;
    }
    Some(base)
}

fn pg_array_elem_base_is_numeric(elem_base: &str) -> bool {
    let b = elem_base.to_ascii_lowercase();
    matches!(
        b.as_str(),
        "smallint"
            | "integer"
            | "bigint"
            | "int2"
            | "int4"
            | "int8"
            | "oid"
            | "real"
            | "float4"
            | "float8"
            | "double precision"
    ) || b.starts_with("numeric(")
        || b.starts_with("decimal(")
        || b == "numeric"
        || b == "decimal"
        || b == "money"
}

fn pg_json_scalar_to_string(v: Value) -> String {
    match v {
        Value::String(s) => s,
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "NULL".to_string(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(&v).unwrap_or_else(|_| v.to_string())
        }
    }
}

/// Split `inner` on commas outside of double quotes (with basic `\"` escapes).
fn pg_split_array_list(inner: &str) -> Vec<String> {
    let mut res = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut prev_escape = false;
    for ch in inner.chars() {
        if prev_escape {
            cur.push(ch);
            prev_escape = false;
            continue;
        }
        match ch {
            '\\' => {
                prev_escape = true;
                cur.push(ch);
            }
            '"' => {
                in_quote = !in_quote;
                cur.push(ch);
            }
            ',' if !in_quote => {
                res.push(pg_trim_array_token(&cur));
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    res.push(pg_trim_array_token(&cur));
    res.into_iter().filter(|s| !s.is_empty()).collect()
}

fn pg_trim_array_token(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1]
            .replace(r#"\""#, "\"")
            .replace(r"\\", r"\")
    } else {
        s.to_string()
    }
}

fn pg_parse_array_elements(raw: &str) -> Vec<String> {
    let t = raw.trim();
    if t.is_empty() {
        return vec![];
    }
    if let Ok(json_vals) = serde_json::from_str::<Vec<Value>>(t) {
        return json_vals
            .into_iter()
            .map(pg_json_scalar_to_string)
            .collect();
    }
    let inner = if let Some(s) =
        t.strip_prefix('[').and_then(|x| x.strip_suffix(']'))
    {
        s
    } else if let Some(s) =
        t.strip_prefix('{').and_then(|x| x.strip_suffix('}'))
    {
        s
    } else {
        return vec![t.to_string()];
    };
    pg_split_array_list(inner)
}

fn pg_array_elem_needs_quotes(s: &str) -> bool {
    s.is_empty()
        || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || s.chars()
            .any(|c| matches!(c, ',' | '{' | '}' | ' ' | '\t' | '\n' | '\r'))
}

fn pg_format_array_text_element(s: &str) -> String {
    if s.contains('"') || s.contains('\\') || pg_array_elem_needs_quotes(s) {
        let escaped = s.replace('\\', r"\\").replace('"', r#"\""#);
        return format!("\"{escaped}\"");
    }
    s.to_string()
}

/// Turn UI / JSON-like array text into a `PostgreSQL` array literal for `CAST($1::text AS …[])`.
fn pg_user_text_to_pg_array_literal(
    raw: &str,
    elem_base: &str,
) -> Result<String, ()> {
    let elems = pg_parse_array_elements(raw);
    let numeric = pg_array_elem_base_is_numeric(elem_base);
    let boolean = elem_base.eq_ignore_ascii_case("boolean");
    let mut parts = Vec::with_capacity(elems.len());
    for e in elems {
        let e = e.trim();
        if e.is_empty() {
            continue;
        }
        if e.eq_ignore_ascii_case("null") {
            parts.push("NULL".to_string());
            continue;
        }
        if boolean {
            let v = match e.to_ascii_lowercase().as_str() {
                "t" | "true" | "1" | "yes" | "y" => "t",
                "f" | "false" | "0" | "no" | "n" => "f",
                _ => return Err(()),
            };
            parts.push(v.to_string());
            continue;
        }
        if numeric {
            parts.push(e.to_string());
            continue;
        }
        parts.push(pg_format_array_text_element(e));
    }
    Ok(format!("{{{}}}", parts.join(",")))
}

/// Normalize values for `CAST($n::text AS <format_type>)`: arrays get a proper `{…}` literal.
fn pg_coerce_typed_text_input<'a>(
    raw: &'a str,
    pg_format_type: &str,
) -> Cow<'a, str> {
    if let Some(elem_base) = pg_array_element_base_type(pg_format_type) {
        let t = raw.trim();
        if t.starts_with('{') && t.ends_with('}') {
            return Cow::Borrowed(raw);
        }
        if let Ok(lit) = pg_user_text_to_pg_array_literal(t, elem_base) {
            return Cow::Owned(lit);
        }
    }
    Cow::Borrowed(raw)
}

fn prepend_ctid_to_select(base_select: &str) -> String {
    let Some(rest) = base_select.strip_prefix("SELECT ") else {
        return base_select.to_string();
    };
    format!("SELECT ctid::text, {rest}")
}

fn build_table_data_select_base(
    schema_name: &str,
    table_name: &str,
    info: &CachedTableColumnInfo,
) -> String {
    let select_list = if info.ordered_columns.is_empty() {
        "*".to_string()
    } else {
        info.ordered_columns
            .iter()
            .map(|col| {
                let q = pg_quote_ident(col);
                if info.udt_columns.contains(col) {
                    format!("{q}::text")
                } else {
                    q
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    format!(
        "SELECT {select_list} FROM {}.{}",
        pg_quote_ident(schema_name),
        pg_quote_ident(table_name)
    )
}

#[derive(Debug, Clone, Default)]
pub struct Postgres {
    pub name: String,
    pub host: Option<String>,
    pub port: Option<String>,
    pub user: String,
    pub database: String,
    pub password: String,
    pub tls: TlsConfig,
}

impl TableData for Postgres {
    fn title() -> &'static str {
        "Postgres"
    }

    fn ref_array(&self) -> Vec<String> {
        vec![
            self.name.clone(),
            self.host.clone().unwrap_or_default(),
            self.port.clone().unwrap_or_default(),
            self.user.clone(),
            self.password.clone(),
        ]
    }

    fn num_columns(&self) -> usize {
        self.ref_array().len()
    }

    fn cols() -> Vec<&'static str> {
        vec!["Name", "Host", "Port", "User", "Password"]
    }
}

#[async_trait::async_trait]
impl Database for Postgres {
    async fn test(&self) -> bool {
        let config = format!(
            "host={} port={} user={} password={} dbname={}",
            self.host.clone().unwrap_or_else(|| "localhost".to_string()),
            self.port.clone().unwrap_or_else(|| "5432".to_string()),
            self.user,
            self.password,
            self.database
        );

        connect_pg(&config, &self.tls).await.is_ok()
    }

    async fn execute_sql(
        &self,
        sql: &str,
    ) -> Result<Vec<TableRow>, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;

        // Prepare once; statements without result columns are executed (not
        // queried) so nothing runs twice.
        let stmt = client.prepare(sql).await?;
        let mut result = Vec::new();

        if stmt.columns().is_empty() {
            let affected_rows = client.execute(&stmt, &[]).await?;
            result.push(TableRow {
                values: vec![format!("Affected rows: {affected_rows}")],
                column_names: vec!["Result".to_string()],
            });
        } else {
            let rows = client.query(&stmt, &[]).await?;
            let column_names: Vec<String> = stmt
                .columns()
                .iter()
                .map(|col| col.name().to_string())
                .collect();

            for row in &rows {
                let values = row
                    .columns()
                    .iter()
                    .enumerate()
                    .map(|(i, col)| column_to_string(row, i, col.type_()))
                    .collect();
                result.push(TableRow {
                    values,
                    column_names: column_names.clone(),
                });
            }
        }

        Ok(result)
    }

    async fn get_schemas(
        &self,
    ) -> Result<Vec<Schema>, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;

        let query = "
            SELECT schema_name, schema_owner
            FROM information_schema.schemata
            WHERE schema_name NOT IN ('information_schema', 'pg_catalog', 'pg_toast')
            ORDER BY schema_name
        ";

        let rows = pg_query(&client, query, &[]).await?;
        let mut schemas = Vec::new();

        for row in rows {
            let schema = Schema {
                name: row.get(0),
                owner: row.get(1),
            };
            schemas.push(schema);
        }

        Ok(schemas)
    }

    async fn get_tables(
        &self,
        schema_name: &str,
    ) -> Result<Vec<Table>, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;

        let query = "
            SELECT
                t.table_name,
                t.table_schema,
                pg_size_pretty(pg_total_relation_size(quote_ident(t.table_schema)||'.'||quote_ident(t.table_name))) as size
            FROM information_schema.tables t
            WHERE t.table_schema = $1
            AND t.table_type = 'BASE TABLE'
            ORDER BY t.table_name;
        ";

        let rows = pg_query(&client, query, &[&schema_name]).await?;
        let tables = rows
            .iter()
            .map(|row| Table {
                name: row.get(0),
                schema: row.get(1),
                size: row.get(2),
            })
            .collect();

        Ok(tables)
    }

    async fn get_columns(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<Column>, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;

        let query = "
            SELECT
                c.column_name,
                c.data_type,
                c.is_nullable,
                c.column_default,
                pgd.description
            FROM information_schema.columns c
            LEFT JOIN pg_catalog.pg_statio_all_tables st ON (c.table_schema = st.schemaname AND c.table_name = st.relname)
            LEFT JOIN pg_catalog.pg_description pgd ON (pgd.objoid = st.relid AND pgd.objsubid = c.ordinal_position)
            WHERE c.table_schema = $1
            AND c.table_name = $2
            ORDER BY c.ordinal_position
        ";

        let rows =
            pg_query(&client, query, &[&schema_name, &table_name]).await?;
        let columns = rows
            .iter()
            .map(|row| Column {
                name: row.get(0),
                data_type: row.get(1),
                is_nullable: row.get::<_, String>(2) == "YES",
                default_value: row.get(3),
                description: row.get(4),
            })
            .collect();

        Ok(columns)
    }

    async fn get_table_data_page(
        &self,
        schema_name: &str,
        table_name: &str,
        offset: u64,
        limit: u32,
    ) -> Result<TableDataPage, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;

        let layout = self
            .get_or_fetch_table_column_layout(&client, schema_name, table_name)
            .await?;

        let base =
            build_table_data_select_base(schema_name, table_name, &layout);
        let query =
            format!("{} LIMIT $1 OFFSET $2", prepend_ctid_to_select(&base));
        let limit_i: i64 = i64::from(limit);
        let offset_i: i64 = offset.try_into().unwrap_or(i64::MAX);
        let rows = pg_query(&client, &query, &[&limit_i, &offset_i]).await?;
        let mut column_names = Vec::new();

        if let Some(first_row) = rows.first() {
            for column in first_row.columns() {
                let name = column.name();
                if name == "ctid" {
                    continue;
                }
                column_names.push(name.to_string());
            }
        }

        let mut row_ids = Vec::with_capacity(rows.len());
        let mut data = Vec::with_capacity(rows.len());
        for row in &rows {
            let cols = row.columns();
            let Some(first_col) = cols.first() else {
                continue;
            };
            let ctid = column_to_string(row, 0, first_col.type_());
            row_ids.push(Some(DbRowId::PostgresCtid(ctid)));
            let values: Vec<String> = cols
                .iter()
                .enumerate()
                .skip(1)
                .map(|(i, col)| column_to_string(row, i, col.type_()))
                .collect();
            data.push(values);
        }

        Ok(TableDataPage {
            rows: data,
            column_names,
            row_ids,
        })
    }

    async fn get_primary_key_columns(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;
        let q = "
            SELECT kcu.column_name
            FROM information_schema.table_constraints tc
            INNER JOIN information_schema.key_column_usage kcu
                ON tc.constraint_schema = kcu.constraint_schema
                AND tc.constraint_name = kcu.constraint_name
                AND tc.table_schema = kcu.table_schema
                AND tc.table_name = kcu.table_name
            WHERE tc.constraint_type = 'PRIMARY KEY'
                AND tc.table_schema = $1
                AND tc.table_name = $2
            ORDER BY kcu.ordinal_position
        ";
        let rows = pg_query(&client, q, &[&schema_name, &table_name]).await?;
        Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
    }

    async fn update_table_cell(
        &self,
        schema_name: &str,
        table_name: &str,
        set_column: &str,
        new_value: &str,
        primary_key: &[(String, String)],
        row_id_fallback: Option<DbRowId>,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;
        let col_types =
            pg_column_format_types(&client, schema_name, table_name).await?;
        let set_ty = pg_resolve_format_type(&col_types, set_column);
        let set_q = pg_quote_ident(set_column);
        let tgt = format!(
            "{}.{}",
            pg_quote_ident(schema_name),
            pg_quote_ident(table_name),
        );

        if !primary_key.is_empty() {
            // Bind every value as text on the wire (`$n::text`), then cast to the column type.
            // Otherwise PostgreSQL infers `$n` as the target type (e.g. int4) and tokio-postgres
            // rejects `&str` for that OID.
            let mut sql = format!(
                "UPDATE {tgt} SET {set_q} = CAST($1::text AS {set_ty}) WHERE "
            );
            let mut owned: Vec<String> =
                Vec::with_capacity(1 + primary_key.len());
            owned.push(
                pg_coerce_typed_text_input(new_value, &set_ty).into_owned(),
            );
            for (i, (k, v)) in primary_key.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let param_num = i + 2;
                let pk_ty = pg_resolve_format_type(&col_types, k);
                let _ = write!(
                    sql,
                    "{} = CAST(${}::text AS {pk_ty})",
                    pg_quote_ident(k),
                    param_num
                );
                owned.push(pg_coerce_typed_text_input(v, &pk_ty).into_owned());
            }
            let mut params: Vec<&(dyn ToSql + Sync)> =
                Vec::with_capacity(owned.len());
            for s in &owned {
                params.push(s);
            }
            let n = pg_execute(&client, &sql, &params[..]).await?;
            return Ok(n);
        }

        if let Some(DbRowId::PostgresCtid(ctid)) = row_id_fallback {
            let sql = format!(
                "UPDATE {tgt} SET {set_q} = CAST($1::text AS {set_ty}) WHERE ctid = $2::text::tid"
            );
            let set_bound =
                pg_coerce_typed_text_input(new_value, &set_ty).into_owned();
            let p0: &(dyn ToSql + Sync) = &set_bound;
            let p1: &(dyn ToSql + Sync) = &ctid;
            let n = pg_execute(&client, &sql, &[p0, p1]).await?;
            return Ok(n);
        }

        Err(
            "Cannot update row: table has no primary key and no row address was recorded"
                .into(),
        )
    }

    async fn insert_table_row(
        &self,
        schema_name: &str,
        table_name: &str,
        values: &[String],
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;
        let columns = self.get_columns(schema_name, table_name).await?;
        if values.len() != columns.len() {
            return Err("INSERT column count does not match table.".into());
        }
        let col_types =
            pg_column_format_types(&client, schema_name, table_name).await?;
        let tgt = format!(
            "{}.{}",
            pg_quote_ident(schema_name),
            pg_quote_ident(table_name),
        );
        let mut col_list: Vec<String> = Vec::new();
        let mut val_placeholders: Vec<String> = Vec::new();
        let mut owned: Vec<String> = Vec::new();
        for (i, c) in columns.iter().enumerate() {
            let raw = values.get(i).map_or("", String::as_str);
            if should_omit_for_insert_default(c, raw, false, false) {
                continue;
            }
            if raw.trim().is_empty() || raw.eq_ignore_ascii_case("null") {
                if c.is_nullable {
                    col_list.push(pg_quote_ident(&c.name));
                    val_placeholders.push("NULL".to_string());
                } else {
                    return Err(format!(
                        "Column \"{}\" is NOT NULL and has no value or default in the form.",
                        c.name
                    )
                    .into());
                }
            } else {
                col_list.push(pg_quote_ident(&c.name));
                let ty = pg_resolve_format_type(&col_types, &c.name);
                let param_num = owned.len() + 1;
                val_placeholders
                    .push(format!("CAST(${param_num}::text AS {ty})"));
                owned.push(pg_coerce_typed_text_input(raw, &ty).into_owned());
            }
        }
        if col_list.is_empty() {
            let sql = format!("INSERT INTO {tgt} DEFAULT VALUES");
            return Ok(pg_execute(&client, &sql, &[]).await?);
        }
        let sql = format!(
            "INSERT INTO {tgt} ({}) VALUES ({})",
            col_list.join(", "),
            val_placeholders.join(", ")
        );
        let mut params: Vec<&(dyn ToSql + Sync)> =
            Vec::with_capacity(owned.len());
        for s in &owned {
            params.push(s);
        }
        let n = pg_execute(&client, &sql, &params[..]).await?;
        Ok(n)
    }

    async fn delete_table_row(
        &self,
        schema_name: &str,
        table_name: &str,
        primary_key: &[(String, String)],
        row_id_fallback: Option<DbRowId>,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;
        let col_types =
            pg_column_format_types(&client, schema_name, table_name).await?;
        let tgt = format!(
            "{}.{}",
            pg_quote_ident(schema_name),
            pg_quote_ident(table_name),
        );

        if !primary_key.is_empty() {
            let mut sql = format!("DELETE FROM {tgt} WHERE ");
            let mut owned: Vec<String> = Vec::with_capacity(primary_key.len());
            for (i, (k, v)) in primary_key.iter().enumerate() {
                if i > 0 {
                    sql.push_str(" AND ");
                }
                let param_num = i + 1;
                let pk_ty = pg_resolve_format_type(&col_types, k);
                let _ = write!(
                    sql,
                    "{} = CAST(${param_num}::text AS {pk_ty})",
                    pg_quote_ident(k)
                );
                owned.push(pg_coerce_typed_text_input(v, &pk_ty).into_owned());
            }
            let mut params: Vec<&(dyn ToSql + Sync)> =
                Vec::with_capacity(owned.len());
            for s in &owned {
                params.push(s);
            }
            let n = pg_execute(&client, &sql, &params[..]).await?;
            return Ok(n);
        }

        if let Some(DbRowId::PostgresCtid(ctid)) = row_id_fallback {
            let sql = format!("DELETE FROM {tgt} WHERE ctid = $1::text::tid");
            let p: &(dyn ToSql + Sync) = &ctid;
            let n = pg_execute(&client, &sql, &[p]).await?;
            return Ok(n);
        }

        Err(
            "Cannot delete row: no primary key and no row address was recorded"
                .into(),
        )
    }

    async fn get_table_row_count(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;
        let q = format!(
            "SELECT COUNT(*)::bigint FROM {}.{}",
            pg_quote_ident(schema_name),
            pg_quote_ident(table_name),
        );
        let row = pg_query_one(&client, &q, &[]).await?;
        let count: i64 = row.get(0);
        Ok(count.cast_unsigned())
    }

    async fn get_table_index_names(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;
        let q = "SELECT indexname FROM pg_indexes WHERE schemaname = $1 AND tablename = $2 ORDER BY indexname";
        let rows = pg_query(&client, q, &[&schema_name, &table_name]).await?;
        Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
    }

    async fn get_table_size(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;
        let q =
            "SELECT pg_size_pretty(pg_total_relation_size($1::text::regclass))";
        let ident = format!(
            "{}.{}",
            pg_quote_ident(schema_name),
            pg_quote_ident(table_name)
        );
        let row = pg_query_one(&client, q, &[&ident]).await?;
        Ok(Some(row.get::<_, String>(0)))
    }

    async fn get_databases(
        &self,
    ) -> Result<Vec<DatabaseInfo>, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;

        let query = "
            SELECT datname
            FROM pg_database
            WHERE datistemplate = false
            ORDER BY datname
        ";

        let rows = pg_query(&client, query, &[]).await?;
        let databases = rows
            .iter()
            .map(|row| DatabaseInfo { name: row.get(0) })
            .collect();

        Ok(databases)
    }
}

impl Postgres {
    /// Load ordered columns and UDT flags from `information_schema`, using a process-wide cache.
    async fn get_or_fetch_table_column_layout(
        &self,
        client: &tokio_postgres::Client,
        schema_name: &str,
        table_name: &str,
    ) -> Result<CachedTableColumnInfo, Box<dyn std::error::Error>> {
        let key = PostgresTableIdentity {
            host: self.host.clone().unwrap_or_else(|| "localhost".to_string()),
            port: self.port.clone().unwrap_or_else(|| "5432".to_string()),
            database: self.database.clone(),
            schema: schema_name.to_string(),
            table: table_name.to_string(),
        };

        {
            let guard = table_column_cache().lock().map_err(|_| {
                std::io::Error::other("table column cache lock poisoned")
            })?;
            if let Some(info) = guard.get(&key) {
                return Ok(info.clone());
            }
        }

        let layout_query = r"
            SELECT column_name, data_type
            FROM information_schema.columns
            WHERE table_schema = $1 AND table_name = $2
            ORDER BY ordinal_position
        ";

        let rows = pg_query(client, layout_query, &[&schema_name, &table_name])
            .await?;
        let mut ordered_columns = Vec::new();
        let mut udt_columns = HashSet::new();
        for row in rows {
            let name: String = row.get(0);
            let data_type: String = row.get(1);
            ordered_columns.push(name.clone());
            if data_type == "USER-DEFINED" {
                udt_columns.insert(name);
            }
        }

        let info = CachedTableColumnInfo {
            ordered_columns,
            udt_columns,
        };

        let mut guard = table_column_cache().lock().map_err(|_| {
            std::io::Error::other("table column cache lock poisoned")
        })?;
        match guard.entry(key) {
            Entry::Occupied(e) => Ok(e.get().clone()),
            Entry::Vacant(e) => Ok(e.insert(info).clone()),
        }
    }

    async fn get_connection(
        &self,
    ) -> Result<tokio_postgres::Client, Box<dyn std::error::Error>> {
        let config = format!(
            "host={} port={} user={} password={} dbname={}",
            self.host.clone().unwrap_or_else(|| "localhost".to_string()),
            self.port.clone().unwrap_or_else(|| "5432".to_string()),
            self.user,
            self.password,
            self.database
        );
        connect_pg(&config, &self.tls).await
    }

    /// Retrieves sample data from a table.
    ///
    /// # Errors
    ///
    /// Returns an error if the database connection fails or the query cannot be executed.
    #[allow(dead_code)]
    pub async fn get_sample_data(
        &self,
        schema_name: &str,
        table_name: &str,
        limit: i64,
    ) -> Result<Vec<Vec<String>>, Box<dyn std::error::Error>> {
        let client = self.get_connection().await?;

        let query =
            format!("SELECT * FROM {schema_name}.{table_name} LIMIT $1");

        let rows = pg_query(&client, &query, &[&limit]).await?;
        let data = rows
            .iter()
            .map(|row| {
                row.columns()
                    .iter()
                    .enumerate()
                    .map(|(i, col)| column_to_string(row, i, col.type_()))
                    .collect()
            })
            .collect();

        Ok(data)
    }
}

/// Try and convert the value of the row to a string based on the type of the
/// column.
///
/// The types used to check against are gathered from:
/// <https://docs.rs/tokio-postgres/latest/tokio_postgres/types/struct.Type.html>
#[allow(clippy::too_many_lines)]
fn column_to_string(row: &Row, index: usize, ty: &Type) -> String {
    if row.is_empty() {
        return "NULL".to_string();
    }

    match *ty {
        // Boolean
        Type::BOOL => try_get::<bool>(row, index),

        // Binary data
        Type::BYTEA => try_get_bytes(row, index),

        // Integer types
        Type::CHAR => try_get::<i8>(row, index),
        Type::INT2 => try_get::<i16>(row, index),
        Type::INT4 => try_get::<i32>(row, index),
        Type::INT8 => try_get::<i64>(row, index),

        // Floating point types
        Type::FLOAT4 => try_get::<f32>(row, index),
        Type::FLOAT8 => try_get::<f64>(row, index),

        // Object identifier types
        Type::OID => try_get::<u32>(row, index),

        // UUID
        Type::UUID => try_get::<Uuid>(row, index),

        // JSON types
        Type::JSON | Type::JSONB => try_get::<serde_json::Value>(row, index),

        // Date/Time types with special handling
        Type::TIMESTAMP => try_get_timestamp(row, index),
        Type::TIMESTAMPTZ => try_get_timestamptz(row, index),
        Type::DATE => try_get_date(row, index),
        Type::TIME | Type::TIMETZ => try_get_time(row, index),

        // Typed array types
        Type::BOOL_ARRAY => try_get_array::<bool>(row, index),
        Type::CHAR_ARRAY => try_get_array::<i8>(row, index),
        Type::INT2_ARRAY => try_get_array::<i16>(row, index),
        Type::INT4_ARRAY => try_get_array::<i32>(row, index),
        Type::INT8_ARRAY => try_get_array::<i64>(row, index),
        Type::FLOAT4_ARRAY => try_get_array::<f32>(row, index),
        Type::FLOAT8_ARRAY => try_get_array::<f64>(row, index),
        Type::OID_ARRAY => try_get_array::<u32>(row, index),
        Type::UUID_ARRAY => try_get_array::<Uuid>(row, index),
        Type::TEXT_ARRAY
        | Type::VARCHAR_ARRAY
        | Type::BPCHAR_ARRAY
        | Type::NAME_ARRAY => try_get_array::<String>(row, index),

        // Numeric types with rust_decimal
        Type::NUMERIC => try_get_numeric(row, index),
        Type::NUMERIC_ARRAY => try_get_numeric_array(row, index),

        // All other types as strings - consolidated for clippy::match_same_arms
        Type::INTERVAL
        | Type::TEXT
        | Type::VARCHAR
        | Type::BPCHAR
        | Type::NAME
        | Type::UNKNOWN
        | Type::INET
        | Type::CIDR
        | Type::MACADDR
        | Type::MACADDR8
        | Type::XML
        | Type::BIT
        | Type::VARBIT
        | Type::MONEY
        | Type::TID
        | Type::XID
        | Type::CID
        | Type::REGPROC
        | Type::REGPROCEDURE
        | Type::REGOPER
        | Type::REGOPERATOR
        | Type::REGCLASS
        | Type::REGTYPE
        | Type::REGNAMESPACE
        | Type::REGROLE
        | Type::REGCONFIG
        | Type::REGDICTIONARY
        | Type::REGCOLLATION
        | Type::POINT
        | Type::LSEG
        | Type::PATH
        | Type::BOX
        | Type::POLYGON
        | Type::LINE
        | Type::CIRCLE
        | Type::TS_VECTOR
        | Type::TSQUERY
        | Type::GTS_VECTOR
        | Type::TID_ARRAY
        | Type::XID_ARRAY
        | Type::CID_ARRAY
        | Type::INT4_RANGE
        | Type::INT8_RANGE
        | Type::NUM_RANGE
        | Type::TS_RANGE
        | Type::TSTZ_RANGE
        | Type::DATE_RANGE
        | Type::INT4MULTI_RANGE
        | Type::INT8MULTI_RANGE
        | Type::NUMMULTI_RANGE
        | Type::TSMULTI_RANGE
        | Type::TSTZMULTI_RANGE
        | Type::DATEMULTI_RANGE
        | Type::PG_LSN
        | Type::PG_SNAPSHOT
        | Type::TXID_SNAPSHOT
        | Type::PG_NDISTINCT
        | Type::PG_DEPENDENCIES
        | Type::PG_MCV_LIST
        | Type::PG_BRIN_BLOOM_SUMMARY
        | Type::PG_BRIN_MINMAX_MULTI_SUMMARY
        | Type::JSONPATH
        | Type::XID8
        | Type::ACLITEM
        | Type::REFCURSOR
        | Type::BYTEA_ARRAY
        | Type::TIMESTAMP_ARRAY
        | Type::TIMESTAMPTZ_ARRAY
        | Type::DATE_ARRAY
        | Type::TIME_ARRAY
        | Type::TIMETZ_ARRAY
        | Type::INTERVAL_ARRAY
        | Type::INET_ARRAY
        | Type::CIDR_ARRAY
        | Type::MACADDR_ARRAY
        | Type::MACADDR8_ARRAY
        | Type::JSON_ARRAY
        | Type::JSONB_ARRAY
        | Type::REGPROC_ARRAY
        | Type::REGPROCEDURE_ARRAY
        | Type::REGOPER_ARRAY
        | Type::REGOPERATOR_ARRAY
        | Type::REGCLASS_ARRAY
        | Type::REGTYPE_ARRAY
        | Type::REGNAMESPACE_ARRAY
        | Type::REGROLE_ARRAY
        | Type::REGCONFIG_ARRAY
        | Type::REGDICTIONARY_ARRAY
        | Type::REGCOLLATION_ARRAY
        | Type::POINT_ARRAY
        | Type::LSEG_ARRAY
        | Type::PATH_ARRAY
        | Type::BOX_ARRAY
        | Type::POLYGON_ARRAY
        | Type::LINE_ARRAY
        | Type::CIRCLE_ARRAY
        | Type::BIT_ARRAY
        | Type::VARBIT_ARRAY
        | Type::MONEY_ARRAY
        | Type::TS_VECTOR_ARRAY
        | Type::TSQUERY_ARRAY
        | Type::GTS_VECTOR_ARRAY
        | Type::INT4_RANGE_ARRAY
        | Type::INT8_RANGE_ARRAY
        | Type::NUM_RANGE_ARRAY
        | Type::TS_RANGE_ARRAY
        | Type::TSTZ_RANGE_ARRAY
        | Type::DATE_RANGE_ARRAY
        | Type::INT4MULTI_RANGE_ARRAY
        | Type::INT8MULTI_RANGE_ARRAY
        | Type::NUMMULTI_RANGE_ARRAY
        | Type::TSMULTI_RANGE_ARRAY
        | Type::TSTZMULTI_RANGE_ARRAY
        | Type::DATEMULTI_RANGE_ARRAY
        | Type::XML_ARRAY
        | Type::JSONPATH_ARRAY
        | Type::XID8_ARRAY
        | Type::PG_LSN_ARRAY
        | Type::PG_SNAPSHOT_ARRAY
        | Type::TXID_SNAPSHOT_ARRAY
        | Type::ACLITEM_ARRAY
        | Type::REFCURSOR_ARRAY
        | Type::CSTRING_ARRAY
        | Type::INT2_VECTOR
        | Type::OID_VECTOR
        | Type::INT2_VECTOR_ARRAY
        | Type::OID_VECTOR_ARRAY
        | Type::PG_DDL_COMMAND
        | Type::PG_NODE_TREE
        | Type::TABLE_AM_HANDLER
        | Type::INDEX_AM_HANDLER
        | Type::TSM_HANDLER
        | Type::FDW_HANDLER
        | Type::LANGUAGE_HANDLER
        | Type::INTERNAL
        | Type::EVENT_TRIGGER
        | Type::TRIGGER
        | Type::VOID
        | Type::RECORD
        | Type::CSTRING
        | Type::ANY
        | Type::ANYARRAY
        | Type::ANYELEMENT
        | Type::ANYNONARRAY
        | Type::ANYENUM
        | Type::ANY_RANGE
        | Type::ANYMULTI_RANGE
        | Type::ANYCOMPATIBLE
        | Type::ANYCOMPATIBLEARRAY
        | Type::ANYCOMPATIBLENONARRAY
        | Type::ANYCOMPATIBLE_RANGE
        | Type::ANYCOMPATIBLEMULTI_RANGE
        | Type::RECORD_ARRAY => try_get::<String>(row, index),

        // Fallback for unknown types
        _ => row
            .columns()
            .get(index)
            .map(|c| c.type_().name())
            .map_or_else(
                || try_get::<String>(row, index),
                |type_name| try_get_or_label::<String>(row, index, type_name),
            ),
    }
}

/// Generic helper to get a value and convert it to a string, handling NULL values
fn try_get<'a, T: ToString + FromSql<'a>>(
    row: &'a Row,
    index: usize,
) -> String {
    row.try_get::<_, Option<T>>(index)
        .ok()
        .flatten()
        .map_or_else(|| "NULL".to_string(), |v| v.to_string())
}

/// Helper to get array values and format them as a comma-separated list
fn try_get_array<'a, T: ToString + FromSql<'a>>(
    row: &'a Row,
    index: usize,
) -> String {
    row.try_get::<_, Option<Vec<T>>>(index)
        .ok()
        .flatten()
        .map(|arr| {
            arr.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .map_or_else(|| "NULL".to_string(), |s| format!("[{s}]"))
}

/// Helper to get binary data and display the byte count
fn try_get_bytes(row: &Row, index: usize) -> String {
    row.try_get::<_, Option<Vec<u8>>>(index)
        .ok()
        .flatten()
        .map_or_else(
            || "NULL".to_string(),
            |bytes| format!("<{} bytes>", bytes.len()),
        )
}

/// Fallback helper for unknown types - tries to get as string or shows type label
fn try_get_or_label<'a, T: ToString + FromSql<'a>>(
    row: &'a Row,
    index: usize,
    type_name: &str,
) -> String {
    if let Ok(Some(v)) = row.try_get::<_, Option<T>>(index) {
        v.to_string()
    } else {
        format!("<{type_name}>")
    }
}

/// Helper to get NUMERIC values using `rust_decimal`
fn try_get_numeric(row: &Row, index: usize) -> String {
    row.try_get::<_, Option<Decimal>>(index)
        .ok()
        .flatten()
        .map_or_else(|| "NULL".to_string(), |v| v.to_string())
}

/// Helper to get NUMERIC array values
fn try_get_numeric_array(row: &Row, index: usize) -> String {
    row.try_get::<_, Option<Vec<Decimal>>>(index)
        .ok()
        .flatten()
        .map(|arr| {
            arr.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .map_or_else(|| "NULL".to_string(), |s| format!("[{s}]"))
}

/// Helper to get TIMESTAMP values with better compatibility
/// Tries multiple datetime representations for robustness
fn try_get_timestamp(row: &Row, index: usize) -> String {
    // Try NaiveDateTime first (TIMESTAMP WITHOUT TIME ZONE)
    if let Ok(Some(v)) = row.try_get::<_, Option<NaiveDateTime>>(index) {
        return v.to_string();
    }

    // Fallback to DateTime<Utc> in case it's stored with timezone info
    if let Ok(Some(v)) = row.try_get::<_, Option<DateTime<Utc>>>(index) {
        return v.naive_utc().to_string();
    }

    // Final fallback to string
    try_get::<String>(row, index)
}

/// Helper to get TIMESTAMPTZ values with better compatibility
/// Handles timezone-aware timestamps
fn try_get_timestamptz(row: &Row, index: usize) -> String {
    // Try DateTime<Utc> first (TIMESTAMP WITH TIME ZONE)
    if let Ok(Some(v)) = row.try_get::<_, Option<DateTime<Utc>>>(index) {
        return v.to_rfc3339();
    }

    // Fallback to NaiveDateTime if stored without timezone
    if let Ok(Some(v)) = row.try_get::<_, Option<NaiveDateTime>>(index) {
        return v.to_string();
    }

    // Final fallback to string
    try_get::<String>(row, index)
}

/// Helper to get DATE values
fn try_get_date(row: &Row, index: usize) -> String {
    if let Ok(Some(v)) = row.try_get::<_, Option<NaiveDate>>(index) {
        return v.format("%Y-%m-%d").to_string();
    }

    // Fallback to string
    try_get::<String>(row, index)
}

/// Helper to get TIME values
fn try_get_time(row: &Row, index: usize) -> String {
    if let Ok(Some(v)) = row.try_get::<_, Option<NaiveTime>>(index) {
        return v.format("%H:%M:%S%.f").to_string();
    }

    // Fallback to string
    try_get::<String>(row, index)
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// Needs `just docker-up` (d7s-test-db on localhost:5432).
    #[tokio::test]
    #[ignore = "requires docker test database"]
    async fn insert_executes_exactly_once() {
        let pg = Postgres {
            name: "test".into(),
            host: Some("localhost".into()),
            port: Some("5432".into()),
            user: "d7s_user".into(),
            database: "d7s_test".into(),
            password: "d7s_password".into(),
            tls: crate::db::tls::TlsConfig::default(),
        };
        let t = format!("d7s_once_{}", std::process::id());
        pg.execute_sql(&format!("CREATE TABLE {t} (id int)"))
            .await
            .unwrap();
        let r = pg.execute_sql(&format!("INSERT INTO {t} VALUES (1)")).await;
        let n = pg.execute_sql(&format!("SELECT count(*) FROM {t}")).await;
        let e = pg
            .execute_sql(&format!("SELECT * FROM {t} WHERE id = 99"))
            .await;
        pg.execute_sql(&format!("DROP TABLE {t}")).await.unwrap();
        assert_eq!(r.unwrap()[0].values[0], "Affected rows: 1");
        assert_eq!(n.unwrap()[0].values[0], "1");
        assert!(e.unwrap().is_empty());
    }

    /// Needs `just docker-up`. Exercises the real tokio-postgres path of
    /// `$1::text::regclass` with a quote in the table name.
    #[tokio::test]
    #[ignore = "requires docker test database"]
    async fn table_size_and_indexes_with_odd_names() {
        let pg = Postgres {
            name: "test".into(),
            host: Some("localhost".into()),
            port: Some("5432".into()),
            user: "d7s_user".into(),
            database: "d7s_test".into(),
            password: "d7s_password".into(),
            tls: crate::db::tls::TlsConfig::default(),
        };
        let pid = std::process::id();
        let odd = format!("Odd'Tbl_{pid}");
        let plain = format!("d7s_size_{pid}");
        for t in [&odd, &plain] {
            pg.execute_sql(&format!(
                "CREATE TABLE {} (id int PRIMARY KEY)",
                pg_quote_ident(t)
            ))
            .await
            .unwrap();
        }
        let mut got = Vec::new();
        for t in [&odd, &plain] {
            let size = pg.get_table_size("public", t).await;
            let idx = pg.get_table_index_names("public", t).await;
            got.push((size, idx));
        }
        for t in [&odd, &plain] {
            pg.execute_sql(&format!("DROP TABLE {}", pg_quote_ident(t)))
                .await
                .unwrap();
        }
        for ((size, idx), t) in got.into_iter().zip([&odd, &plain]) {
            let size = size.unwrap().unwrap();
            assert!(size.ends_with("kB") || size.ends_with("bytes"), "{size}");
            assert_eq!(idx.unwrap(), vec![format!("{t}_pkey")]);
        }
    }
}
