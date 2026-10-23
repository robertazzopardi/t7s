# p7s

Part of [t7s](https://github.com/robertazzopardi/t7s). k9s-style system process viewer on [Ratatui](https://ratatui.rs) / [k9tui](https://crates.io/crates/k9tui).

```sh
just p7s-demo   # vhs + `p7s --demo` (canned data, nothing real is read)
```

## Features

Live process list (PID, PPID, user, CPU%, MEM%, RSS, state, CPU time, command) polled with [`sysinfo`](https://crates.io/crates/sysinfo) on Linux, macOS and Windows; `/` search; sort by CPU/memory/PID/name; per-process describe; system summary (`i`); signal picker with confirmation (`K`). Keyboard-first, table-first, k9s-style.

CPU% is per core and can exceed 100% for multi-threaded processes, exactly like `top`.

## Usage

```sh
cargo install p7s --locked && p7s
# or: cargo run -p p7s
p7s --demo   # canned data, no real processes are read
```

## Hotkeys

| Key | Action |
| --- | --- |
| `j`/`k` · `g`/`G` | Move · top/bottom |
| `c` / `m` / `p` / `n` | Sort by CPU / memory / pid / name (again to flip) |
| `/` | Filter the process list |
| `:` | `:cpu` `:mem` `:pid` `:name` `:info` `:q` (Tab completes) |
| `d` / Enter | Describe the selected process |
| `i` | System summary |
| `K` / `Delete` | Signal the selected process (pick, then confirm) |
| `q` / `Ctrl-C` | Quit |

Signalling always asks first (default No), never targets p7s itself or PID 1, and re-checks the process start time so a recycled PID is refused. Signals unsupported on the platform are not offered.

## Views

v1 is the process list plus the system summary and per-process describe. No tree view or per-thread view yet.
