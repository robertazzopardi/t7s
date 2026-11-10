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
            let edit = keymap.label("drill");
            format!("{new} new row · {edit} edit cell · {help} help")
        }
        (AppState::DatabaseConnected, _) => {
            let back = keymap.label("back");
            format!("{help} help · {back} back · {quit} quit")
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
        assert!(keymap.set("drill", KeyCode::F(2)));
        assert!(keymap.set("back", KeyCode::F(3)));
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
            "x new row · F2 edit cell · F1 help"
        );
    }
}
