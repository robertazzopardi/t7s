# Workspace commands (d7s, c8s, k9tui)
# Run `just` or `just --list` to see all recipes

default:
    @just --list

# Build the project
build:
    cargo build

# Build release
build-release:
    cargo build --release --locked

# Run the application
run:
    cargo run

# Run tests
test:
    cargo test --workspace --locked --all-features --all-targets

# Format with nightly rustfmt (rustfmt.toml uses nightly-only options: group_imports, imports_granularity).
# Uses RUST_NIGHTLY_BIN in nix shell, or the pinned rustup nightly otherwise.
fmt *ARGS:
    #!/usr/bin/env bash
    set -e
    if [ -n "${RUST_NIGHTLY_BIN:-}" ]; then
        PATH="${RUST_NIGHTLY_BIN}:$PATH" cargo fmt -- {{ARGS}}
    else
        rustup run nightly-2026-09-08 cargo fmt -- {{ARGS}}
    fi

# Check formatting (pinned nightly rustfmt, no write)
fmt-check *ARGS: (fmt "--check" ARGS)

# Clippy: same flags as CI. CI uses the latest stable (no rust-toolchain file), so run `rustup update stable` often or new lints will be missed locally.
clippy *ARGS:
    cargo clippy --workspace --all-features --all-targets {{ARGS}} -- -D warnings -W clippy::all -W clippy::pedantic -W clippy::nursery -W clippy::cargo -A clippy::multiple_crate_versions

# Clippy and apply fixes where possible
clippy-fix: (clippy "--fix" "--allow-dirty")

# Code coverage via llvm-cov (requires cargo-llvm-cov and llvm-tools)
cov:
    cargo llvm-cov --all-features --all-targets

# Coverage report (terminal)
cov-report:
    cargo llvm-cov report --all-features --all-targets

# Coverage as HTML (opens in browser or inspect lcov-report/)
cov-html:
    cargo llvm-cov html --all-features --all-targets
    @echo "Open target/llvm-cov/html/index.html"

# LCOV report for CI / tooling
cov-lcov:
    cargo llvm-cov lcov --all-features --all-targets --output-path lcov.info

# Full check: format, clippy, test
check: fmt-check clippy test
    @echo "All checks passed"

# Demo: isolated HOME + sample DBs for vhs (does not touch real connections)
demo-prep:
    #!/usr/bin/env bash
    set -euo pipefail
    cd crates/d7s
    rm -rf .demo-home .demo-data
    mkdir -p ".demo-home/Library/Application Support/d7s" .demo-data
    sqlite3 .demo-data/sample.db < demo/sample.sql
    sqlite3 ".demo-home/Library/Application Support/d7s/d7s.db" < demo/connections.sql
    test "$(sqlite3 ".demo-home/Library/Application Support/d7s/d7s.db" 'SELECT count(*) FROM connections;')" = "2"
    sqlite3 .demo-data/sample.db "SELECT count(*) FROM users;" | grep -qx 4

# Demo: prep, build release binary, record demo.gif
demo: demo-prep build-release
    cd crates/d7s && vhs demo.tape

# c8s demo: start fake containers, build, record demo.gif, tear down
c8s-demo: c8s-demo-prep
    cargo build --release --locked -p c8s
    cd crates/c8s && vhs demo.tape
    just c8s-demo-teardown

# c8s demo: start fake demo-web/demo-cache containers
c8s-demo-prep:
    docker compose -f crates/c8s/demo/docker-compose.yml up -d

# c8s demo: remove fake demo-web/demo-cache containers
c8s-demo-teardown:
    docker compose -f crates/c8s/demo/docker-compose.yml down

# Docker: start database services
docker-up:
    cd crates/d7s && docker compose up -d

# Docker: start TLS-enabled Postgres (15432) and MariaDB (3308); CA at crates/d7s/docker/certs/ca.crt
docker-tls-up:
    crates/d7s/docker/gen-tls-certs.sh
    cd crates/d7s && docker compose --profile tls up -d postgres-tls mariadb-tls

# Docker: stop services
docker-down:
    cd crates/d7s && docker compose down

# Docker: view logs
docker-logs:
    cd crates/d7s && docker compose logs -f


# Publish order (d7s and c8s depend on k9tui, so k9tui goes first):
#   just release k9tui 0.1.0   (tag k9tui-v0.1.0, no workflow)
#   just release d7s 0.5.0     (tag v0.5.0, triggers the d7s binary release workflow)
#   just release c8s 0.1.0     (tag c8s-v0.1.0, no workflow)
# Release one crate whose version is already bumped on main: publish, then tag and push.
release CRATE VERSION:
    #!/usr/bin/env bash
    set -euo pipefail

    case "{{CRATE}}" in
        d7s) TAG="v{{VERSION}}" ;;
        c8s|k9tui) TAG="{{CRATE}}-v{{VERSION}}" ;;
        *) echo "Error: CRATE must be one of: k9tui, d7s, c8s"; exit 1 ;;
    esac

    if ! git diff --quiet || ! git diff --cached --quiet; then
        echo "Error: working tree is not clean."; exit 1
    fi

    CURRENT=$(cargo pkgid -p {{CRATE}} | sed 's/.*[#@]//')
    [ "$CURRENT" = "{{VERSION}}" ] || { echo "Error: {{CRATE}} is $CURRENT in Cargo.toml, not {{VERSION}}."; exit 1; }

    CL=crates/{{CRATE}}/CHANGELOG.md
    if [ -f "$CL" ] && ! grep -q "\[{{VERSION}}\]" "$CL"; then
        echo "Error: no [{{VERSION}}] entry in $CL."; exit 1
    fi

    # Publish first: a failure (e.g. k9tui not yet on crates.io) leaves no tag behind.
    cargo publish -p {{CRATE}}
    git tag "$TAG"
    git push origin "$TAG"
    echo "Released $TAG"
