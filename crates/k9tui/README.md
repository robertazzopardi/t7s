# k9tui

Part of [t7s](https://github.com/robertazzopardi/t7s#naming) (terminals). Shared UI crate for [k9s](https://k9scli.io/)-style ratatui apps — theme, tables, modals, top bar and hotkeys for building keyboard-driven, row-based TUIs. Extracted from [d7s](https://github.com/robertazzopardi/t7s), which remains its reference consumer.

## What's in it

- `theme` — shared color/style helpers (focus, borders, success/danger states).
- `widgets::table` — `TableData` trait, `TableModel`/`TableViewState`/`TableDataState`, `DataTable` widget for dynamic-column row tables.
- `widgets::navigation` — `TableNavigationHandler`: vim-style (`hjkl`)/arrow row and column navigation with wrapping.
- `widgets::top_bar` — `TopBarView`: title, connection/status summary, hotkey stack.
- `widgets::hotkey` / `widgets::hotkey_view` — `Hotkey` type and the hotkey-list widget shown in the top bar and help screen.
- `widgets::status_line` — bottom status/idle-hint bar.
- `widgets::modal` — `ConfirmDialog`, `TextPromptModal`, `ModalField`: generic confirm/prompt dialogs and form-field building block.
- `widgets::help_view` — `HelpRow` table-row builder for a help screen.
- `widgets::buttons` — modal button row widget.
- `widgets::text_input` — masked/plain single-line text input.

## Using it

```toml
[dependencies]
k9tui = "0.1"
```

App-specific behavior (domain types, orchestration, key-handling policy) stays in the consuming crate; k9tui only holds shared UI widgets and interaction patterns with no dependency back on any app.

## Status

Pre-1.0 and published on crates.io. Two apps use it, [d7s](https://crates.io/crates/d7s) and [c8s](https://crates.io/crates/c8s), both in the same repository; the API may still change between 0.x releases.
