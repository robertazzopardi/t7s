## Stale explorer load failures (review fix)

- `TablePageLoaded` errors gated with `should_apply_table_page` (schema/table/record_recent on event).
- `ExplorerLoadFailed` gated via `should_apply_explorer_load_failed` (resource + schema/table); `in_flight.end` unchanged.
- Tests: `stale_table_page_load_error_is_discarded`, busy status on `load_table_data_refuses_when_table_key_busy`.

### Verification

- `cargo test -p d7s stale_table` — 2 passed
- `cargo test -p d7s load_table_data_refuses` — 1 passed
- `cargo test -p d7s` — 120 passed, 11 ignored
- `cargo clippy -p d7s -- -D warnings` — clean
