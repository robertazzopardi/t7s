# t7s

A monorepo of k9s-style terminal UIs, built in Rust with [Ratatui](https://ratatui.rs).

## Why

After discovering [k9s](https://k9scli.io/), I liked its format enough to want it for more than Kubernetes — a keyboard-driven, table-first TUI shell reused across a few small tools.

## Apps

### [d7s](crates/d7s)

A TUI database client for PostgreSQL and SQLite: connection management, keyring-backed credentials, schema/table/row navigation, and a SQL executor. See its [README](crates/d7s/README.md).

![d7s — connect, browse tables, filter, help, SQL](crates/d7s/demo.gif)

### [c8s](crates/c8s)

A TUI for Docker: containers (with CPU/memory), images, volumes and networks, start/stop/restart/remove, live log tail, and exec shell. See its [README](crates/c8s/README.md).

![c8s — container list, restart, live logs](crates/c8s/demo.gif)

## Shared widget kit

- **[`crates/k9tui`](crates/k9tui)** — the k9s-style ratatui chrome (theme, tables, modals, top bar, hotkeys) both apps build on, with no dependency back on either. See its [README](crates/k9tui/README.md).

## Installing

```sh
cargo install d7s --locked
cargo install c8s --locked
```

See each app's README for details.

## Contributing

PRs welcome. `just check` runs fmt, clippy (pedantic/nursery), and tests before you push — see `just --list` for other recipes.
