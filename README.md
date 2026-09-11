# Vertex

Moteur de recherche personnel écrit en Rust : crawler multi-sites, contenu nettoyé,
recherche plein texte (SQLite FTS5), serveur web (axum) et application desktop native (Tauri).

## Composants
- `apps/crawler-rs` : crawler (lib) + CLI. Suit les liens externes, multi-thread, budget de temps.
- `apps/search-rs` : bibliothèque de recherche FTS5 (BM25) + récupération de contenu.
- `apps/api-rs` : serveur HTTP (axum) servant l'interface web et l'API.
- `apps/desktop` : app Windows native (Tauri) — interface + bouton Crawler en arrière-plan.
- `apps/web` : interface web (HTML/JS) double mode (Tauri ou navigateur).
- `apps/landing` : page de présentation déployée sur Vercel.
- `database/` : base SQLite (`vertex.db`).

## Téléchargement
- Landing : https://landing-gamma-dun-83.vercel.app
- Release GitHub : https://github.com/dedsecrootme/vertex/releases

## Build & exécution
```sh
# workspace (crawler/search/api)
cargo build --workspace

# crawler CLI (300 s par défaut)
cargo run -p crawler-rs

# serveur web sur 127.0.0.1:3000
cargo run -p api-rs

# app desktop (installeurs dans target/release/bundle/)
cd apps/desktop/src-tauri && cargo tauri build

# landing (déploiement Vercel)
cd apps/landing && vercel deploy --prod --yes
```

## Outils
`just` (task runner, commandes build/web/desktop/deploy) et `cargo-watch` (redev). Voir le `justfile`.

La base de l'app desktop est stockée dans `%APPDATA%\Vertex` (writable) ; le crawl écrit
dans la même base.

## Licence
MIT — voir `LICENSE`.
