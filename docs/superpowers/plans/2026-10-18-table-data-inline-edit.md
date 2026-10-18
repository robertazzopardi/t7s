# Table Data Inline Edit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Spreadsheet-style inline cell editing for table data, with hybrid commit (saved cells auto-commit on leave; drafts need `s`), create keys `n`/`N`/`c`, and discoverability chrome — per `docs/superpowers/specs/2026-10-18-table-data-inline-edit-design.md`.

**Architecture:** Add a small `CellEditSession` on `App` (buffer + pre-edit snapshot + row/col). While editing, write the buffer into the grid cell for display (no k9tui API change). Route Cell-edit keys inside `handle_table_data_hotkeys` before Browse bindings / modal Enter. Reuse `apply_cell_value_edit` / `CellValueApply` for saved-cell DB writes and draft in-memory stash. Keep `CellValueModal` as Ctrl+e / `e` escape hatch.

**Tech Stack:** Rust, crossterm keys, existing draft machinery in `table_data_actions.rs`, `Database::update_table_cell` / `get_columns`, ratatui `Table` via k9tui.

## Global Constraints

- Prefer jj: `jj describe -m "…" && jj new` (force-track under `docs/superpowers/` with `jj file track --include-ignored` if needed).
- Do not abandon ratatui `Block`/`Table` borders or strip the logo.
- Browse bindings always win over type-to-overwrite.
- In Browse, `e` remains SQL editor; modal only via Ctrl+e/`e` **while in Cell edit**.
- Filter must stay clear for add/dup/commit/delete (existing guards).
- One focused test per non-trivial rule; reuse `tables_app` / `on_key_event` patterns in `app.rs` tests.
- YAGNI: no vim dual-mode, no separate compose UI, no draft persistence across sessions.

## File map

| File | Role |
|------|------|
| `crates/d7s/src/cell_edit.rs` | `CellEditSession` + pure helpers (next/prev editable col, browse-binding set) |
| `crates/d7s/src/app.rs` | Own `cell_edit: Option<CellEditSession>`; status tokens; integration tests |
| `crates/d7s/src/table_data_actions.rs` | Create keys, defaults draft, Cell-edit key routing |
| `crates/d7s/src/database_explorer.rs` | Build `CellValueApply` from session; Enter no longer opens modal by default |
| `crates/d7s/src/event_handlers.rs` | Esc while editing reverts cell (before draft-discard); Ctrl+e when editing |
| `crates/d7s/src/ui/widgets/hotkeys.rs` | Context hotkey lists |
| `crates/d7s/src/ui/widgets/help_content.rs` | Table-data help rows |
| `crates/d7s/src/ui/widgets/idle_hint.rs` | Table-data idle hint when connected + on TableData |
| `crates/d7s/src/rendering.rs` | Swap hotkey bar when `cell_edit` active / draft present |
| `crates/d7s/src/main.rs` or `lib` mod | `mod cell_edit;` |

---

### Task 1: `CellEditSession` + column helpers

