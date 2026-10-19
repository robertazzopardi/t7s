//! Session query log: every statement d7s itself sends to a database.
//!
//! Recorded at one choke point, the [`LoggedDatabase`] decorator around the
//! [`Database`] trait, so Postgres and `SQLite` (and any future backend) are
//! covered without per-caller changes. Only SQL text / labels are stored;
//! never passwords, connection strings or edited cell values.

use alloc::{collections::VecDeque, sync::Arc};
use core::{
    cell::RefCell,
    error::Error,
    fmt::{self, Display, Formatter},
    future::Future,
    time::Duration,
};
use std::{
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use k9tui::{theme::error, widgets::table::TableData};
use ratatui::style::Style;

use super::{
    Column, Database, DatabaseInfo, DbRowId, Schema, Table, TableDataPage,
    TableRow,
};

/// Ring buffer capacity.
pub const QUERY_LOG_CAPACITY: usize = 500;
const SQL_DISPLAY_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryOrigin {
    User,
    Watch,
    Activity,
    Metadata,
}

impl Display for QueryOrigin {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::User => "user",
            Self::Watch => "watch",
            Self::Activity => "activity",
            Self::Metadata => "metadata",
        })
    }
}

#[derive(Debug, Clone)]
pub struct QueryLogEntry {
    pub at: SystemTime,
    pub origin: QueryOrigin,
    pub duration: Duration,
    pub sql: String,
    /// `Ok(rows)` or `Err(message)`.
    pub outcome: Result<usize, String>,
}

impl TableData for QueryLogEntry {
    fn title() -> &'static str {
        "Query log"
    }

    fn ref_array(&self) -> Vec<String> {
        let secs = self
            .at
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let day = secs % 86_400;
        let outcome = match &self.outcome {
            Ok(n) => format!("{n} {}", if *n == 1 { "row" } else { "rows" }),
            Err(e) => format!("ERR {}", one_line(e)),
        };
        vec![
            format!("{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60),
            self.origin.to_string(),
            format!("{}ms", self.duration.as_millis()),
            outcome,
            clip_chars(&one_line(&self.sql), SQL_DISPLAY_CHARS),
        ]
    }

    fn num_columns(&self) -> usize {
        5
    }

    fn cols() -> Vec<&'static str> {
        vec!["Time (UTC)", "Origin", "Duration", "Rows/Error", "SQL"]
    }

    fn fill_column() -> Option<usize> {
        Some(4)
    }

    fn cell_style(&self, column: usize) -> Option<Style> {
        if column == 3 && self.outcome.is_err() {
            return Some(error());
        }
        None
    }
}

/// At most `max` chars (never byte-slices), ending in `…` when cut.
fn clip_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

tokio::task_local! {
    /// SQL texts reported by the backend while a decorated call is running.
    static SQL_SINK: RefCell<Vec<String>>;
}

/// Called by backends right before sending a statement. Records the SQL
/// text only (placeholders, never bound values) for the decorated call in
/// progress; a no-op when the backend is not wrapped in a [`LoggedDatabase`].
pub fn report_sql(sql: &str) {
    let _ = SQL_SINK.try_with(|s| s.borrow_mut().push(sql.to_string()));
}

#[derive(Default)]
struct Inner {
    entries: VecDeque<QueryLogEntry>,
    /// Tag applied to the next `execute_sql` calls (set by the app).
    origin: Option<QueryOrigin>,
}

/// Shared, bounded, cheap-to-clone handle.
#[derive(Clone, Default)]
pub struct QueryLog(Arc<Mutex<Inner>>);

impl QueryLog {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn push(&self, entry: QueryLogEntry) {
        let mut g = self.lock();
        if g.entries.len() >= QUERY_LOG_CAPACITY {
            g.entries.pop_front();
        }
        g.entries.push_back(entry);
    }

    /// Origin used for subsequent `execute_sql` calls (default `User`).
    pub fn set_origin(&self, origin: QueryOrigin) {
        self.lock().origin = Some(origin);
    }

    fn sql_origin(&self) -> QueryOrigin {
        self.lock().origin.unwrap_or(QueryOrigin::User)
    }

    /// Newest first.
    #[must_use]
    pub fn snapshot(&self) -> Vec<QueryLogEntry> {
        self.lock().entries.iter().rev().cloned().collect()
    }

