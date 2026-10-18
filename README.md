# t7s

k9s-style terminal UIs in Rust ([Ratatui](https://ratatui.rs)). **t7s** is short for “terminals”: **d7s** (databases), **c8s** (containers), and **k9tui** (shared UI crate).

Inspired by [k9s](https://k9scli.io/) — keyboard-driven, table-first TUIs for more than Kubernetes.

## Apps

### [d7s](crates/d7s)

PostgreSQL, MySQL/MariaDB, and SQLite client: connections, keyring credentials, schema/table/row navigation, SQL editor. [README](crates/d7s/README.md).

![d7s — connect, browse tables, filter, help, SQL](crates/d7s/demo.gif)

### [c8s](crates/c8s)

Docker TUI: containers (CPU/memory), images, volumes, networks, logs, exec. [README](crates/c8s/README.md).

![c8s — container list, restart, live logs](crates/c8s/demo.gif)

### [k9tui](crates/k9tui)

Shared UI crate (theme, tables, modals, top bar, hotkeys). [README](crates/k9tui/README.md).

## Install

```sh
cargo install d7s --locked
cargo install c8s --locked
```

Prebuilt binaries are on [GitHub Releases](https://github.com/robertazzopardi/t7s/releases), or `cargo binstall d7s` / `cargo binstall c8s`.

## Contributing

PRs welcome. `just check` runs fmt, clippy, and tests (`just --list` for more).

## Releasing

1. Put notes under `## [x.y.z]` in `crates/<crate>/CHANGELOG.md` (not only `[Unreleased]`).
2. `just release <k9tui|d7s|c8s> <version>` — publishes to crates.io, then tags (`vX.Y.Z` / `c8s-vX.Y.Z` / `k9tui-vX.Y.Z`). Binstall picks up archive URLs from the published crate metadata.
3. Tag runs `.github/workflows/release.yml` (per-target archives + `SHA256SUMS`; only d7s is GitHub Latest). k9tui has no binary release.
4. Rebuild an existing tag: `gh workflow run release.yml --ref main -f tag=v0.5.0`. Delete stale bare assets if present: `gh release delete-asset <tag> d7s -y` (and `d7s.exe`).
