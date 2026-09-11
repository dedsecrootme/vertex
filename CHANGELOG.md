# Changelog

Toutes les evolutions notables du projet Vertex sont documentees ici.

## [0.1.1] - 2026-09-04

### Correctifs
- Le crawl de l app desktop utilise la meme base SQLite que l interface
  (%APPDATA%\\Vertex). Avant, l app lisait %APPDATA% et le crawl ecrivait
  dans le workspace -> rien n apparaissait.
- Plus de console en arriere-plan sur l app release (windows_subsystem).
- Duree de crawl configurable dans l interface (10 min a 10 h) au lieu de 30 s.

### Outils
- Ajout de `just` (task runner, fichier `justfile`) et `cargo-watch` (dev).

### Distribution
- Landing Vercel pointe vers la Release GitHub v0.1.0 (telechargement public).
- Repo GitHub dedsecrootme/vertex passe en public.

### Nettoyage
- Suppression des dossiers vides (documentation placeholders) et du docker-compose vide.
- .gitignore complete (.vercel/, gen/, logs).

## [0.14.0] - 2026-08-31

- Recherche SQLite FTS5 (BM25), pagination, snippets.

## [0.13.0] - 2026-08-31

- Crawler multi-sites multi-thread, limite de duree, contenu nettoye.
- Serveur web axum et interface de recherche.
- App desktop Tauri.
- Corrections : code mort supprime, chemin de base fiable, resolver 3.

