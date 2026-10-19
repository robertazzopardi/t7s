use crate::db::{Column, DatabaseInfo, Schema, Table, TableDataPage};

/// Successful SQL execute payload for the UI thread.
#[derive(Debug, Clone)]
pub struct SqlExecuteOk {
    pub cols: Vec<String>,
    pub rows: Vec<Vec<String>>,
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
    #[allow(dead_code)] // Tasks 11–12
    ActionDone { key: String, message: String },
    /// Background action failed.
    #[allow(dead_code)] // Tasks 11–12
    ActionFailed { key: String, message: String },
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
}
