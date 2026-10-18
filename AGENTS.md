## Learned User Preferences

- Keep d7s visually and interaction-wise k9s-like (layout, density, and flow should read as a k9s-style TUI).
- Prefer real ratatui `Block` / `Table` / `Paragraph` widgets with borders over custom borderless or hand-drawn layouts.
- Do not strip the ASCII logo or Block borders for “density”; a compact layout that removes branded panel structure is a rejected look.
- When restyling, match k9s as the visual reference but keep the widget-based structure of the polished baseline screenshot—do not abandon ratatui widgets.
- Funnel help and similar lists through the main navigable table (Key / Description style), not a separate boxed panel.
- Prefer jj for local branch/bookmark workflows; verify a bookmark before merge and be careful with `main`.
- Demo/VHS recordings must not leak local paths; prefer `just` + docker compose (or isolated demo HOME) over one-off prep scripts.

## Learned Workspace Facts

- d7s is a Ratatui TUI database client for PostgreSQL, MySQL/MariaDB, and SQLite, inspired by k9s.
- Common tasks run through `just`; demos use VHS (`just demo`) with an isolated demo HOME so recordings stay fake-data-only.
- Large table browsing uses virtual/paginated loading rather than pulling all rows at once.
- Passwords use the platform keyring (or prompt every time if not saved).
