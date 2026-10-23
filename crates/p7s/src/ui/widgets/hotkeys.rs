use crossterm::event::KeyCode;
use k9tui::widgets::hotkey::Hotkey;

/// Hotkeys shown on the process list.
pub fn list_hotkeys() -> Vec<Hotkey> {
    vec![
        Hotkey::new('c', "sort cpu"),
        Hotkey::new('m', "sort mem"),
        Hotkey::new('p', "sort pid"),
        Hotkey::new('n', "sort name"),
        Hotkey::new('d', "describe"),
        Hotkey::new('i', "system info"),
        Hotkey::new('K', "signal"),
        Hotkey::new(':', "command"),
    ]
}

pub fn global_hotkeys() -> Vec<Hotkey> {
    vec![Hotkey::new('/', "search"), Hotkey::new('q', "quit")]
}

/// Hotkeys shown while the signal picker is open.
pub fn signal_hotkeys() -> Vec<Hotkey> {
    vec![
        Hotkey::new('j', "down"),
        Hotkey::new('k', "up"),
        Hotkey::new('g', "top"),
        Hotkey::new('G', "bottom"),
        Hotkey::code(KeyCode::Enter, "confirm"),
        Hotkey::code(KeyCode::Esc, "back"),
        Hotkey::new('q', "back"),
    ]
}

/// Hotkeys shown in the describe view.
pub fn describe_hotkeys() -> Vec<Hotkey> {
    vec![
        Hotkey::new('j', "down"),
        Hotkey::new('k', "up"),
        Hotkey::code(KeyCode::PageUp, "page up"),
        Hotkey::code(KeyCode::PageDown, "page down"),
        Hotkey::new('g', "top"),
        Hotkey::new('G', "bottom"),
        Hotkey::code(KeyCode::Esc, "back"),
        Hotkey::new('q', "back"),
    ]
}
