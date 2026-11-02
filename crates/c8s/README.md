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

### Skin (colors)

Drop a `~/.config/c8s/skin.yml` to override theme colors. Any color omitted
keeps its built-in default; if the file is absent, c8s uses the built-in
theme unchanged. Colors are ANSI names (`white`, `darkgray`, `cyan`, ...) or
`#rrggbb` hex.

```yaml
colors:
  text: white
  focus: "#ffcc00"
  error: red
  success: green
```

Available color names: `muted`, `text`, `focus`, `on_focus`, `selection_fg`,
`selection_bg`, `bg_alt`, `draft`, `multi_select`, `info`, `error`,
`success`, `warning`, `link`.

### Key bindings

Drop a `~/.config/c8s/keys.yml` to remap the action hotkeys (not navigation,
quit, or the `Delete` fallback). It's a flat map of action name to a single
character or a named key (`esc`, `f1`–`f12`, ...):

```yaml
start_stop: s
remove: x
```

Remappable actions (default key): `start_stop` (`s`), `restart` (`r`), `logs`
(`l`), `exec` (`e`), `describe` (`d`), `remove` (`D`). `Enter` (describe),
`Delete` (remove) and `S` (start/stop) stay as fixed aliases. Keys in the
describe/log views, `/`, `:`, `a`, `p`, `i`, `W`, `1`-`4`, and quit are
not remappable. The hotkey bar
shows remapped keys.

Bad entries (unknown action, unknown key name, invalid YAML) are ignored with
a one-line warning in the status bar; defaults stay in force. Bind each key
to one action per screen, since conflicts are not detected.
