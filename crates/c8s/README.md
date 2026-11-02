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
that is set. The file and everything in it are optional. Settings merge in this
order, with the most specific winning: built-in defaults, `[t7s]`, then `[c8s]`.

```toml
[t7s.keys]
quit = "q"

[c8s.keys]
start-stop = "x"
```

Keys are one character (case-sensitive) or a name such as `esc`, `page-up` or
`f5`. Ctrl and Alt chords aren't supported.

| Table | Actions (default key) |
|---|---|
| `[t7s.keys]` or `[c8s.keys]` | `quit` (`q`), `search` (`/`), `command` (`:`) |
| `[c8s.keys]` | `running` (`a`), `processes` (`p`), `start-stop` (`s`), `restart` (`r`), `logs` (`l`), `exec` (`e`), `describe` (`d`), `remove` (`D`), `health` (`i`), `width` (`W`) |

These stay fixed: `j k g G`, arrow keys, `1`-`4`, `Esc`, `Enter` (describe), `Delete`
(remove), `S` (start/stop), `Ctrl-C` (quit), and keys in the log, process and
describe views.

`c8s` won't start, and prints every error to stderr, if the file has a TOML
syntax error or can't be read, a key name is unknown, two actions share a key, or
an action is bound to a fixed key. Other problems, such as an unknown setting,
show in the status line and are printed on exit.

`skin`, `theme` and `colours` are accepted but don't change anything yet.
