# Vertex

Moteur de recherche personnel ÃƒÆ’Ã‚Â©crit en Rust. Ce projet est un workspace Cargo
organisÃƒÆ’Ã‚Â© en plusieurs binaires liÃƒÆ’Ã‚Â©s entre eux via une base SQLite.

## Structure

- `apps/crawler-rs` : crawler web (tÃƒÆ’Ã‚Â©lÃƒÆ’Ã‚Â©charge, analyse et stocke les pages).
- `apps/search-rs` : bibliothÃƒÆ’Ã‚Â¨que de recherche (titre + contenu, snippets).
- `apps/indexer-rs` : (ÃƒÆ’Ã‚Â  implÃƒÆ’Ã‚Â©menter) indexation avancÃƒÆ’Ã‚Â©e.
- `apps/api-rs` : serveur HTTP (axum) ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â recherche, contenu et page web.
- `apps/web` : interface web (HTML/JS) embarquÃƒÆ’Ã‚Â©e dans le binaire `api-rs`.
- `database/` : base SQLite locale (`vertex.db`).

## PrÃƒÆ’Ã‚Â©requis

- Rust (ÃƒÆ’Ã‚Â©dition 2024) : `rustc`/`cargo` >= 1.85.

## Build

```sh
cargo build
```

## Crawler

Crawl les pages et remplit la base (sauvegarde le contenu texte) :

```sh
cargo run -p crawler-rs
```

## Serveur web (moteur de recherche)

Lance le serveur, puis ouvre `http://127.0.0.1:3000` dans un navigateur :

```sh
cargo run -p api-rs
```

### Endpoints

- `GET /` : page web de recherche.
- `GET /api/search?q=terme&offset=n` : recherche plein texte (JSON), paginÃƒÂ© par `offset`.
- `GET /api/page?url=ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â¦` : contenu complet d'une page (JSON).
- `GET /health` : ÃƒÆ’Ã‚Â©tat du serveur.

## Application desktop (Tauri)

Une application **Windows native** (Tauri v2) qui embarque l'interface de
recherche et un bouton **Â« Crawler Â»** (crawl en arriÃ¨re-plan).

```sh
cd apps/desktop/src-tauri
cargo build        # produit target/debug/desktop.exe
# installateur optimisÃ© :
# cargo tauri build    # release : installeurs + desktop.exe optimisÃ© (voir ci-dessous)
```
## Licence

Voir le fichier `LICENSE`.