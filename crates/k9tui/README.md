# k9tui

Part of [t7s](https://github.com/robertazzopardi/t7s). Shared UI crate for [k9s](https://k9scli.io/)-style [ratatui](https://ratatui.rs) apps (theme, tables, modals, top bar, hotkeys). Used by [d7s](https://crates.io/crates/d7s) and [c8s](https://crates.io/crates/c8s). Pre-1.0; API may change.

```toml
[dependencies]
k9tui = "0.1"
```

| Module | Role |
|--------|------|
| `theme` | Colors / styles |
| `widgets::table` | `TableData`, `DataTable`, view state |
| `widgets::navigation` | vim/arrow row+column nav |
| `widgets::top_bar` | Title, status, hotkey stack |
| `widgets::hotkey` / `hotkey_view` | Hotkey list |
| `widgets::status_line` | Bottom status |
| `widgets::modal` | Confirm / prompt / fields |
| `widgets::help_view` | Help rows |
| `widgets::buttons` | Modal button row |
| `widgets::text_input` | Single-line input |

Domain logic stays in the app; k9tui has no dependency back on d7s/c8s.
