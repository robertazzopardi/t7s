# t7s

A monorepo of k9s-style terminal UIs, built in Rust with [Ratatui](https://ratatui.rs).

## Naming

**t7s** is short for “terminals”. Apps in the family follow it: **d7s** (databases), **c8s** (containers), plus **k9tui**, the shared UI crate.

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

- **[`crates/k9tui`](crates/k9tui)** — shared UI crate (theme, tables, modals, top bar, hotkeys) both apps build on, with no dependency back on either. See its [README](crates/k9tui/README.md).

## Installing

```sh
cargo install d7s --locked
cargo install c8s --locked
```

Prebuilt binaries (Linux x86_64/aarch64, macOS x86_64/arm64, Windows x86_64) are attached to each [GitHub release](https://github.com/robertazzopardi/t7s/releases), or use `cargo binstall d7s` / `cargo binstall c8s`.

See each app's README for details.

## Contributing

PRs welcome. `just check` runs fmt, clippy (pedantic/nursery), and tests before you push — see `just --list` for other recipes.

## Releasing

1. Bump the crate version and move notes into a `## [x.y.z]` section in `crates/<crate>/CHANGELOG.md` (do not leave the release notes only under `[Unreleased]`). Use these headings in order (omit empty ones): optional highlights / `### New crates`, `### Breaking`, `### Added`, `### Changed`, `### Fixed`.
2. Merge to `main`, then `just release <k9tui|d7s|c8s> <version>` (publishes to crates.io, then tags `vX.Y.Z` for d7s or `c8s-vX.Y.Z` for c8s). Publishing is what makes `cargo binstall` see new archive URLs; repairing GitHub assets alone does not update crates.io metadata for an already-published version.
3. The tag triggers `.github/workflows/release.yml`: it builds archives (`<crate>-v<version>-<target>.tar.gz`/`.zip`) plus `SHA256SUMS`, and publishes a release whose body is that version’s CHANGELOG section plus a generated Install table. Only d7s is marked GitHub “Latest”. k9tui tags build nothing.
4. To rebuild assets for an existing tag: `gh workflow run release.yml --ref main -f tag=v0.5.0`. Same-named archives are replaced; delete leftover pre-archive assets from older releases (`d7s`, `d7s.exe`) with `gh release delete-asset <tag> <name> -y`.
