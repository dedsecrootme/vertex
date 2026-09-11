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

# Deploy la landing sur Vercel
deploy:
    cd apps/landing && vercel deploy --prod --yes

# Test rapide du workspace (check)
check:
    cargo check --workspace
