#!/bin/sh
# Runs the release binary in demo mode (canned data only).
ROOT="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$ROOT/../.." && pwd)"
exec "$REPO_ROOT/target/release/p7s" --demo "$@"
