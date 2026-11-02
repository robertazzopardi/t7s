use k9tui::keymap::Keymap;

use crate::app_state::{AppState, DatabaseExplorerState};

#[must_use]
pub fn default_idle_hint(
    app_state: AppState,
    explorer_state: &DatabaseExplorerState,
    keymap: &Keymap,
) -> String {
    let help = keymap.label("help");
    let quit = keymap.label("quit");
    match (app_state, explorer_state) {
        (AppState::ConnectionList, _) => {
            let new = keymap.label("new-connection");
            format!("{help} help · {new} new · {quit} quit")
        }
        (
            AppState::DatabaseConnected,
            DatabaseExplorerState::TableData(_, _),
        ) => {
            let new = keymap.label("new-row");
            format!("{new} new row · Enter edit cell · {help} help")
        }
        (AppState::DatabaseConnected, _) => {
            format!("{help} help · Esc back · {quit} quit")
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use k9tui::config::defaults;

    use super::*;
    use crate::keymap::SPEC;

    #[test]
    fn default_hints_are_unchanged() {
        let keymap = defaults(&SPEC).keymap;
        assert_eq!(
            default_idle_hint(
                AppState::ConnectionList,
                &DatabaseExplorerState::Connections,
                &keymap
            ),
            "? help · n new · q quit"
        );
    }

    #[test]
    fn hints_show_remapped_keys() {
        let mut keymap = defaults(&SPEC).keymap;
        assert!(keymap.set("help", KeyCode::F(1)));
        assert!(keymap.set("new-connection", KeyCode::Char('m')));
        assert!(keymap.set("quit", KeyCode::Char('z')));
        assert!(keymap.set("new-row", KeyCode::Char('x')));
        assert_eq!(
            default_idle_hint(
                AppState::ConnectionList,
                &DatabaseExplorerState::Connections,
                &keymap
            ),
            "F1 help · m new · z quit"
        );
        assert_eq!(
            default_idle_hint(
                AppState::DatabaseConnected,
                &DatabaseExplorerState::TableData("s".into(), "t".into()),
                &keymap
            ),
            "x new row · Enter edit cell · F1 help"
        );
    }
}
