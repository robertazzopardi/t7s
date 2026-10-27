use core::future::Future;
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use crate::{
    db::{
        Column, Database, DatabaseInfo, Schema, Table, TableDataPage,
        connection::{Connection, ConnectionStatus},
    },
    ui::widgets::describe_content::DescribeRow,
};

/// Successful SQL execute payload for the UI thread.
#[derive(Debug, Clone)]
pub struct SqlExecuteOk {
    pub cols: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// After a successful table mutation, refetch the current page if still on that table.
#[derive(Debug, Clone)]
pub struct TableMutationReload {
    pub schema: String,
    pub table: String,
    pub success_status: Option<String>,
}

/// Column + PK metadata for table data editing (off the UI thread).
#[derive(Debug, Clone)]
pub struct TableColumnMetaOk {
    pub columns: Vec<Column>,
    pub pk_names: Vec<String>,
}

/// Table page fetch completed off the UI thread.
#[derive(Debug, Clone)]
pub struct TablePageLoadOk {
    pub schema: String,
    pub table: String,
    pub window_start: u64,
    pub page_size: u32,
    pub total_rows: Option<u64>,
    pub page: TableDataPage,
    pub table_columns: Option<Vec<Column>>,
    pub table_pk_names: Option<Vec<String>>,
    pub record_recent: bool,
    pub select_row_at: Option<usize>,
    pub success_status: Option<String>,
}

/// Update pushed from a background task into the app's event loop.
pub enum BgEvent {
    /// Background action finished successfully.
    ActionDone {
        key: String,
        message: String,
        reload_table: Option<TableMutationReload>,
    },
    /// Background action failed.
    ActionFailed {
        key: String,
        message: String,
    },
    /// SQL execute finished (success or error).
    SqlFinished {
        key: String,
        sql: String,
        record_history: bool,
        result: Result<SqlExecuteOk, String>,
    },
    DatabasesLoaded {
        key: String,
        databases: Vec<DatabaseInfo>,
    },
    SchemasLoaded {
        key: String,
        schemas: Vec<Schema>,
    },
    TablesLoaded {
        key: String,
        schema: String,
        tables: Vec<Table>,
    },
    ColumnsLoaded {
        key: String,
        schema: String,
        table: String,
        columns: Vec<Column>,
    },
    TableColumnMetaLoaded {
        key: String,
        schema: String,
        table: String,
        result: Result<TableColumnMetaOk, String>,
    },
    TablePageLoaded {
        key: String,
        schema: String,
        table: String,
        record_recent: bool,
        result: Result<TablePageLoadOk, String>,
    },
    ExplorerLoadFailed {
        key: String,
        resource: String,
        schema: Option<String>,
        table: Option<String>,
        message: String,
    },
    /// Database connect succeeded off the UI thread.
    Connected {
        key: String,
        connection: Connection,
        database: Arc<dyn Database>,
    },
    /// Database connect failed off the UI thread.
    ConnectFailed {
        key: String,
        message: String,
    },
    /// Keyring had no password; UI should prompt (connect key cleared).
    ConnectPasswordRequired {
        key: String,
        connection: Connection,
        prompt: String,
    },
    /// Per-connection health ping result.
    ConnectionStatus {
        key: String,
        name: String,
        status: ConnectionStatus,
    },
    /// User selected a database from the list; reconnect succeeded.
    DatabaseSelected {
        key: String,
        database_name: String,
        database: Arc<dyn Database>,
    },
    /// Reconnect to the chosen database failed.
    DatabaseSelectFailed {
        key: String,
    },
    /// Describe panel rows built off the UI thread.
    DescribeRowsLoaded {
        key: String,
        target: DescribeTarget,
        rows: Vec<DescribeRow>,
    },
    /// Connection modal test finished.
    ConnectionTestResult {
        key: String,
        success: bool,
        message: String,
    },
}

/// What the user had selected when describe was requested (stale guard).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescribeTarget {
    Connection {
        name: String,
    },
    Table {
        schema: String,
        name: String,
    },
    Column {
        schema: String,
        table: String,
        column: String,
    },
    TableDataRow {
        schema: String,
        table: String,
        row_index: usize,
    },
}

/// `tokio::spawn` for work holding an in-flight `key`: if the task panics, an
/// `ActionFailed` still releases the key so the resource is not left busy.
pub fn spawn_keyed<F>(tx: UnboundedSender<BgEvent>, key: String, task: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    let handle = tokio::spawn(task);
    tokio::spawn(async move {
        if handle.await.is_err() {
            let _ = tx.send(BgEvent::ActionFailed {
                key,
                message: "Background task failed unexpectedly".into(),
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;

    #[tokio::test]
    async fn spawn_keyed_releases_key_when_task_panics() {
        let (tx, mut rx) = unbounded_channel();
        spawn_keyed(tx, "k".into(), async { panic!("boom") });
        match rx.recv().await {
            Some(BgEvent::ActionFailed { key, .. }) => assert_eq!(key, "k"),
            _ => panic!("expected ActionFailed"),
        }
    }
}
