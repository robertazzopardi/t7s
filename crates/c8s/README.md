# c8s

Part of [t7s](https://github.com/robertazzopardi/t7s). k9s-style Docker TUI on [Ratatui](https://ratatui.rs) / [k9tui](https://crates.io/crates/k9tui).

![c8s](https://raw.githubusercontent.com/robertazzopardi/t7s/main/crates/c8s/demo.gif)

```sh
just c8s-demo   # vhs + Docker; fake demo-web/demo-cache containers
```

## Features

Containers (CPU/memory, start/stop/restart/remove, running-only filter), images, volumes, networks; `/` search; log tail with dedup; process list; describe; `docker exec` shell; daemon health (`i`). Needs a reachable Docker daemon and `docker` on `PATH` for exec.

v1 is flat list views — no compose graphs.

## Usage

```sh
cargo install c8s --locked && c8s
# or: cargo run -p c8s
```

## Hotkeys

| Key | Action |
| --- | --- |
| `1`/`2`/`3`/`4` | Containers / images / volumes / networks |
| `j`/`k` · `g`/`G` | Move · top/bottom |
| `:` | `:containers` `:images` `:volumes` `:networks` `:q` (Tab completes) |
| `/` | Filter current list |
| `s` / `r` / `a` | Start/stop · restart · running-only (containers) |
| `p` / `l` / `e` | Processes · logs · exec shell |
| `D` / `Delete` | Remove (confirm, default No) |
| `d` / Enter | Describe |
| `i` | Daemon health |
| `q` / `Ctrl-C` | Quit |

`d` describes; `D` deletes. No `y` (no manifests). `/` filters the active list and searches logs and describe.

## Configuration

`c8s` reads `~/.config/t7s/config.toml`, or `$XDG_CONFIG_HOME/t7s/config.toml` if
that is set. The file and everything in it are optional. The defaults live in
TOML files in this repo: `crates/k9tui/src/config.toml` for shared settings and
`crates/c8s/src/config.toml` for this app. Settings merge in this order, with
the most specific winning: `k9tui`, the app's file, your `[t7s]`, your `[c8s]`.

Every hotkey can be remapped, including navigation:

```toml
[t7s.keys]
quit = "q"
down = ["j", "n", "down"]

[c8s.keys]
start-stop = "x"
```

A key is one character (case-sensitive) or a name such as `esc`, `page-up`,
`back-tab`, `space` or `f5`. An action can have several keys. The first is the
one the hotkey bar shows. A list replaces the list below it, so `down = "z"`
frees `j` and the Down arrow. Ctrl and Alt chords can't be bound.

Shared actions go in `[t7s.keys]` or `[c8s.keys]`. See
`crates/k9tui/src/config.toml` for them and `crates/c8s/src/config.toml` for
every action this app has.

The hotkey bars use the same files, and you can override them:

```toml
[c8s.screens.list]
hotkeys = [
  { action = "logs", label = "my label" },
  { key = "Ctrl+c", label = "quit" },
]
```

A row with `action` shows that action's current key. A row with `key` is display
text. Editing a bar never changes which keys work.

`c8s` won't start, and prints the error to stderr, if the file has a TOML
syntax error or can't be read, a setting, action, screen, key name or colour is
unknown or malformed, an action has no keys, or two actions share a key on one
screen. Every table in the file must be valid, including another app's. An
action's later keys may share another action's first key, and the first key
wins on that screen.

These can't be remapped: `Ctrl-C` always quits, `Ctrl-U` and `Ctrl-D` page in logs, and typing inside
text inputs, modals and the search and command bars.

`skin`, `theme` and `colours` are accepted but don't change anything yet.
