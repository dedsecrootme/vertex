# Changelog

Toutes les evolutions notables du projet Vertex sont documentees ici.

## [Unreleased]

### Moteur de crawl (réécriture complète)
- File d'URL persistante avec statuts (`pending`, `crawling`, `done`, `failed`,
  `blocked`, `skipped`), profondeur, priorité et nombre de tentatives.
- Concurrence multi-thread, **délai de politesse par domaine**, reprise après arrêt.
- Respect de `robots.txt` (Allow/Disallow/Crawl-delay) et découverte automatique des
  `sitemap.xml` (y compris les index de sitemaps).
- Détection des types de contenu, plafond de taille, redirections bornées,
  User-Agent identifiable, protection contre le SSRF (adresses privées/loopback).

### Sûreté des liens (100 % local)
- Nouveau module `safety` : blocklist embarquée + blocklist utilisateur
  (`database/blocklist.txt`) + heuristiques (IP brute, punycode, raccourcisseurs,
  exécutables/archives, ports inhabituels, identifiants dans l'URL).
- Verdict stocké par URL (`url_safety`) ; les liens malveillants ne sont **jamais**
  visités (statut `blocked`).

### Visionneuse assainie (« voir le site au complet »)
- Extraction des **liens complets** (URL absolue + ancre + externe + sûreté) et des
  **images** (URL + texte alternatif), stockés en base.
- Assainisseur HTML : suppression des scripts, iframes, formulaires, pubs, bandeaux
  cookies, trackers (doubleclick, taboola, outbrain, analytics…) et des paramètres
  d'affiliation/suivi (`utm_*`, `fbclid`, `gclid`, `affid`…).
- Images et favicons servis via un **proxy local** (API `api-rs` ou commande Tauri
  `fetch_media`) : le navigateur ne contacte jamais un tiers (aucun tracker).

### Signets et contact administrateur
- Signets : table dédiée, endpoints API + commandes Tauri, interface dédiée.
- Contact admin : formulaire, stockage des messages, boîte de réception (admin).

### Base de données
- Schéma versionné avec migrations automatiques (v1 → v2), index sur statut, domaine,
  priorité et sûreté ; tables `links`, `images`, `bookmarks`, `contact_messages`,
  `url_safety`, `crawl_runs`, `meta` ; FTS5 synchronisé automatiquement.
- Optimisation post-crawl (`wal_checkpoint`, `ANALYZE`).

### Correctifs
- Nettoyage des textes corrompus (mojibake) dans `tauri.conf.json`, `parser.rs`
  et `main.rs`.

### Outillage
- Toolchain Rust unifiee sur rustup + `stable-x86_64-pc-windows-msvc` ; suppression de
  l installation standalone `Rust stable GNU 1.96` (source du melange de toolchains :
  E0107 sur indexmap, E0658 sur embed-resource).
- PATH machine/utilisateur nettoye et deduplique ; `%USERPROFILE%\.cargo\bin` prioritaire.
- Ajout des outils Cargo : `cargo-binstall`, `cargo-nextest`, `cargo-edit`, `bacon`,
  `cargo-expand`, `cargo-machete`, `cargo-llvm-cov`, `sccache`, `taplo`, `typos`.
- Retrait des outils redondants : `rls` (deprecie), `cargo-watch` (remplace par `bacon`),
  `cargo-outdated` (remplace par `cargo-edit`), `cargo-geiger`.
- Extensions VS Code : rust-analyzer, CodeLLDB, Even Better TOML, Dependi, Error Lens.

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

