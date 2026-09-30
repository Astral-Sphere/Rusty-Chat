# Rusty-Chat task runner
default:
    @just --list

# Fast compile check
check:
    cargo check --workspace

# Run all tests (Postgres tests skipped unless RC_TEST_PG_URL is set)
test:
    cargo test --workspace

# Full test run including live Postgres (starts the fixture container if needed)
test-pg:
    #!/usr/bin/env bash
    set -euo pipefail
    just pg-up >/dev/null
    export RC_TEST_PG_URL='postgres://postgres:fixture@127.0.0.1:5433/postgres'
    cargo test --workspace

# Lint (CI gate: zero warnings)
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

# Start the Postgres fixture container (podman) for PG tests
pg-up:
    #!/usr/bin/env bash
    set -euo pipefail
    if podman container exists rc-pg-fixture; then
        podman start rc-pg-fixture >/dev/null
    else
        podman run -d --name rc-pg-fixture \
            -e POSTGRES_PASSWORD=fixture -e POSTGRES_DB=fixture \
            -p 127.0.0.1:5433:5432 docker.io/library/postgres:17-alpine >/dev/null
    fi
    for i in $(seq 1 30); do
        podman exec rc-pg-fixture pg_isready -U postgres >/dev/null 2>&1 && exit 0
        sleep 1
    done
    echo "postgres did not become ready" >&2; exit 1

# Stop the Postgres fixture container
pg-down:
    podman stop rc-pg-fixture

# Build the Dioxus frontend (wasm) and copy the bundle into web/dist
web-build: web-css
    cd web && dx build --release
    rm -rf web/dist
    mkdir -p web/dist
    cp -r web/target/dx/rusty-chat-web/release/web/public/. web/dist/

# Single-binary release build: frontend embedded via rust-embed
web-release: web-build
    cargo build --release -p rusty-chat --features embed-frontend

# Regenerate web/assets/tailwind.css (Tailwind v4 standalone CLI, no Node).
# The generated file is gitignored — dx build / bare cargo builds in web/
# need it to exist first. Add --watch in a second terminal during dev.
web-css:
    cd web && tailwindcss -i input.css -o assets/tailwind.css --minify

# Full CI gate
ci:
    just fmt-check
    just clippy
    just test-pg
