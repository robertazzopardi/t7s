# a4s

Part of [t7s](https://github.com/robertazzopardi/t7s). k9s-style TUI for running CLI coding agents side by side on [Ratatui](https://ratatui.rs) / [k9tui](https://crates.io/crates/k9tui).

## Features

Early scaffold: an empty sessions table. Agent sessions in PTYs come in later releases.

## Usage

```sh
cargo install a4s --locked && a4s
# or: cargo run -p a4s
```

## Hotkeys

| Key | Action |
| --- | --- |
| `Enter` | Open the selected session |
| `c` | New session |
| `K` | Kill the selected session |
| `F` | Forget the selected session |
| `r` | Restart the selected session |
| `q` / `Ctrl-C` | Quit |
