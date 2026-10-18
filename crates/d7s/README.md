# d7s

Part of [t7s](https://github.com/robertazzopardi/t7s). TUI database client for PostgreSQL, MySQL/MariaDB, and SQLite — [Ratatui](https://ratatui.rs), [k9s](https://k9scli.io/)-style.

## Features

- **Postgres, MySQL/MariaDB, SQLite** — TLS (rustls) for Postgres/MySQL; tables without a primary key are read-only on MySQL
- **Connections** — save/edit/delete; passwords in the platform keyring (or prompt every time)
- **Browse** — databases → schemas → tables → columns → rows (vim keys)
- **SQL editor** — multi-statement picker; mutating statements need confirm
- **Watch** — re-run SQL results every 2s (`w`)
- **Activity** — Postgres `pg_stat_activity` (`A`)
- **Query log** — session history (`L`), last 500
- **Health** — ping saved connections from the list (`p`)

## Demo

![d7s](https://raw.githubusercontent.com/robertazzopardi/t7s/main/crates/d7s/demo.gif)

```sh
just demo   # needs vhs + sqlite3; isolated HOME, no real connections
```

## Install

Rust 1.89+.

```sh
cargo install d7s --locked
# or from a checkout:
cargo build --release --locked   # → target/release/d7s
nix develop                      # then just --list
```

```sh
d7s
# -c/--connection NAME, -h/--help, -V/--version
```

## Hotkeys

| Key | Action |
|-----|--------|
| `q` / `Ctrl-c` | Quit |
| `?` | Help |
| `y` / `Y` | Copy cell / row (TSV) |
| `j`/`k` `h`/`l` · `g`/`G` · `0`/`$` | Move · top/bottom · first/last column |

**Connections:** `n` new · `e` edit · `D` delete · `o`/Enter open · `O` reconnect · `p` ping · `d` describe

**Connected:** `e` SQL editor · `E` run SQL · `t` table structure · `d` describe · `/` search · `1`–`5` recent table · `L` query log · `A` activity (Postgres) · `:` command bar

**Table data:** `r` refresh · `a` new row · `c` duplicate draft · `s` commit · `D` delete · Space multi-select · Enter edit cell · `#` / `:123` jump row

**SQL results:** `Ctrl-s`/`x` export TSV · `w` watch

Paste works in modals and the search bar.

### `:` commands

Tab completes; Esc cancels. Unique prefixes work (`:sch`). Draft row pending blocks navigation.

| Command | Action |
|---------|--------|
| `:conn` / `:connections` | Connection list |
| `:schemas` / `:schema` | Schema list |
| `:schema <name>` | Open schema (exact → prefix → substring). SQLite: no schemas |
| `:tables` / `:table` | Tables in current schema |
| `:table <name>` | Open table (same match rules; spaces ok) |
| `:columns` / `:cols` | Columns |
| `:sql` / `:editor` | SQL editor |
| `:log` / `:querylog` | Query log |
| `:activity` | Activity (Postgres) |
| `:help` | Help |
| `:q` / `:quit` | Quit |
| `:123` | Jump to row |

### Notes vs k9s

`d` describes; `D` deletes (confirm). `y` copies a cell (not YAML). `/` filters the current view.
