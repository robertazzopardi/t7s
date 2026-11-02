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
| `:table conn.db.schema.table` | Jump across connections, databases and schemas (each part matched like above; an ambiguous connection is refused). MySQL: `conn.db.table`. SQLite: `conn.table`. Tab completes each level of the current context |
| `:table schema.table` / `:table db.schema.table` | Inside Postgres: relative to the current database / connection (MySQL: `:table db.table`). A path starting with an exact connection name that fits stays absolute |
| `:columns` / `:cols` | Columns |
| `:sql` / `:editor` | SQL editor |
| `:log` / `:querylog` | Query log |
| `:activity` | Activity (Postgres) |
| `:help` | Help |
| `:q` / `:quit` | Quit |
| `:123` | Jump to row |

### Notes vs k9s

`d` describes; `D` deletes (confirm). `y` copies a cell (not YAML). `/` filters the current view.

## Configuration

`d7s` reads `~/.config/t7s/config.toml`, or `$XDG_CONFIG_HOME/t7s/config.toml` if
that is set. The file and everything in it are optional. Settings merge in this
order, with the most specific winning: built-in defaults, `[t7s]`, then `[d7s]`.

```toml
[t7s.keys]
quit = "q"

[d7s.keys]
new-connection = "m"
```

Keys are one character (case-sensitive) or a name such as `esc`, `page-up` or
`f5`. Ctrl and Alt chords aren't supported.

| Table | Actions (default key) |
|---|---|
| `[t7s.keys]` or `[d7s.keys]` | `quit` (`q`), `search` (`/`), `command` (`:`) |
| `[d7s.keys]`, connection list | `new-connection` (`n`), `edit-connection` (`e`), `delete-connection` (`D`), `open-connection` (`o`), `reconnect` (`O`), `ping` (`p`) |
| `[d7s.keys]`, connected | `sql-editor` (`e`), `table-structure` (`t`), `run-sql` (`E`), `watch` (`w`), `query-log` (`L`), `activity` (`A`), `export` (`x`) |
| `[d7s.keys]`, table data | `refresh` (`r`), `new-row` (`n`), `defaults` (`N`), `edit` (`i`), `duplicate` (`c`), `commit` (`s`), `delete-row` (`D`) |
| `[d7s.keys]`, everywhere | `describe` (`d`), `copy` (`y`), `copy-row` (`Y`), `width` (`W`), `help` (`?`) |

These stay fixed: `h j k l`, `b w`, `g G`, `0 $`, arrow keys, `1`-`5`, `#`, space,
`Esc`, `Enter`, `Backspace`, `Delete`, `R` (refresh), `a` (new row), `Ctrl+s`
(export), `Ctrl+c` (quit), and keys in modals, text inputs and the describe view.

`d7s` won't start, and prints every error to stderr, if the file has a TOML
syntax error or can't be read, a key name is unknown, two actions share a key on
one screen, or an action is bound to a fixed key. Other problems, such as an
unknown setting, show in the status line and are printed on exit.

`skin`, `theme` and `colours` are accepted but don't change anything yet.
