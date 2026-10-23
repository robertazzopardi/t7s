# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [0.1.0] - 2026-10-24

Initial release.

### Added
- k9s-style TUI process viewer built on `k9tui`.
- Live process list (PID, PPID, user, CPU%, MEM%, RSS, state, CPU time, command) polled from `sysinfo` on Linux, macOS and Windows.
- Sort by CPU, memory, PID or name (`c`/`m`/`p`/`n`; press again to flip direction).
- `/` live substring filter over every column.
- Per-process describe view (`d`/Enter): exe, cwd, root, start time, full command line and environment.
- System summary panel (`i`): OS, kernel, host, uptime, load average, CPUs, memory and swap.
- Signal picker (`K`/Delete) with a confirmation that defaults to No; refuses p7s itself and PID 1, and refuses a recycled PID by comparing start times.
- `:` command bar (`:cpu` `:mem` `:pid` `:name` `:info` `:q`).
- `--demo` mode with canned data, used for recordings and tests (no real processes are read).
