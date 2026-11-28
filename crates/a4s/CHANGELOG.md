# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [0.1.0] - Unreleased

Initial release.

### Added
- k9s-style TUI for running CLI coding agents side by side, built on `k9tui`.
- Agent sessions in PTYs with a vt100 screen, scrollback, bracketed paste and the kitty keyboard protocol.
- Answers to terminal queries agents wait on (cursor position, device attributes, kitty flags).
- Sessions table with working / blocked / done / idle / exited status, from `a4s notify` hooks or output detection.
- Agent view with single-press chords: `Ctrl+]` back to sessions (also `Esc` once the agent has exited), `Ctrl+1`-`Ctrl+9` focus and `Ctrl+q` quit; every other key goes to the agent.
- New session dialog (name, agent, directory), kill and forget with confirmation, restart with resume.
- A bell when a background session becomes blocked.
- Sessions saved per git repo (`.a4s/sessions.toml`) or in the state directory, and restored at startup.
