use mysql_async::{
    Conn, OptsBuilder, Row, Value,
    consts::{ColumnFlags, ColumnType},
    prelude::Queryable,
};

use crate::db::{
    Column, Database, DatabaseInfo, DbRowId, Schema, Table, TableData,
    TableDataPage, TableRow, should_omit_for_insert_default,
};

/// Databases hidden from the database list (server internals).
const SYSTEM_DATABASES: &str =
    "'information_schema', 'mysql', 'performance_schema', 'sys'";

type DbResult<T> = Result<T, Box<dyn std::error::Error>>;

/// Backtick-quote an identifier, doubling embedded backticks.
fn my_quote_ident(ident: &str) -> String {
    format!("`{}`", ident.replace('`', "``"))
}

fn qualified(schema: &str, table: &str) -> String {
    format!("{}.{}", my_quote_ident(schema), my_quote_ident(table))
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    #[allow(clippy::cast_precision_loss)]
    let mut size = bytes as f64;
    let mut unit = "B";
    for next in UNITS.iter().skip(1) {
        if size < 1024.0 {
            break;
        }
        size /= 1024.0;
        unit = next;
    }
    if unit == "B" {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {unit}")
    }
}

/// Binary-charset (63) blob/binary/bit columns are shown as a size label, not raw bytes.
fn is_binary_column(col: &mysql_async::Column) -> bool {
    let binary_type = matches!(
        col.column_type(),
        ColumnType::MYSQL_TYPE_TINY_BLOB
            | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
            | ColumnType::MYSQL_TYPE_LONG_BLOB
            | ColumnType::MYSQL_TYPE_BLOB
            | ColumnType::MYSQL_TYPE_VAR_STRING
            | ColumnType::MYSQL_TYPE_STRING
            | ColumnType::MYSQL_TYPE_VARCHAR
    );
    (binary_type
        && col.character_set() == 63
        && col.flags().contains(ColumnFlags::BINARY_FLAG))
        || matches!(
            col.column_type(),
            ColumnType::MYSQL_TYPE_BIT | ColumnType::MYSQL_TYPE_GEOMETRY
        )
}

/// Text-protocol cells arrive as `Value::Bytes`; other variants are handled for safety.
fn cell_to_string(value: &Value, col: &mysql_async::Column) -> String {
    match value {
        Value::NULL => "NULL".to_string(),
        Value::Bytes(b) if is_binary_column(col) => {
            format!("<{} bytes>", b.len())
        }
        Value::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
        other @ (Value::Int(_)
        | Value::UInt(_)
        | Value::Float(_)
        | Value::Double(_)
        | Value::Date(..)
        | Value::Time(..)) => other.as_sql(true).trim_matches('\'').to_string(),
    }
}

