# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [0.5.0] - 2026-10-10

### Breaking
- Describe and delete keys are split, k9s-style: `d` now **describes** the selected connection, table, column or row, and `D` (Shift) **deletes** a connection or table row (with confirmation). Previously `d` deleted.

### Added
- Describe view (`d`) with richer metadata from the database: tables show row count, column count, primary key, index names and size (size on PostgreSQL only); columns show whether they are part of a key.
- k9s-style `:` command bar: `:conn`, `:schemas`, `:tables`, `:columns`, `:sql`, `:log`, `:activity`, `:help`, `:q`, `:123` (jump to row), and `:table <name>` to open a table by name. Unique prefixes work and Tab completes verbs and table names.
- Watch mode (`w`) re-runs the current SQL results query every 2s until toggled off.
- PostgreSQL activity view (`A`) showing `pg_stat_activity` (pid, query, state, wait event, start time).
- Query log (`L`) listing every query d7s ran this session, with origin, duration, rows or error, and the real SQL sent.
- Connection health check (`p` in the connection list) pings every saved connection concurrently and shows a Status column.

### Fixed
- Statements that return no rows are executed exactly once instead of twice.
- Wide hotkey hints in the top bar are no longer clipped, and table row selection is centred instead of hugging the bottom of the viewport.

### Changed
- Repository is now a Cargo workspace; the shared k9s-style widget kit is the `k9tui` crate, which `d7s` depends on. A second app, `c8s` (Docker), lives alongside it.
- Declared minimum supported Rust version 1.88.
- Dependency updates; `tokio-postgres`, `slab`, `tracing-subscriber` and `rand` were bumped to clear RustSec advisories.

## [0.4.0] - 2026-09-09

### Added
- k9s-style help view in main panel.
- Bracketed paste handling and row TSV copy.
- Persist MRU tables, SQL history, and page size.
- Live filter, jump-to-row, and quick reconnect.
- `--connection` and `--version` CLI flags.

### Changed
- k9s-style UI theme pass.
- k9s-style stacked connection info in top bar.
- Demo improvements and clippy lint fixes.

## [0.3.0] - 2026-06-06

### Added
- Table CRUD operations (insert, update, delete rows).
- Edit table cell values in place.
- Recently opened tables section in the top bar.
- Virtual table viewer for large row values.
- Better parsing for user-defined PostgreSQL enum types.

### Changed
- Improved table navigation to show more columns at once.
- SQL executor and editor improvements.
- Simplified internal rendering and data-loading code.

## [0.2.0] - 2026-03-29

### Added
- Copy table cell values to clipboard.
- Clear the status line on key press.

### Changed
- Consolidated from a multi-crate workspace (`d7s_auth`, `d7s_db`, `d7s_ui`) into a single crate for simpler publishing. The supporting crates published to crates.io under v0.1.0 (`d7s_auth`, `d7s_db`, `d7s_ui`) have been removed and will be yanked from crates.io.
- Simplified widget render methods.
- Moved connections and SQL executor into `DatabaseExplorer`.

### Fixed
- Fixed top bar height rendering.
- Fixed no connections displayed when exiting a database.
- Fixed column selection blur on first column when returning to row selection.

## [0.1.0] - 2026-02-03

### Added

- TUI database client for PostgreSQL and SQLite.
- Save and manage named database connections.
- Keyring support for the major platforms mac, linux and windows (not fully tested). 
- Browse databases, schemas, tables, columns, and table data with keyboard navigation.
- Execute SQL queries and view the results.
- Connection environment management (dev, staging, prod).
- Nix flake support.

