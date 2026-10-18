use crate::app_state::{AppState, DatabaseExplorerState};

#[must_use]
pub fn default_idle_hint(
    app_state: AppState,
    explorer_state: &DatabaseExplorerState,
) -> String {
    match (app_state, explorer_state) {
        (AppState::ConnectionList, _) => "? help · n new · q quit".to_string(),
        (
            AppState::DatabaseConnected,
            DatabaseExplorerState::TableData(_, _),
        ) => "n new row · Enter edit cell · ? help".to_string(),
        (AppState::DatabaseConnected, _) => {
            "? help · Esc back · q quit".to_string()
        }
    }
}
