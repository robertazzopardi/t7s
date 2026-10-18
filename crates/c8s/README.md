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

`d` describes; `D` deletes. No `y` (no manifests). `/` filters the active list.