    /// Wrap a backend so everything it runs is recorded here.
    #[must_use]
    pub fn wrap(&self, inner: Box<dyn Database>) -> Box<dyn Database> {
        Box::new(LoggedDatabase {
            inner,
            log: self.clone(),
        })
    }
}

/// Decorator recording every [`Database`] call into a [`QueryLog`].
pub struct LoggedDatabase {
    inner: Box<dyn Database>,
    log: QueryLog,
}

impl LoggedDatabase {
    async fn record<T, F>(
        &self,
        origin: QueryOrigin,
        label: String,
        count: fn(&T) -> usize,
        fut: F,
    ) -> Result<T, Box<dyn Error>>
    where
        F: Future<Output = Result<T, Box<dyn Error>>>,
    {
        let (at, start) = (SystemTime::now(), Instant::now());
        // Real SQL reported by the backend; fall back to the label.
        let (res, reported) = SQL_SINK
            .scope(RefCell::default(), async {
                let res = fut.await;
                (res, SQL_SINK.with(RefCell::take))
            })
            .await;
        let sql = if reported.is_empty() {
            label
        } else {
            reported.join("; ")
        };
        self.log.push(QueryLogEntry {
            at,
            origin,
            duration: start.elapsed(),
            sql,
            outcome: match &res {
                Ok(v) => Ok(count(v)),
                Err(e) => Err(e.to_string()),
            },
        });
        res
    }
}

#[async_trait::async_trait]
impl Database for LoggedDatabase {
    // Connectivity probe, not a query: unlogged.
    async fn test(&self) -> bool {
        self.inner.test().await
    }

    async fn execute_sql(
        &self,
        sql: &str,
    ) -> Result<Vec<TableRow>, Box<dyn Error>> {
        self.record(
            self.log.sql_origin(),
            sql.to_string(),
            Vec::len,
            self.inner.execute_sql(sql),
        )
        .await
    }

    // The methods below pass a descriptive label that is only shown when the
    // backend reports no SQL via `report_sql`; no values are included.
    async fn get_schemas(&self) -> Result<Vec<Schema>, Box<dyn Error>> {
        self.record(
            QueryOrigin::Metadata,
            "get_schemas".into(),
            Vec::len,
            self.inner.get_schemas(),
        )
        .await
    }

    async fn get_tables(
        &self,
        schema_name: &str,
    ) -> Result<Vec<Table>, Box<dyn Error>> {
        self.record(
            QueryOrigin::Metadata,
            format!("get_tables {schema_name}"),
            Vec::len,
            self.inner.get_tables(schema_name),
        )
        .await
    }

