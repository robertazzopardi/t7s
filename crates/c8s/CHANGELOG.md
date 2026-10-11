# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [0.1.0] - 2026-10-10

Initial release.

### Added
- k9s-style TUI for Docker built on `k9tui`.
- Container list (name, image, status, ports, uptime, CPU and memory) polled from the daemon; `a` toggles running-only.
- Container actions: start/stop (`s`), restart (`r`), exec shell (`e`), remove with a confirmation that defaults to No (`D`/`Delete`).
- Image, volume and network lists, switched with `1`-`4` or the `:` command bar, with remove (`D`).
- `/` live substring filter on every list view.
- Live log tail (`l`) with `/` search and `d` to cycle dedup (off, exact, similar) of repeated lines.
- Process view (`p`) listing container processes, with per-process output (`l`).
- Describe view (`d` or `Enter`) for containers, images, volumes and networks.
- Daemon health panel (`i`).
- Connection error screen with retry when the Docker daemon is unreachable.
