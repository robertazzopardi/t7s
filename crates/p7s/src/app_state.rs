use crate::proc::SystemInfo;

/// Top-level application state.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum AppState {
    /// The process list (main view).
    #[default]
    List,
    /// System summary panel (`i`).
    Info(SystemInfo),
    /// Full describe text for one process (`d` or Enter).
    Describe {
        pid: i32,
        name: String,
        text: String,
    },
    /// Signal picker for one process (`K`): choose a signal, then confirm.
    Signal {
        pid: i32,
        name: String,
        start_time: u64,
        index: usize,
    },
}