**Files:**
- Create: `crates/d7s/src/cell_edit.rs`
- Modify: `crates/d7s/src/lib.rs` or `main.rs` (wherever modules are declared — today logic lives under `main.rs` / `app` modules; add `mod cell_edit;` next to `mod table_data_actions;`)
- Test: unit tests inside `cell_edit.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Clone)]
  pub struct CellEditSession {
      pub row_index: usize,
      pub col_index: usize,
      pub buffer: String,
      pub pre_edit_value: String,
      pub row_snapshot: Vec<String>, // full row values at edit start (for apply + original sync)
  }

  /// Table-data Browse bindings that must not type-to-overwrite.
  pub fn is_table_data_browse_binding(c: char) -> bool;

  pub fn next_editable_column(cols: &[EditableCol], from: usize) -> Option<usize>;
  pub fn prev_editable_column(cols: &[EditableCol], from: usize) -> Option<usize>;

  #[derive(Debug, Clone, Copy)]
  pub struct EditableCol {
      pub index: usize,
      pub editable: bool,
  }
  ```

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_bindings_include_create_and_commit_keys() {
        for c in ['n', 'N', 'c', 's', 'r', 'D', 'a', 'i', 'y', 'Y', 'd', '/', ':', '#', '?'] {
            assert!(is_table_data_browse_binding(c), "{c}");
        }
        assert!(!is_table_data_browse_binding('z'));
        assert!(!is_table_data_browse_binding('1'));
    }

    #[test]
    fn tab_skips_non_editable_and_wraps_within_row() {
        let cols = [
            EditableCol { index: 0, editable: false },
            EditableCol { index: 1, editable: true },
            EditableCol { index: 2, editable: true },
        ];
        assert_eq!(next_editable_column(&cols, 1), Some(2));
        assert_eq!(next_editable_column(&cols, 2), Some(1)); // wrap
        assert_eq!(prev_editable_column(&cols, 1), Some(2));
        assert_eq!(next_editable_column(&cols, 0), Some(1));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p d7s cell_edit -- --nocapture`  
Expected: compile fail (module missing) or test fail.

- [ ] **Step 3: Implement minimal module**

Implement `is_table_data_browse_binding` as a `matches!` on the char set above (include Space only as multi-select in Browse — Space is a binding in Browse, so return true for `' '`).

`next_editable_column` / `prev_editable_column`: walk editable indices only; wrap within that list; if none editable return `None`.

- [ ] **Step 4: Run tests — expect PASS**

Run: `cargo test -p d7s cell_edit -- --nocapture`

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(d7s): CellEditSession helpers for inline table edit"
jj new
```

---

### Task 2: Start / type / Esc-cancel inline edit (draft path first)

**Files:**
- Modify: `crates/d7s/src/app.rs` — add `pub(crate) cell_edit: Option<CellEditSession>`
- Modify: `crates/d7s/src/table_data_actions.rs` — `begin_cell_edit`, `cancel_cell_edit`, key routing
- Modify: `crates/d7s/src/database_explorer.rs` — TableData `Enter` should call `begin_cell_edit` instead of opening modal
- Modify: `crates/d7s/src/event_handlers.rs` — when `cell_edit.is_some()`, Esc cancels cell first (do not advance draft-discard)
- Test: `crates/d7s/src/app.rs` tests

**Interfaces:**
- Consumes: `CellEditSession`, `get_selected_cell_for_modal` fields (row/col/value/snap)
- Produces:
  - `App::begin_cell_edit(&mut self, replace_with: Option<String>)` — `None` = keep value, caret semantics = buffer = current; `Some("")` = clear; `Some(ch)` = replace with that string
  - `App::cancel_cell_edit(&mut self)` — restore `pre_edit_value` into grid + clear session
  - `App::is_cell_editing(&self) -> bool`

Display strategy: on each buffer change, write `buffer` into `items[row].values[col]` (and keep `original` untouched until commit). Cancel restores from `pre_edit_value`.

- [ ] **Step 1: Failing integration test**

```rust
#[tokio::test]
async fn enter_edits_draft_cell_inline_esc_reverts() {
    let mut app = tables_app("inline_esc").await;
    run_cmd(&mut app, "table users").await;
    app.on_key_event(key('a')).await.unwrap(); // blank draft (until Task 5 remaps)
    assert!(app.has_table_draft_rows());
    app.on_key_event(enter()).await.unwrap();
    assert!(app.is_cell_editing());
    app.on_key_event(key('x')).await.unwrap();
    let cell = /* read selected cell value via table_data */;
    assert_eq!(cell, "x");
    app.on_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await.unwrap();
    assert!(!app.is_cell_editing());
    let cell = /* read again */;
    assert_eq!(cell, ""); // reverted
    assert!(app.has_table_draft_rows()); // draft remains
}
```

Helper: read cell from `app.database_explorer.table_data` selected row/col.

- [ ] **Step 2: Run — expect FAIL** (`is_cell_editing` missing or Enter still opens modal)

- [ ] **Step 3: Implement**

1. Add `cell_edit: None` to `App::default` / `new`.
2. `begin_cell_edit`: refuse if no selection; build session; apply `replace_with` to buffer; write buffer into grid cell; `set_status(format!("EDIT col={name}"))`.
3. In `handle_database_navigation` TableData branch: call `begin_cell_edit(None)` instead of `open_cell_value_modal`.
4. In `handle_table_data_hotkeys` **first**: if `cell_edit` active, handle:
   - Char → push to buffer (all printables)
   - Backspace → pop
   - Esc → `cancel_cell_edit`; return true
   - leave Tab/Enter/Ctrl+e for later tasks (stub: Enter calls cancel for now **or** finish — prefer finish-as-stash in Task 3; for this task Esc + typing only, Enter can no-op finish stash)
5. Wire `i` in table-data Browse → `begin_cell_edit(None)`.
6. Esc in `handle_hotkeys`: if `is_cell_editing()`, `cancel_cell_edit` and return true before draft-discard logic.

- [ ] **Step 4: Run test — PASS**

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(d7s): inline cell edit start/type/Esc for table data"
jj new
```

---

### Task 3: Finish cell + hybrid commit

**Files:**
- Modify: `crates/d7s/src/table_data_actions.rs` / `database_explorer.rs`
- Reuse: `apply_cell_value_edit`, `CellValueApply` in `connection_modal.rs`
- Test: `app.rs`

**Interfaces:**
- Produces: `App::finish_cell_edit(&mut self) -> Result<()>` — builds `CellValueApply` from session + schema/table/pk (same as modal path), awaits `apply_cell_value_edit`, clears session on success; on saved-row failure keep session + status error and restore buffer into cell.
- Enter in Cell edit → `finish_cell_edit` then Browse.
- Status on success: draft → keep existing draft wording; saved → `UPDATED {col}` (or keep “Cell updated.” — prefer spec token).

- [ ] **Step 1: Failing test — draft Enter stashes without INSERT**

```rust
#[tokio::test]
async fn draft_enter_stashes_without_insert() {
    let mut app = tables_app("inline_stash").await;
    run_cmd(&mut app, "table users").await;
    app.on_key_event(key('a')).await.unwrap();
    app.on_key_event(enter()).await.unwrap();
    app.on_key_event(key('z')).await.unwrap();
    app.on_key_event(enter()).await.unwrap();
    assert!(!app.is_cell_editing());
    assert!(app.has_table_draft_rows());
    // cell shows z; no commit yet — row still draft
}
```

- [ ] **Step 2: Failing test — saved cell finish calls update** (sqlite table with a row)

Open table with existing data, Enter, change value, Enter; assert cell value persisted after reload or that draft flag is false and value matches. Use a dedicated temp table:

```sql
CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO t VALUES (1, 'old');
```

Edit `name` to `new`, finish, `reload_current_table_data`, assert `new`.

- [ ] **Step 3: Implement `finish_cell_edit`** mirroring modal apply construction (pk from `get_primary_key_columns`, skip pk for drafts). Clear `cell_edit` only after successful apply (draft always succeeds in-memory).

- [ ] **Step 4: Tests PASS**

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(d7s): hybrid commit on leaving inline cell edit"
jj new
```

---

### Task 4: Tab / Shift+Tab between editable columns

**Files:**
- Modify: `crates/d7s/src/table_data_actions.rs`
- Modify: `crates/d7s/src/cell_edit.rs` if needed for “build EditableCol from Column meta”
- Test: `app.rs`

**Interfaces:**
- On Tab: `finish_cell_edit` (stash/commit current) → if still ok, move `selected_column` to `next_editable_column` → `begin_cell_edit(None)` on that column (same row).
- Shift+Tab: same with `prev_editable_column`.
- Until Task 6, treat all columns as editable (`editable: true`).
- Wrap within row only (already in helpers).

- [ ] **Step 1: Failing test**

```rust
#[tokio::test]
async fn tab_moves_to_next_column_on_draft() {
    let mut app = tables_app("inline_tab").await;
    // table with >=2 columns
    run_cmd(&mut app, "table users").await;
    app.on_key_event(key('a')).await.unwrap();
    app.on_key_event(enter()).await.unwrap();
    let c0 = selected_col(&app);
    app.on_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)).await.unwrap();
    assert!(app.is_cell_editing());
    assert_ne!(selected_col(&app), c0);
}
```

- [ ] **Step 2: Run — FAIL**

- [ ] **Step 3: Implement Tab/Shift+Tab in cell-edit branch of `handle_table_data_hotkeys`**

Note: `allows_hotkey_modifiers` currently rejects non-SHIFT mods; Tab has no mods. Shift+Tab arrives as `KeyCode::BackTab` in crossterm (not Shift+Tab) — handle `KeyCode::BackTab` explicitly.

- [ ] **Step 4: PASS + commit**

```bash
jj describe -m "feat(d7s): Tab/BackTab moves inline edit across columns"
jj new
```

---

### Task 5: Create keys `n` / `N` / `c` + `a` alias + auto-enter edit

**Files:**
- Modify: `crates/d7s/src/table_data_actions.rs` (`handle_table_data_hotkeys`, `table_data_add_blank_draft`, add `table_data_add_defaults_draft` stub that for now equals blank — real defaults in Task 6)
- Update existing tests that send `key('a')` for drafts to `key('n')` (keep one test that `a` still works)
- Hotkeys/help can wait for Task 7

**Interfaces:**
- `n` → blank draft + `begin_cell_edit` on first editable col  
- `N` → defaults draft (Task 6 body) + auto-enter  
- `c` → duplicate (unchanged) + auto-enter  
- `a` → same as `n` (alias)  
- Stop matching `a|A` / `c|C` / `s|S` as pairs where case matters: use `Char('n')`, `Char('N')`, `Char('c')`, `Char('s')`, `Char('a')` separately. Keep `Char('D')` for delete. Refresh: `Char('r')` only (or keep `R`).

- [ ] **Step 1: Failing tests**

```rust
#[tokio::test]
async fn n_creates_draft_and_enters_edit() {
    let mut app = tables_app("key_n").await;
    run_cmd(&mut app, "table users").await;
    app.on_key_event(key('n')).await.unwrap();
    assert!(app.has_table_draft_rows());
    assert!(app.is_cell_editing());
}

#[tokio::test]
async fn a_aliases_n() {
    let mut app = tables_app("key_a").await;
    run_cmd(&mut app, "table users").await;
    app.on_key_event(key('a')).await.unwrap();
    assert!(app.has_table_draft_rows());
}
```

- [ ] **Step 2–4: Implement, fix old `key('a')` tests, PASS, commit**

```bash
jj describe -m "feat(d7s): n/N/c create drafts; a aliases n; auto-enter edit"
jj new
```

---

### Task 6: Defaults (`N`) + skip non-editable columns

**Files:**
- Modify: `crates/d7s/src/table_data_actions.rs` — `table_data_add_defaults_draft`
- Modify: `crates/d7s/src/cell_edit.rs` / actions — build `EditableCol` from `get_columns`
- Test: sqlite table with DEFAULT

**Rules (from spec):**
- Prefill display: for columns with a usable `default_value`, leave cell **empty** so `should_omit_for_insert_default` / INSERT omits and DB applies default (matches existing insert path) **or** show a placeholder like `(default)` only if you can round-trip — **prefer empty + omit** (ponytail; already how insert works).
- Nullable no default → empty (NULL on insert).
- NOT NULL no default → empty; commit may error (OK).
- Non-editable: serial/identity/`nextval`/sqlite integer PK rowid — `editable: false`; Tab skips; `begin_cell_edit` on that col sets status and refuses.
- Status: `Draft with defaults (N cols)` where N = columns that will omit-for-default.

- [ ] **Step 1: Failing test**

```rust
#[tokio::test]
async fn N_draft_omits_default_columns_on_commit() {
    // CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT DEFAULT 'x', note TEXT);
    // N → edit note only → s → SELECT shows name='x'
}
```

- [ ] **Step 2: Implement using `database.get_columns` + `should_omit_for_insert_default` heuristics for editable flags**

```rust
fn column_is_editable(col: &Column, engine_sqlite: bool, sqlite_rowid_pk: bool) -> bool {
    if engine_sqlite && sqlite_rowid_pk { return false; }
    let u = col.data_type.to_lowercase();
    if u.contains("serial") || u.contains("identity") { return false; }
    if col.default_value.as_deref().is_some_and(|d| d.contains("nextval")) {
        return false;
    }
    true
}
```

- [ ] **Step 3: Tab uses real editable flags; PASS; commit**

```bash
jj describe -m "feat(d7s): N draft with defaults; skip generated columns"
jj new
```

---

### Task 7: Discoverability (hotkeys, help, status, idle hint)

**Files:**
- Modify: `crates/d7s/src/ui/widgets/hotkeys.rs`
- Modify: `crates/d7s/src/ui/widgets/help_content.rs`
- Modify: `crates/d7s/src/ui/widgets/idle_hint.rs` + `rendering.rs` (pass enough context for TableData hint)
- Modify: `crates/d7s/src/rendering.rs` — choose hotkey slice:

```rust
if app.cell_edit.is_some() {
    CELL_EDIT_HOTKEYS.iter()
} else {
    TABLE_DATA_VIEW_HOTKEYS.iter() // updated labels: n New, N Defaults, …
}
```

**Constants:**

```rust
pub const TABLE_DATA_VIEW_HOTKEYS: [Hotkey; 6] = [
    Hotkey::new('n', "New row"),
    Hotkey::new('N', "Defaults"),
    Hotkey::new('c', "Duplicate"),
    Hotkey::new('s', "Commit"),
    Hotkey::new('D', "Delete"),
    Hotkey::new('i', "Edit"),
];

pub const CELL_EDIT_HOTKEYS: [Hotkey; 5] = [
    // Hotkey may need KeyCode variants — if Hotkey only supports char,
    // use descriptive labels in a small Vec built manually or extend Hotkey.
];
```

Check `k9tui::widgets::hotkey::Hotkey` — if char-only, represent Tab as `Hotkey` with a char placeholder or add a `Hotkey::label_only` if one exists; otherwise show Tab bindings only in status/`?` help (ponytail: help + status first, bar shows `Esc Cancel` via a char if needed).

Idle hint: when `DatabaseExplorerState::TableData`, prefer `n new row · Enter edit cell · ? help`.

- [ ] **Step 1: Test render_text / help_rows contains new strings**

- [ ] **Step 2: Implement; PASS; commit**

```bash
jj describe -m "feat(d7s): table-data hotkeys/help/status for inline edit"
jj new
```

---

### Task 8: Row-ops polish + modal escape hatch

**Files:**
- Modify: `crates/d7s/src/table_data_actions.rs` — in cell-edit branch: Space inserts space (already if Char handled); `D` not reached; Browse `D`/`y` unchanged
- Modify: `crates/d7s/src/event_handlers.rs` / table hotkeys — Ctrl+e or `e` while editing opens `open_cell_value_modal` with current buffer; on modal apply, clear `cell_edit`
- Ensure `handle_table_data_hotkeys` runs cell-edit handler before Browse `D`

- [ ] **Step 1: Tests**

```rust
#[tokio::test]
async fn d_ignored_while_editing() {
    // begin edit on saved row; press D; still editing; row not deleted
}

#[tokio::test]
async fn ctrl_e_opens_cell_modal_while_editing() {
    // begin edit; Ctrl+e; assert modal_manager.get_cell_value_modal().is_some()
}
```

- [ ] **Step 2: Implement; PASS; commit**

```bash
jj describe -m "feat(d7s): cell-edit modal escape hatch; ignore Del while editing"
jj new
```

---

### Task 9: Type-to-overwrite + Backspace/Delete entry from Browse

**Files:**
- Modify: `crates/d7s/src/table_data_actions.rs` Browse branch (when not editing):
  - `KeyCode::Backspace` | `Delete` → `begin_cell_edit(Some(String::new()))`
  - `KeyCode::Char(c)` if `!is_table_data_browse_binding(c)` && not a navigation conflict → `begin_cell_edit(Some(c.to_string()))`
- Do **not** steal `j`/`k`/`h`/`l`/`g`/`G`/`0`/`$` — treat those as browse bindings in `is_table_data_browse_binding`.

- [ ] **Step 1: Test** unbound `z` starts edit with buffer `z`; `n` still creates row

- [ ] **Step 2: Implement; PASS; commit**

```bash
jj describe -m "feat(d7s): type-to-overwrite and Backspace enter cell edit"
jj new
```

---

### Task 10: Spec coverage smoke + CHANGELOG

**Files:**
- Modify: `crates/d7s/CHANGELOG.md` — under Unreleased, note inline edit + key changes
- Run: `cargo test -p d7s` full package
- Manual checklist (note in commit body): n/N/c, Tab, Esc, s, saved cell update, Ctrl+e

- [ ] **Step 1: Full test**

Run: `cargo test -p d7s`  
Expected: all pass

- [ ] **Step 2: CHANGELOG entry**

- [ ] **Step 3: Commit**

```bash
jj describe -m "docs(d7s): changelog for table data inline edit"
jj new
```

---

## Spec coverage checklist (plan self-review)

| Spec item | Task |
|-----------|------|
| Browse vs Cell edit modes | 2–4 |
| Hybrid commit saved vs draft | 3 |
| Esc cell revert vs draft discard | 2 + existing Esc |
| `n` / `N` / `c` / `a` alias | 5–6 |
| Tab / Shift+Tab same row | 4 |
| Auto-enter after create | 5 |
| Defaults + skip generated | 6 |
| Modal Ctrl+e / `e` in edit only | 8 |
| Hotkey bar / help / status / idle | 7 |
| Space/D/y polish while editing | 8 |
| Type-to-overwrite unbound | 9 |
| Filter/draft guards unchanged | reuse existing |
| No k9s border strip | display via cell buffer |

## Placeholder / consistency notes

- Crossterm Shift+Tab = `KeyCode::BackTab` (Task 4).
- `Hotkey` char limits: degrade Tab labels into help/status if needed (Task 7).
- Defaults display = empty cells + INSERT omit (Task 6), not literal DEFAULT expression text.