    async fn get_columns(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<Column>, Box<dyn Error>> {
        self.record(
            QueryOrigin::Metadata,
            format!("get_columns {schema_name}.{table_name}"),
            Vec::len,
            self.inner.get_columns(schema_name, table_name),
        )
        .await
    }

    async fn get_table_data_page(
        &self,
        schema_name: &str,
        table_name: &str,
        offset: u64,
        limit: u32,
    ) -> Result<TableDataPage, Box<dyn Error>> {
        self.record(
            QueryOrigin::Metadata,
            format!(
                "get_table_data_page {schema_name}.{table_name} offset={offset} limit={limit}"
            ),
            |p: &TableDataPage| p.rows.len(),
            self.inner
                .get_table_data_page(schema_name, table_name, offset, limit),
        )
        .await
    }

    async fn get_primary_key_columns(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<String>, Box<dyn Error>> {
        self.record(
            QueryOrigin::Metadata,
            format!("get_primary_key_columns {schema_name}.{table_name}"),
            Vec::len,
            self.inner.get_primary_key_columns(schema_name, table_name),
        )
        .await
    }

    async fn update_table_cell(
        &self,
        schema_name: &str,
        table_name: &str,
        set_column: &str,
        new_value: &str,
        primary_key: &[(String, String)],
        row_id_fallback: Option<DbRowId>,
    ) -> Result<u64, Box<dyn Error>> {
        self.record(
            QueryOrigin::User,
            format!(
                "update_table_cell {schema_name}.{table_name}.{set_column}"
            ),
            |n: &u64| usize::try_from(*n).unwrap_or(usize::MAX),
            self.inner.update_table_cell(
                schema_name,
                table_name,
                set_column,
                new_value,
                primary_key,
                row_id_fallback,
            ),
        )
        .await
    }

    async fn insert_table_row(
        &self,
        schema_name: &str,
        table_name: &str,
        values: &[String],
    ) -> Result<u64, Box<dyn Error>> {
        self.record(
            QueryOrigin::User,
            format!("insert_table_row {schema_name}.{table_name}"),
            |n: &u64| usize::try_from(*n).unwrap_or(usize::MAX),
            self.inner.insert_table_row(schema_name, table_name, values),
        )
        .await
    }

    async fn delete_table_row(
        &self,
        schema_name: &str,
        table_name: &str,
        primary_key: &[(String, String)],
        row_id_fallback: Option<DbRowId>,
    ) -> Result<u64, Box<dyn Error>> {
        self.record(
            QueryOrigin::User,
            format!("delete_table_row {schema_name}.{table_name}"),
            |n: &u64| usize::try_from(*n).unwrap_or(usize::MAX),
            self.inner.delete_table_row(
                schema_name,
                table_name,
                primary_key,
                row_id_fallback,
            ),
        )
        .await
    }

    async fn get_table_row_count(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<u64, Box<dyn Error>> {
        self.record(
            QueryOrigin::Metadata,
            format!("get_table_row_count {schema_name}.{table_name}"),
            |_: &u64| 1,
            self.inner.get_table_row_count(schema_name, table_name),
        )
        .await
    }

    async fn get_table_index_names(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Vec<String>, Box<dyn Error>> {
        self.record(
            QueryOrigin::Metadata,
            format!("get_table_index_names {schema_name}.{table_name}"),
            Vec::len,
            self.inner.get_table_index_names(schema_name, table_name),
        )
        .await
    }

    async fn get_table_size(
        &self,
        schema_name: &str,
        table_name: &str,
    ) -> Result<Option<String>, Box<dyn Error>> {
        self.record(
            QueryOrigin::Metadata,
            format!("get_table_size {schema_name}.{table_name}"),
            |size: &Option<String>| usize::from(size.is_some()),
            self.inner.get_table_size(schema_name, table_name),
        )
        .await
    }

    async fn get_databases(&self) -> Result<Vec<DatabaseInfo>, Box<dyn Error>> {
        self.record(
            QueryOrigin::Metadata,
            "get_databases".into(),
            Vec::len,
            self.inner.get_databases(),
        )
        .await
    }
}

#[cfg(test)]
pub mod tests {
    use std::{env::temp_dir, fs::remove_file, process::id, thread::current};

    use rusqlite::Connection;
    use tokio::task::yield_now;

    use super::*;
    use crate::{
        app::ACTIVITY_QUERY,
        db,
        db::{postgres::Postgres, sqlite::Sqlite, tls::TlsConfig},
    };

    fn entry(sql: &str) -> QueryLogEntry {
        QueryLogEntry {
            at: UNIX_EPOCH,
            origin: QueryOrigin::User,
            duration: Duration::from_millis(3),
            sql: sql.to_string(),
            outcome: Ok(1),
        }
    }

    /// Unique temp `SQLite` file path (each `Sqlite` call opens a fresh
    /// connection, so `:memory:` would not persist between calls).
    pub fn temp_sqlite(tag: &str) -> Sqlite {
        let path = temp_dir().join(format!(
            "d7s-test-{tag}-{}-{:?}.db",
            id(),
            current().id()
        ));
        let _ = remove_file(&path);
        Sqlite {
            name: tag.to_string(),
            path: path.to_string_lossy().into_owned(),
        }
    }

    #[test]
    fn ring_buffer_caps_and_orders_newest_first() {
        let log = QueryLog::default();
        for i in 0..QUERY_LOG_CAPACITY + 10 {
            log.push(entry(&format!("q{i}")));
        }
        let snap = log.snapshot();
        assert_eq!(snap.len(), QUERY_LOG_CAPACITY);
        assert_eq!(snap.first().map(|e| e.sql.as_str()), Some("q509"));
        // oldest 10 evicted
        assert_eq!(snap.last().map(|e| e.sql.as_str()), Some("q10"));
    }

    #[test]
    fn row_formatting_truncates_and_flattens_sql() {
        let long = format!("SELECT\n  {}", "x".repeat(500));
        let cells = entry(&long).ref_array();
        assert_eq!(cells.first().map(String::as_str), Some("00:00:00"));
        assert_eq!(cells.get(2).map(String::as_str), Some("3ms"));
        let sql = cells.get(4).cloned().unwrap_or_default();
        assert_eq!(sql.chars().count(), SQL_DISPLAY_CHARS);
        assert!(sql.starts_with("SELECT xxx"));
    }

    #[tokio::test]
    async fn decorator_records_user_watch_error_and_metadata() {
        let log = QueryLog::default();
        let sqlite = temp_sqlite("decorator");
        // Seed directly: Sqlite::execute_sql re-runs row-less statements.
        Connection::open(&sqlite.path)
            .unwrap()
            .execute_batch("CREATE TABLE t(a); INSERT INTO t VALUES (1),(2);")
            .unwrap();
        let db = log.wrap(Box::new(sqlite));

        db.execute_sql("SELECT 1").await.unwrap();
        log.set_origin(QueryOrigin::Watch);
        db.execute_sql("SELECT * FROM t").await.unwrap();
        assert!(db.execute_sql("SELEKT nope").await.is_err());
        db.get_tables("sqlite_schema").await.unwrap();
        assert!(db.test().await); // probe is not logged

        let snap = log.snapshot();
        let got: Vec<_> =
            snap.iter().map(|e| (e.origin, e.outcome.is_ok())).collect();
        assert_eq!(
            got,
            vec![
                (QueryOrigin::Metadata, true),
                (QueryOrigin::Watch, false),
                (QueryOrigin::Watch, true),
                (QueryOrigin::User, true),
            ]
        );
        assert_eq!(snap.get(2).map(|e| e.outcome.clone()), Some(Ok(2)));
        assert_eq!(snap.get(1).map(|e| e.sql.as_str()), Some("SELEKT nope"));
    }

    #[tokio::test]
    async fn metadata_and_edits_log_real_sql_without_values() {
        let log = QueryLog::default();
        let sqlite = temp_sqlite("realsql");
        Connection::open(&sqlite.path)
            .unwrap()
            .execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT);")
            .unwrap();
        let db = log.wrap(Box::new(sqlite));
        db.insert_table_row(
            "sqlite_schema",
            "t",
            &["1".into(), "secret1".into()],
        )
        .await
        .unwrap();
        db.update_table_cell(
            "sqlite_schema",
            "t",
            "a",
            "secret2",
            &[("id".into(), "1".into())],
            None,
        )
        .await
        .unwrap();
        db.get_tables("sqlite_schema").await.unwrap();
        db.get_table_row_count("sqlite_schema", "t").await.unwrap();
        db.get_table_data_page("sqlite_schema", "t", 0, 10)
            .await
            .unwrap();
        db.get_table_index_names("sqlite_schema", "t")
            .await
            .unwrap();
        db.get_schemas().await.unwrap(); // no SQL: label fallback

        let sqls: Vec<String> =
            log.snapshot().into_iter().rev().map(|e| e.sql).collect();
        let has = |i: usize, needle: &str| {
            assert!(
                sqls.get(i).is_some_and(|s| s.contains(needle)),
                "{i}: {needle} not in {sqls:?}"
            );
        };
        has(0, "INSERT INTO \"t\" (");
        has(1, "UPDATE \"t\" SET \"a\" = CAST(?1 AS");
        has(2, "SELECT name FROM sqlite_schema WHERE type='table'");
        has(3, "SELECT COUNT(*) FROM t");
        has(4, "SELECT rowid,");
        has(5, "index_list");
        assert_eq!(sqls.get(6).map(String::as_str), Some("get_schemas"));
        assert!(sqls.iter().all(|s| !s.contains("secret")));
    }

    fn origin_entry(origin: QueryOrigin, sql: &str) -> QueryLogEntry {
        QueryLogEntry {
            origin,
            ..entry(sql)
        }
    }

    #[test]
    fn multibyte_sql_is_clipped_by_chars_not_bytes() {
        // 3-byte chars: byte slicing at 120 would panic mid-codepoint.
        let sql = "é日本😀".repeat(100);
        let cell = entry(&sql).ref_array().get(4).cloned().unwrap();
        assert_eq!(cell.chars().count(), SQL_DISPLAY_CHARS);
        assert!(cell.ends_with('…'));
        // exactly at the limit: untouched, no ellipsis
        let exact = "日".repeat(SQL_DISPLAY_CHARS);
        let cell = entry(&exact).ref_array().get(4).cloned().unwrap();
        assert_eq!(cell, exact);
        // short and empty
        assert_eq!(clip_chars("", 5), "");
        assert_eq!(clip_chars("日本語", 0), "…");
        assert_eq!(clip_chars("日本語", 2), "日…");
    }

    #[test]
    fn newlines_tabs_and_runs_of_space_flatten_to_single_spaces() {
        let cell = entry("SELECT\n\ta,\r\n   b\nFROM\t\tt\n")
            .ref_array()
            .get(4)
            .cloned()
            .unwrap();
        assert_eq!(cell, "SELECT a, b FROM t");
        let err = QueryLogEntry {
            outcome: Err("line1\n  line2\t".into()),
            ..entry("x")
        };
        assert_eq!(
            err.ref_array().get(3).map(String::as_str),
            Some("ERR line1 line2")
        );
    }

    #[test]
    fn ring_buffer_evicts_oldest_across_interleaved_origins() {
        let log = QueryLog::default();
        let origins = [
            QueryOrigin::Watch,
            QueryOrigin::Activity,
            QueryOrigin::User,
            QueryOrigin::Metadata,
        ];
        let total = QUERY_LOG_CAPACITY + 7;
        for i in 0..total {
            let o = origins.get(i % origins.len()).copied().unwrap();
            log.push(origin_entry(o, &format!("q{i}")));
        }
        let snap = log.snapshot();
        assert_eq!(snap.len(), QUERY_LOG_CAPACITY);
        // newest first, strictly descending sequence, origin preserved
        for (k, e) in snap.iter().enumerate() {
            let i = total - 1 - k;
            assert_eq!(e.sql, format!("q{i}"));
            assert_eq!(Some(e.origin), origins.get(i % origins.len()).copied());
        }
        assert_eq!(snap.last().map(|e| e.sql.as_str()), Some("q7"));
    }

    #[test]
    fn report_sql_outside_scope_is_a_noop() {
        // no panic, nothing to observe
        report_sql("SELECT 1");
        let log = QueryLog::default();
        assert_eq!(log.snapshot().len(), 0);
    }

    #[tokio::test]
    async fn reported_sql_joins_with_semicolons_and_scopes_do_not_leak() {
        let log = QueryLog::default();
        let db = LoggedDatabase {
            inner: Box::new(temp_sqlite("join")),
            log: log.clone(),
        };
        let one = db.record(
            QueryOrigin::Metadata,
            "label1".into(),
            |(): &()| 0,
            async {
                report_sql("A");
                report_sql("B");
                report_sql("C");
                Ok(())
            },
        );
        let two = db.record(
            QueryOrigin::Metadata,
            "label2".into(),
            |(): &()| 0,
            async {
                yield_now().await;
                report_sql("X");
                Ok(())
            },
        );
        let none = db.record(
            QueryOrigin::Metadata,
            "label3".into(),
            |(): &()| 0,
            async { Ok(()) },
        );
        // concurrently polled futures each get their own sink
        let (a, b, c) = tokio::join!(one, two, none);
        a.unwrap();
        b.unwrap();
        c.unwrap();
        let mut sqls: Vec<String> =
            log.snapshot().into_iter().map(|e| e.sql).collect();
        sqls.sort();
        assert_eq!(sqls, vec!["A; B; C", "X", "label3"]);
        // a later report outside any scope never lands in the log
        report_sql("late");
        assert_eq!(log.snapshot().len(), 3);
    }

    #[tokio::test]
    async fn nested_record_inner_sql_stays_with_inner_entry() {
        let log = QueryLog::default();
        let db = LoggedDatabase {
            inner: Box::new(temp_sqlite("nest")),
            log: log.clone(),
        };
        let r = db
            .record(QueryOrigin::User, "outer".into(), |(): &()| 0, async {
                report_sql("o1");
                db.record(
                    QueryOrigin::Metadata,
                    "inner".into(),
                    |(): &()| 0,
                    async {
                        report_sql("i1");
                        Ok(())
                    },
                )
                .await?;
                report_sql("o2");
                Ok(())
            })
            .await;
        r.unwrap();
        let sqls: Vec<String> =
            log.snapshot().into_iter().rev().map(|e| e.sql).collect();
        assert_eq!(sqls, vec!["i1", "o1; o2"]);
    }

    #[tokio::test]
    async fn edit_paths_never_log_cell_or_key_values() {
        let log = QueryLog::default();
        let sqlite = temp_sqlite("novalues");
        Connection::open(&sqlite.path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE t(k TEXT PRIMARY KEY, a TEXT, b TEXT);",
            )
            .unwrap();
        let db = log.wrap(Box::new(sqlite));
        let pk = vec![("k".to_string(), "secretkey".to_string())];
        db.insert_table_row(
            "sqlite_schema",
            "t",
            &["secretkey".into(), "secretA".into(), "secretB".into()],
        )
        .await
        .unwrap();
        db.update_table_cell("sqlite_schema", "t", "a", "secretNew", &pk, None)
            .await
            .unwrap();
        db.update_table_cell(
            "sqlite_schema",
            "t",
            "b",
            "secretFallback",
            &[],
            Some(db::DbRowId::Sqlite(1)),
        )
        .await
        .unwrap();
        db.delete_table_row("sqlite_schema", "t", &pk, None)
            .await
            .unwrap();
        let sqls: Vec<String> =
            log.snapshot().into_iter().rev().map(|e| e.sql).collect();
        assert!(
            sqls.iter().any(|s| s.contains("INSERT INTO"))
                && sqls.iter().any(|s| s.contains("UPDATE"))
                && sqls.iter().any(|s| s.contains("DELETE FROM")),
            "{sqls:?}"
        );
        assert!(
            sqls.iter().all(|s| !s.to_lowercase().contains("secret")),
            "{sqls:?}"
        );
    }

    /// Needs `just docker-up`: bound values must not appear in logged SQL.
    #[tokio::test]
    #[ignore = "requires docker test database"]
    async fn pg_edit_paths_never_log_cell_or_key_values() {
        let log = QueryLog::default();
        let pg = Postgres {
            name: "test".into(),
            host: Some("localhost".into()),
            port: Some("5432".into()),
            user: "d7s_user".into(),
            database: "d7s_test".into(),
            password: "d7s_password".into(),
            tls: TlsConfig::default(),
        };
        let t = format!("d7s_qlog_{}", id());
        pg.execute_sql(&format!(
            "CREATE TABLE {t} (k text PRIMARY KEY, a text)"
        ))
        .await
        .unwrap();
        let db = log.wrap(Box::new(pg));
        let pk = vec![("k".to_string(), "secretkey".to_string())];
        let r1 = db
            .insert_table_row(
                "public",
                &t,
                &["secretkey".into(), "secretA".into()],
            )
            .await;
        let r2 = db
            .update_table_cell("public", &t, "a", "secretNew", &pk, None)
            .await;
        let r3 = db.delete_table_row("public", &t, &pk, None).await;
        db.execute_sql(&format!("DROP TABLE {t}")).await.unwrap();
        r1.unwrap();
        r2.unwrap();
        r3.unwrap();
        let sqls: Vec<String> =
            log.snapshot().into_iter().rev().map(|e| e.sql).collect();
        for kw in ["INSERT INTO", "UPDATE", "DELETE FROM"] {
            assert!(sqls.iter().any(|s| s.contains(kw)), "{kw}: {sqls:?}");
        }
        assert!(
            sqls.iter().all(|s| !s.to_lowercase().contains("secret")),
            "{sqls:?}"
        );
    }

    /// Needs `just docker-up` (Postgres on localhost:5432):
    /// `cargo test -p d7s -- --ignored pg_activity`
    #[tokio::test]
    #[ignore = "requires docker compose Postgres"]
    async fn pg_activity_query_is_logged_and_sees_itself() {
        use crate::db::postgres::Postgres;

        let log = QueryLog::default();
        let db = log.wrap(Box::new(Postgres {
            name: "it".into(),
            host: Some("localhost".into()),
            port: Some("5432".into()),
            user: "d7s_user".into(),
            database: "d7s_test".into(),
            password: "d7s_password".into(),
            tls: TlsConfig::default(),
        }));
        log.set_origin(QueryOrigin::Activity);
        let rows = db.execute_sql(ACTIVITY_QUERY).await.unwrap();
        assert!(
            rows.iter().any(|r| r
                .values
                .iter()
                .any(|v| v.contains("pg_stat_activity")))
        );
        let snap = log.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap.first().map(|e| e.origin), Some(QueryOrigin::Activity));
        assert_eq!(
            snap.first().map(|e| e.outcome.clone()),
            Some(Ok(rows.len()))
        );
        // metadata path on Postgres too
        db.get_schemas().await.unwrap();
        assert_eq!(log.snapshot().len(), 2);
        assert!(
            log.snapshot()
                .first()
                .is_some_and(|e| e.sql.contains("information_schema.schemata"))
        );
    }
}
