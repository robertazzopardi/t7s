/// Successful SQL execute payload for the UI thread.
#[derive(Debug, Clone)]
pub struct SqlExecuteOk {
    pub cols: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Update pushed from a background task into the app's event loop.
pub enum BgEvent {
    /// Background action finished successfully.
    #[allow(dead_code)] // Tasks 10–12
    ActionDone { key: String, message: String },
    /// Background action failed.
    #[allow(dead_code)] // Tasks 10–12
    ActionFailed { key: String, message: String },
    /// SQL execute finished (success or error).
    SqlFinished {
        key: String,
        sql: String,
        record_history: bool,
        result: Result<SqlExecuteOk, String>,
    },
}
