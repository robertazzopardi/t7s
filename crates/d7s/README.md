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

### Skin (colors)

Drop a `~/.config/d7s/skin.yml` to override theme colors. Any color omitted
keeps its built-in default; if the file is absent, d7s uses the built-in
theme unchanged. Colors are ANSI names (`white`, `darkgray`, `cyan`, ...) or
`#rrggbb` hex.

```yaml
colors:
  text: white
  focus: "#ffcc00"
  error: red
  success: green
  info: cyan
```

Available color names: `muted`, `text`, `focus`, `on_focus`, `selection_fg`,
`selection_bg`, `bg_alt`, `draft`, `multi_select`, `info`, `error`,
`success`, `warning`, `link`.

### Key bindings

Drop a `~/.config/d7s/keys.yml` to remap the action hotkeys shown in the
hotkey bar (not vim-style navigation, quit, help, or Esc). It's a flat map
of action name to a single character or a named key (`esc`, `enter`, `tab`,
`f1`–`f12`, `pageup`, ...):

```yaml
new_connection: n
delete_connection: x
refresh: f5
```

Remappable actions (default key): `new_connection` (`n`), `edit_connection`
(`e`), `delete_connection` (`D`), `describe` (`d`), `open_connection` (`o`),
`sql_editor` (`e`), `table_structure` (`t`), `run_sql` (`E`), `copy_value`
(`y`), `refresh` (`r`), `new_row` (`n`), `duplicate_row` (`c`), `commit_row`
(`s`), `delete_row` (`D`). `R` (refresh) and `a` (new row) stay as fixed aliases.
`/`, `:`, `w`, `W`, `p`, `N`, `i`, `L`, `A`, `O`, `x`, quit, help and vim navigation are not
remappable. The hotkey bar shows remapped keys; the `?` help panel still lists
the default keys.

Bad entries (unknown action, unknown key name, invalid YAML) are ignored with
a one-line warning in the status bar; defaults stay in force. Bind each key
to one action per screen, since conflicts are not detected.
