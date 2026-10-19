default: help

# --- Vertex: commandes projet ---
help:
    @just --list

# Build du workspace (crawler, search, api)
build:
    cargo build --workspace

# Crawler CLI (300s depuis la racine)
crawl:
    cargo run -p crawler-rs

# Serveur web (api-rs) sur 127.0.0.1:3000
web:
    cargo run -p api-rs

# Build et bundle l app desktop Tauri
desktop:
    cd apps/desktop/src-tauri && cargo tauri build

# Publication explicite du site Cloudflare (compte/bucket a verifier avant usage)
deploy:
    npx wrangler deploy --config apps/landing/wrangler.jsonc

crawl-continuous:
    cargo run -p crawler-rs -- --continuous

# Test rapide du workspace (check)
check:
    cargo check --workspace

test:
    cargo test --workspace --locked

lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

fmt:
    cargo fmt --all
    cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml

audit:
    cargo audit
    cargo audit --file apps/desktop/src-tauri/Cargo.lock

smoke:
    cargo build -p api-rs --locked
    node tests/api-smoke.mjs
    node tests/api-smoke.mjs --public

browser:
    cargo build -p api-rs --locked
    npm run test:web
