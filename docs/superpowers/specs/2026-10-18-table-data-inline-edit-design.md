# Table data: inline edit and keyboard row ops

## Goal

Make table-data insert/edit/delete faster and more self-discoverable for a keyboard-centric TUI: spreadsheet-style inline cell editing, clear draft vs saved commit rules, and consistent create keys — without abandoning ratatui `Table` / `Block` chrome or the existing draft-row machinery.

## Non-goals

- Filter, `:jump`, describe, or SQL-editor UX changes (except existing draft-guards).
- Full vim Normal/Insert dual-mode.
- A separate “compose row” UI outside the grid.
- Background/autosave of drafts across sessions.
- Changing multi-select or delete confirmation semantics beyond the polish notes below.

## Current baseline

- Draft rows: `a` blank, `c` duplicate (PKs cleared), `s` commit INSERT, Esc two-step discard.
- Cell edit: `Enter` → modal (`CellValueModal`).
- Saved cell updates already support in-memory apply after DB write.
- Hotkey bar / `?` document `a`/`c`/`s`/`D`; Enter edit is help-only (hidden from bar).

## Approach

**Spreadsheet-first** on top of existing drafts:

- **Browse** — navigation and commands (letters are bindings, not typing).
- **Cell edit** — caret in the focused cell; Tab walks columns.
- **Modal** — escape hatch for long / multi-line values (`Ctrl+e` / `e` while editing).

## Modes and commit rules

### Browse (default)

- Motion: hjkl / arrows, `g`/`G`, `0`/`$`, `/`, etc. (unchanged).
- Create: `n` blank draft, `N` draft with defaults, `c` duplicate (PKs cleared).
- `a` aliases `n` for one release, then removed.
- `s` commits the selected draft (INSERT).
- `D` delete, Space multi-select, `y`/`Y` copy, `r` refresh — as today, with polish below.
- `Enter` / `i` enter Cell edit on the focused cell.

### Cell edit

- Entry from Browse:
  - `Enter` or `i` — edit current cell (preserve contents; caret at end).
  - Backspace / Delete — start edit with an empty cell.
  - Unbound printable character — start edit and **replace** contents with that character. **Browse bindings always win** (`n`/`N`/`c`/`s`/`r`/`d`/`y`/…); only keys that are not table-data (or global) bindings type-to-overwrite.
- After `n` / `N` / `c`, auto-enter Cell edit on the first editable column so Tab-through works immediately.
- Tab — commit-or-stash cell, move to next editable column on the **same row** (wrap within row; do not jump to another saved row).
- Shift+Tab — previous editable column on the same row.
- Enter — commit-or-stash cell, return to Browse (cursor stays on that cell).
- Esc — revert the active cell to its pre-edit value, return to Browse (draft row remains until draft discard).
- Modal escape hatch: `Ctrl+e` while in Cell edit. Plain `e` in Cell edit also opens the modal; in Browse, `e` remains SQL editor (unchanged).
- Space inserts a space (does not toggle multi-select).
- In Cell edit, printable keys go to the cell buffer (including `y`/`Y`/`d`/…). Copy (`y`/`Y`) and delete (`D`) only work in Browse; `D` is ignored while editing.

### Commit hybrid

| Row kind   | Leaving a cell (Tab / S-Tab / Enter-done) | Explicit `s`      | Esc                         |
|-----------|--------------------------------------------|-------------------|-----------------------------|
| Saved     | Auto-commit that cell to the DB            | N/A (or no-op)    | Revert cell, Browse         |
| Draft     | Stash in memory only                       | INSERT whole row  | Cell revert; draft discard is separate two-step Esc in Browse |

- Saved-cell DB failure: stay in (or re-enter) Cell edit; status shows the error; prior value remains until a successful write.
- Prefer in-memory grid update after successful cell write (existing path); no forced full-page refresh.

### Draft lifecycle

- Unchanged guards: filter must be clear to add/dup/commit/delete; commands that would drop the page while a draft exists remain blocked; Esc warn-once then discard.
- `r` refresh: keep draft-guard / warn behavior.
- Draft styling in the table retained.

### Defaults (`N`)

- Prefill from schema DEFAULT where the driver exposes it.
- Else: NULL for nullable columns; for NOT NULL without default, leave empty and let commit surface the DB error (status may note how many cols got defaults: e.g. `Draft with defaults (3 cols)`).
- Skip / do not Tab-stop on read-only or generated columns (identity, computed); status explains if the user tries to edit one.

## Discoverability

### Hotkey bar (context-sensitive)

- Browse: `n New` · `N Defaults` · `c Dup` · `s Commit` · `D Del` · `i/Enter Edit` · `? Help`
- Cell edit: `Tab Next` · `S-Tab Prev` · `Enter Done` · `Esc Cancel` · `^e Modal`
- While a draft exists in Browse, keep `s Commit` prominent (first or highlighted).

### Status line

Short mode tokens, for example:

- `EDIT col=email`
- `DRAFT · s insert · Esc discard`
- `UPDATED email` / `INSERT failed: …`

### Help (`?`)

Rewrite the table-data section to match bindings and the hybrid commit rule; document `a` → `n` while the alias exists.

### Idle hint

When the table is empty or on first meaningful idle in table data: `n new row · Enter edit cell` (reuse existing idle-hint pattern).

### Visual

Active edit cell: inverse/underline within the existing `Table` widget. No borderless custom layouts; keep Block borders and k9s-like density.

## Row-ops polish

- **Multi-select:** unchanged; refused on drafts; inactive during Cell edit.
- **Delete (`D`):** drafts in the selection removed locally without confirm; saved rows keep the confirmation modal (count + short preview). Ignored while in Cell edit.
- **Copy:** `y` cell / `Y` row TSV in Browse only.
- **Refresh (`r`):** draft-guard as today; successful cell auto-commit does not require a full reload if in-memory apply succeeds.

## Implementation sketch (non-binding)

- Reuse `table_data_actions` draft insert/dup/commit/delete.
- Add a small Cell-edit state on the explorer/app (column index, buffer, pre-edit snapshot, draft vs saved).
- Route keys in table-data Browse vs Cell edit inside `handle_table_data_hotkeys` (already runs before connection-scoped `n`/`e`/…).
- Keep `CellValueModal` for the modal escape hatch and oversized values.
- Update `TABLE_DATA_VIEW_HOTKEYS`, `help_content`, and status strings.

## Testing

- Unit/integration style tests already used in `app.rs` / table actions: draft create (`n`/`N`/`c`), alias `a`, Tab order skips read-only, Esc reverts cell, draft still needs `s`, saved cell leave triggers update path (can mock DB), `D`/Space ignored or inert in Cell edit.
- One focused check per non-trivial rule; no large fixture framework.

## Rollout

1. Inline edit + Tab + hybrid commit + hotkey/status/`?` updates.
2. Remap create keys (`n`/`N`/`c`, `a` alias).
3. Defaults (`N`) + skip generated columns.
4. Row-ops polish (delete/copy/Space while editing) and idle hint.

Ship as one feature arc; steps can be separate commits/PRs if useful, but one design.