fn row_to_strings(row: &Row) -> Vec<String> {
    row.columns_ref()
        .iter()
        .enumerate()
        .map(|(i, col)| {
            row.as_ref(i)
                .map_or_else(|| "NULL".to_string(), |v| cell_to_string(v, col))
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct Mysql {
    pub name: String,
    pub host: Option<String>,
    pub port: Option<String>,
    pub user: String,
    /// Empty = no default database (server-level connection).
    pub database: String,
    pub password: String,
}

impl TableData for Mysql {
    fn title() -> &'static str {
        "MySQL"
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

impl Mysql {
    async fn get_connection(&self) -> DbResult<Conn> {
        let port = self
            .port
            .as_deref()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(3306);
        let opts = OptsBuilder::default()
            .ip_or_hostname(self.host.as_deref().unwrap_or("localhost"))
            .tcp_port(port)
            .user(Some(&self.user))
            .pass(Some(&self.password))
            // Report matched (not just changed) rows so a no-op edit isn't "0 rows".
            .client_found_rows(true)
            .db_name((!self.database.is_empty()).then_some(&self.database));
        Ok(Conn::new(opts).await?)
    }

    /// Build a `WHERE` clause (and its params) from primary-key pairs.
    fn pk_where(primary_key: &[(String, String)]) -> (String, Vec<Value>) {
        let clause = primary_key
            .iter()
            .map(|(k, _)| format!("{} = ?", my_quote_ident(k)))
            .collect::<Vec<_>>()
            .join(" AND ");
        let params = primary_key
            .iter()
            .map(|(_, v)| Value::from(v.as_str()))
            .collect();
        (clause, params)
    }

    async fn data_plus_index_bytes(
        conn: &mut Conn,
        schema_name: &str,
        table_name: &str,
    ) -> DbResult<Option<u64>> {
        let q = "SELECT CAST(COALESCE(data_length, 0) + COALESCE(index_length, 0) AS UNSIGNED) \
                 FROM information_schema.tables WHERE table_schema = ? AND table_name = ?";
        Ok(conn.exec_first(q, (schema_name, table_name)).await?)
    }
}

#[async_trait::async_trait]
impl Database for Mysql {
    async fn test(&self) -> bool {
        self.get_connection().await.is_ok()
    }

    async fn execute_sql(&self, sql: &str) -> DbResult<Vec<TableRow>> {
        let mut conn = self.get_connection().await?;
        let mut res = conn.query_iter(sql).await?;
        let columns = res.columns();
        let Some(columns) = columns.filter(|c| !c.is_empty()) else {
            return Ok(vec![TableRow {
                values: vec![format!("Affected rows: {}", res.affected_rows())],
                column_names: vec!["Result".to_string()],
            }]);
        };
        let column_names: Vec<String> =
            columns.iter().map(|c| c.name_str().into_owned()).collect();
        let rows: Vec<Row> = res.collect().await?;
        Ok(rows
            .iter()
            .map(|row| TableRow {
                values: row_to_strings(row),
                column_names: column_names.clone(),
            })
            .collect())
    }

    /// MySQL has no schema level below a database: the connected database is the only schema.
    async fn get_schemas(&self) -> DbResult<Vec<Schema>> {
        let mut conn = self.get_connection().await?;
        let current: Option<Option<String>> =
            conn.query_first("SELECT DATABASE()").await?;
        Ok(current
            .flatten()
            .map(|name| Schema {
                name,
                owner: self.user.clone(),
            })
            .into_iter()
            .collect())
    }

    async fn get_tables(&self, schema_name: &str) -> DbResult<Vec<Table>> {
        let mut conn = self.get_connection().await?;
        let q = "SELECT table_name, table_schema, \
                 CAST(COALESCE(data_length, 0) + COALESCE(index_length, 0) AS UNSIGNED) \
                 FROM information_schema.tables \
                 WHERE table_schema = ? AND table_type = 'BASE TABLE' \
                 ORDER BY table_name";
        let rows: Vec<(String, String, u64)> =
            conn.exec(q, (schema_name,)).await?;
        Ok(rows
            .into_iter()
            .map(|(name, schema, size)| Table {
                name,
                schema,
                size: Some(format_bytes(size)),
            })
            .collect())
    }

    async fn get_columns(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> DbResult<Vec<Column>> {
        let mut conn = self.get_connection().await?;
        let q = "SELECT column_name, column_type, is_nullable, column_default, extra, column_comment \
                 FROM information_schema.columns \
                 WHERE table_schema = ? AND table_name = ? \
                 ORDER BY ordinal_position";
        let rows: Vec<(
            String,
            String,
            String,
            Option<String>,
            String,
            String,
        )> = conn.exec(q, (schema_name, table_name)).await?;
        Ok(rows
            .into_iter()
            .map(|(name, data_type, nullable, default, extra, comment)| {
                // MariaDB reports "no default" as the string 'NULL'.
                let default =
                    default.filter(|d| !d.eq_ignore_ascii_case("null"));
                // Surface auto-increment as the default so blank inserts omit the column.
                let default = default.or_else(|| {
                    extra
                        .to_lowercase()
                        .contains("auto_increment")
                        .then(|| "auto_increment".to_string())
                });
                Column {
                    name,
                    data_type,
                    is_nullable: nullable == "YES",
                    default_value: default,
                    description: (!comment.is_empty()).then_some(comment),
                }
            })
            .collect())
    }

    async fn get_table_data_page(
        &self,
        schema_name: &str,
        table_name: &str,
        offset: u64,
        limit: u32,
    ) -> DbResult<TableDataPage> {
        let mut conn = self.get_connection().await?;
        // Numeric literals only, so the text protocol (all cells as strings) is used.
        let q = format!(
            "SELECT * FROM {} LIMIT {limit} OFFSET {offset}",
            qualified(schema_name, table_name)
        );
        let mut res = conn.query_iter(q).await?;
        let column_names = res
            .columns()
            .map(|cols| {
                cols.iter().map(|c| c.name_str().into_owned()).collect()
            })
            .unwrap_or_default();
        let rows: Vec<Row> = res.collect().await?;
        let data: Vec<Vec<String>> = rows.iter().map(row_to_strings).collect();
        // No rowid/ctid equivalent: tables without a primary key stay read-only.
        let row_ids: Vec<Option<DbRowId>> = vec![None; data.len()];
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
    ) -> DbResult<Vec<String>> {
        let mut conn = self.get_connection().await?;
        let q = "SELECT column_name FROM information_schema.key_column_usage \
                 WHERE constraint_name = 'PRIMARY' AND table_schema = ? AND table_name = ? \
                 ORDER BY ordinal_position";
        Ok(conn.exec(q, (schema_name, table_name)).await?)
    }

    async fn update_table_cell(
        &self,
        schema_name: &str,
        table_name: &str,
        set_column: &str,
        new_value: &str,
        primary_key: &[(String, String)],
        _row_id_fallback: Option<DbRowId>,
    ) -> DbResult<u64> {
        if primary_key.is_empty() {
            return Err(
                "Cannot update row: table has no primary key (MySQL has no row address fallback)"
                    .into(),
            );
        }
        let mut conn = self.get_connection().await?;
        let (clause, mut pk_params) = Self::pk_where(primary_key);
        let sql = format!(
            "UPDATE {} SET {} = ? WHERE {clause}",
            qualified(schema_name, table_name),
            my_quote_ident(set_column),
        );
        let mut params = vec![Value::from(new_value)];
        params.append(&mut pk_params);
        let res = conn.exec_iter(sql, params).await?;
        Ok(res.affected_rows())
    }

    async fn insert_table_row(
        &self,
        schema_name: &str,
        table_name: &str,
        values: &[String],
    ) -> DbResult<u64> {
        let columns = self.get_columns(schema_name, table_name).await?;
        if values.len() != columns.len() {
            return Err("INSERT column count does not match table.".into());
        }
        let mut col_list = Vec::new();
        let mut params: Vec<Value> = Vec::new();
        let mut placeholders = Vec::new();
        for (c, raw) in columns.iter().zip(values) {
            if should_omit_for_insert_default(c, raw, false, false) {
                continue;
            }
            col_list.push(my_quote_ident(&c.name));
            if raw.trim().is_empty() || raw.eq_ignore_ascii_case("null") {
                if !c.is_nullable {
                    return Err(format!(
                        "Column \"{}\" is NOT NULL and has no value or default in the form.",
                        c.name
                    )
                    .into());
                }
                placeholders.push("NULL");
            } else {
                placeholders.push("?");
                params.push(Value::from(raw.as_str()));
            }
        }
        let tgt = qualified(schema_name, table_name);
        let sql = format!(
            "INSERT INTO {tgt} ({}) VALUES ({})",
            col_list.join(", "),
            placeholders.join(", ")
        );
        let mut conn = self.get_connection().await?;
        let res = conn.exec_iter(sql, params).await?;
        Ok(res.affected_rows())
    }

    async fn delete_table_row(
        &self,
        schema_name: &str,
        table_name: &str,
        primary_key: &[(String, String)],
        _row_id_fallback: Option<DbRowId>,
    ) -> DbResult<u64> {
        if primary_key.is_empty() {
            return Err(
                "Cannot delete row: no primary key (MySQL has no row address fallback)"
                    .into(),
            );
        }
        let mut conn = self.get_connection().await?;
        let (clause, params) = Self::pk_where(primary_key);
        let sql = format!(
            "DELETE FROM {} WHERE {clause}",
            qualified(schema_name, table_name)
        );
        let res = conn.exec_iter(sql, params).await?;
        Ok(res.affected_rows())
    }

    async fn get_table_row_count(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> DbResult<u64> {
        let mut conn = self.get_connection().await?;
        let q = format!(
            "SELECT COUNT(*) FROM {}",
            qualified(schema_name, table_name)
        );
        Ok(conn.query_first::<u64, _>(q).await?.unwrap_or(0))
    }

    async fn get_table_index_names(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> DbResult<Vec<String>> {
        let mut conn = self.get_connection().await?;
        let q = "SELECT DISTINCT index_name FROM information_schema.statistics \
                 WHERE table_schema = ? AND table_name = ? ORDER BY index_name";
        Ok(conn.exec(q, (schema_name, table_name)).await?)
    }

    async fn get_table_size(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> DbResult<Option<String>> {
        let mut conn = self.get_connection().await?;
        Ok(
            Self::data_plus_index_bytes(&mut conn, schema_name, table_name)
                .await?
                .map(format_bytes),
        )
    }

    async fn get_databases(&self) -> DbResult<Vec<DatabaseInfo>> {
        let mut conn = self.get_connection().await?;
        let q = format!(
            "SELECT schema_name FROM information_schema.schemata \
             WHERE schema_name NOT IN ({SYSTEM_DATABASES}) ORDER BY schema_name"
        );
        let names: Vec<String> = conn.query(q).await?;
        Ok(names
            .into_iter()
            .map(|name| DatabaseInfo { name })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_ident_escapes_backticks() {
        assert_eq!(my_quote_ident("a`b"), "`a``b`");
    }

    #[test]
    fn bytes_are_humanised() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(16 * 1024), "16.0 KB");
    }

    /// Needs a MySQL/MariaDB server: `just docker-up` (mysql on 3306, mariadb on 3307),
    /// or set `D7S_MYSQL_PORT`.
    #[tokio::test]
    #[ignore = "requires docker test database"]
    #[allow(clippy::indexing_slicing, clippy::too_many_lines)]
    async fn crud_round_trip() {
        let my = Mysql {
            name: "test".into(),
            host: Some("localhost".into()),
            port: Some(
                std::env::var("D7S_MYSQL_PORT")
                    .unwrap_or_else(|_| "3306".into()),
            ),
            user: "root".into(),
            database: "d7s_test".into(),
            password: "d7s_password".into(),
        };
        let t = format!("d7s_crud_{}", std::process::id());
        my.execute_sql(&format!(
            "CREATE TABLE `{t}` (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(20) NOT NULL, \
             note TEXT, blob_col BLOB, KEY idx_name (name))"
        ))
        .await
        .unwrap();

        assert!(my.test().await);
        assert!(
            my.get_databases()
                .await
                .unwrap()
                .iter()
                .any(|d| d.name == "d7s_test")
        );
        let schemas = my.get_schemas().await.unwrap();
        assert_eq!(schemas[0].name, "d7s_test");
        assert!(
            my.get_tables("d7s_test")
                .await
                .unwrap()
                .iter()
                .any(|x| x.name == t)
        );

        let cols = my.get_columns("d7s_test", &t).await.unwrap();
        assert_eq!(cols.len(), 4);
        assert_eq!(cols[0].default_value.as_deref(), Some("auto_increment"));
        assert!(!cols[1].is_nullable);
        assert_eq!(
            my.get_primary_key_columns("d7s_test", &t).await.unwrap(),
            vec!["id"]
        );

        // blank id (auto-increment) is omitted; blank nullable note becomes NULL
        let n = my
            .insert_table_row(
                "d7s_test",
                &t,
                &[String::new(), "alice".into(), String::new(), String::new()],
            )
            .await
            .unwrap();
        assert_eq!(n, 1);
        assert!(
            my.insert_table_row(
                "d7s_test",
                &t,
                &[String::new(), String::new(), String::new(), String::new(),],
            )
            .await
            .is_err(),
            "NOT NULL name with no value must be rejected"
        );
        my.execute_sql(&format!(
            "INSERT INTO `{t}` (name, blob_col) VALUES ('bob', X'DEADBEEF')"
        ))
        .await
        .unwrap();

        let page = my.get_table_data_page("d7s_test", &t, 0, 10).await.unwrap();
        assert_eq!(page.column_names, vec!["id", "name", "note", "blob_col"]);
        assert_eq!(page.rows[0], vec!["1", "alice", "NULL", "NULL"]);
        assert_eq!(page.rows[1][3], "<4 bytes>");
        assert_eq!(my.get_table_row_count("d7s_test", &t).await.unwrap(), 2);

        let pk = vec![("id".to_string(), "1".to_string())];
        let n = my
            .update_table_cell("d7s_test", &t, "name", "carol", &pk, None)
            .await
            .unwrap();
        assert_eq!(n, 1);
        let same = my
            .update_table_cell("d7s_test", &t, "name", "carol", &pk, None)
            .await
            .unwrap();
        assert_eq!(same, 1, "unchanged value still counts as a matched row");
        let rows = my
            .execute_sql(&format!("SELECT name FROM `{t}` WHERE id = 1"))
            .await
            .unwrap();
        assert_eq!(rows[0].values[0], "carol");

        assert!(
            my.get_table_index_names("d7s_test", &t)
                .await
                .unwrap()
                .contains(&"idx_name".to_string())
        );
        assert!(my.get_table_size("d7s_test", &t).await.unwrap().is_some());

        let n = my
            .delete_table_row("d7s_test", &t, &pk, None)
            .await
            .unwrap();
        assert_eq!(n, 1);
        my.execute_sql(&format!("DROP TABLE `{t}`")).await.unwrap();
    }

    /// Needs the seeded containers from `just docker-up` (see `docker/seed-mysql.sql`).
    #[tokio::test]
    #[ignore = "requires docker test database"]
    #[allow(clippy::indexing_slicing)]
    async fn seed_data_renders() {
        let my = Mysql {
            name: "test".into(),
            host: Some("localhost".into()),
            port: Some(
                std::env::var("D7S_MYSQL_PORT")
                    .unwrap_or_else(|_| "3306".into()),
            ),
            user: "root".into(),
            database: String::new(),
            password: "d7s_password".into(),
        };
        let dbs: Vec<String> = my
            .get_databases()
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        for expected in ["d7s_test", "shop", "edge_cases"] {
            assert!(dbs.contains(&expected.to_string()), "missing {expected}");
        }
        assert_eq!(
            my.get_table_row_count("d7s_test", "big_seed")
                .await
                .unwrap(),
            10_000
        );
        assert_eq!(
            my.get_primary_key_columns("shop", "order_items")
                .await
                .unwrap(),
            vec!["order_id", "line_no"]
        );

        let page = my
            .get_table_data_page("edge_cases", "full_types", 0, 10)
            .await
            .unwrap();
        let col = |name: &str| {
            page.column_names.iter().position(|c| c == name).unwrap()
        };
        assert_eq!(page.rows[0][col("c_bigint")], "9223372036854775807");
        assert_eq!(page.rows[0][col("c_blob")], "<4 bytes>");
        assert_eq!(page.rows[0][col("c_varbinary")], "<4 bytes>");
        assert_eq!(page.rows[0][col("c_bit")], "<1 bytes>");
        assert_eq!(page.rows[0][col("c_enum")], "green");
        assert_eq!(page.rows[0][col("c_date")], "2025-06-15");
        assert!(page.rows[0][col("c_json")].contains("\"d7s\""));
        assert!(page.rows[1].iter().skip(1).all(|v| v == "NULL"));

        let chars = my
            .get_table_data_page("edge_cases", "special_chars", 0, 20)
            .await
            .unwrap();
        assert!(chars.rows.iter().any(|r| r[1].contains("日本語 🚀")));
        // Tables needing identifier quoting still load.
        assert_eq!(
            my.get_table_data_page("edge_cases", "weird name", 0, 10)
                .await
                .unwrap()
                .rows
                .len(),
            2
        );
    }

    #[test]
    fn pk_where_builds_clause_and_params() {
        let pk = vec![("id".to_string(), "1".to_string())];
        let (clause, params) = Mysql::pk_where(&pk);
        assert_eq!(clause, "`id` = ?");
        assert_eq!(params.len(), 1);
    }
}
