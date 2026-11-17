# a4s

Part of [t7s](https://github.com/robertazzopardi/t7s). k9s-style TUI for running CLI coding agents side by side on [Ratatui](https://ratatui.rs) / [k9tui](https://crates.io/crates/k9tui).

## Features

- Run several CLI coding agents (Claude Code, Codex, a shell, ...) at once, each in its own PTY.
- Sessions table with live status: working, blocked, done, idle or exited.
- Full-screen agent view; every key except the a4s chords goes straight to the agent.
- Status from agent hooks (`a4s notify`), with output-based detection as a fallback.
- A bell when a background session becomes blocked.
- Sessions are remembered between runs and can be restarted (resumed where the agent supports it).

## Usage

```sh
cargo install a4s --locked && a4s
# or: cargo run -p a4s
```

## Hotkeys

### Sessions view

| Key | Action |
| --- | --- |
| `Enter` | Open the selected session |
| `c` | New session |
| `K` | Kill the selected session (asks first) |
| `F` | Forget the selected session (asks first) |
| `r` | Restart the selected session |
| `1`-`9` / `Ctrl+1`-`Ctrl+9` | Open session N |
| `q` / `Ctrl+c` / `Ctrl+q` | Quit |

### Agent view

Keys go to the agent, including `q` and `Ctrl+c`. These chords are handled by a4s:

| Key | Action |
| --- | --- |
| `Ctrl+a` `1`-`9` | Focus session N |
| `Ctrl+a` `n` / `p` | Next / previous session |
| `Ctrl+a` `s` | Back to the sessions view |
| `Ctrl+a` `c` | New session |
| `Ctrl+a` `r` | Restart this session |
| `Ctrl+a` `?` | Show the chord help |
| `Ctrl+a` `Ctrl+a` | Send a literal `Ctrl+a` to the agent |
| `Ctrl+1`-`Ctrl+9` | Focus session N |
| `Ctrl+q` | Quit a4s |

Quitting while an agent is working asks first. Quitting stops every agent.

## Agent hooks

a4s sets `A4S_SOCKET` and `A4S_SESSION_ID` in every agent's environment. An agent hook can report its state with:

```sh
a4s notify working   # or: blocked, done
```

Outside a4s (no `A4S_SOCKET`) the command does nothing and exits 0, so hooks are safe to leave on. Unix only.

Claude Code example (`~/.claude/settings.json`):

```json
{
  "hooks": {
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "a4s notify working" }] }],
    "Notification": [{ "hooks": [{ "type": "command", "command": "a4s notify blocked" }] }],
    "Stop": [{ "hooks": [{ "type": "command", "command": "a4s notify done" }] }]
  }
}
```

## Saved sessions

Sessions are saved on every change and loaded at startup:

- Sessions inside a git repo go to `<repo>/.a4s/sessions.toml`, and `/.a4s/` is added to the repo's `.git/info/exclude`.
- Other sessions go to `$XDG_STATE_HOME/a4s/sessions.toml` (default `~/.local/state/a4s/sessions.toml`).

Two a4s instances running at once overwrite each other's session records.
