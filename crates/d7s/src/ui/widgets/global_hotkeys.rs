use crossterm::event::KeyCode;
use k9tui::widgets::hotkey::Hotkey;

/// Always-visible global shortcuts shown in the top bar (d7s-specific keymap).
#[must_use]
pub fn global_hotkeys(on_connection_list: bool) -> Vec<Hotkey> {
    let mut keys = vec![Hotkey::new('?', "Help").action("help")];
    if on_connection_list {
        keys.push(Hotkey::new('q', "Quit").action("quit"));
    } else {
        keys.push(Hotkey::code(KeyCode::Esc, "Back"));
    }
    keys
}
