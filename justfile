default:
    @just --list

build:
    cargo build

build-release:
    cargo build --release --locked

run:
    cargo run

test:
    cargo test --workspace --locked --all-features --all-targets

# Nightly rustfmt (rustfmt.toml needs nightly). RUST_NIGHTLY_BIN or pinned rustup toolchain.
fmt *ARGS:
    #!/usr/bin/env bash
    set -e
    if [ -n "${RUST_NIGHTLY_BIN:-}" ]; then
        PATH="${RUST_NIGHTLY_BIN}:$PATH" cargo fmt -- {{ARGS}}
    else
        rustup run nightly-2026-09-08 cargo fmt -- {{ARGS}}
    fi

fmt-check *ARGS: (fmt "--check" ARGS)

# Same flags as CI (stable).
clippy *ARGS:
    cargo clippy --workspace --all-features --all-targets {{ARGS}} -- -D warnings -W clippy::all -W clippy::pedantic -W clippy::nursery -W clippy::cargo -A clippy::multiple_crate_versions

clippy-fix: (clippy "--fix" "--allow-dirty")

cov:
    cargo llvm-cov --all-features --all-targets

cov-report:
    cargo llvm-cov report --all-features --all-targets

cov-html:
    cargo llvm-cov html --all-features --all-targets
    @echo "Open target/llvm-cov/html/index.html"

cov-lcov:
    cargo llvm-cov lcov --all-features --all-targets --output-path lcov.info

check: fmt-check clippy test
    @echo "All checks passed"

# Isolated HOME + sample DBs for vhs (no real connections).
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

demo: demo-prep build-release
    cd crates/d7s && vhs demo.tape

c8s-demo: c8s-demo-prep
    cargo build --release --locked -p c8s
    cd crates/c8s && vhs demo.tape
    just c8s-demo-teardown

c8s-demo-prep:
    docker compose -f crates/c8s/demo/docker-compose.yml up -d

c8s-demo-teardown:
    docker compose -f crates/c8s/demo/docker-compose.yml down

docker-up:
    cd crates/d7s && docker compose up -d

# TLS fixtures: Postgres 15432, MariaDB 3308/3310; CA at crates/d7s/docker/certs/ca.crt
docker-tls-up:
    crates/d7s/docker/gen-tls-certs.sh
    cd crates/d7s && docker compose --profile tls up -d --force-recreate postgres postgres-tls mariadb-tls mariadb-plain

docker-down:
    cd crates/d7s && docker compose down

docker-logs:
    cd crates/d7s && docker compose logs -f

# Publish then tag. Order: k9tui → d7s → c8s. Tags: k9tui-v* / v* / c8s-v*.
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
