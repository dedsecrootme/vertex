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

## Moteur

- **Crawler complet** : file d'URL persistante (statuts, profondeur, priorité,
  tentatives), concurrence multi-thread, délai de politesse par domaine, respect de
  `robots.txt`, découverte des `sitemap.xml`, reprise après arrêt.
- **Sûreté des liens** (100 % local) : blocklist embarquée + `database/blocklist.txt`
  + heuristiques (IP brute, punycode, raccourcisseurs, exécutables, affiliation…).
  Les liens malveillants ne sont jamais visités.
- **Visionneuse assainie** : chaque page est réindexée avec ses **liens complets**
  et ses **images**, et son HTML est nettoyé (aucun script, pub, tracker ni
  paramètre d'affiliation). Les images et favicons passent par un **proxy local**,
  donc le navigateur ne contacte jamais un tiers.
- **Signets** et **contact administrateur** intégrés (API + interface).
- **Base SQLite versionnée** avec migrations, index et index FTS5 synchronisé.

## Outils

Toolchain : **rustup** + `stable-x86_64-pc-windows-msvc` (obligatoire pour Tauri sous Windows).
Toujours utiliser le `cargo`/`rustc` de rustup ; `%USERPROFILE%\.cargo\bin` doit rester en tête
du `PATH` (aucune autre installation Rust dans le `PATH`).

Extensions Cargo (toutes via `cargo` ou `cargo binstall`) :
`just` (task runner, voir `justfile`), `cargo-binstall`, `bacon` (redev en continu),
`cargo-nextest` (tests), `cargo-edit` (`cargo add/rm/upgrade`), `cargo-expand` (macros),
`cargo-machete` (dépendances inutilisées), `cargo-llvm-cov` (couverture), `sccache`
(cache de compilation), `cargo-audit` (sécurité), `cargo-deny` (licences/bans),
`cargo-tauri` (CLI Tauri), `taplo` (TOML), `typos` (fautes).

La base de l'app desktop est stockée dans `%APPDATA%\Vertex` (writable) ; le crawl écrit
dans la même base.

## Communauté (`apps/hub-rs`)

**vx-hub** : chat en direct, forums (catégories → sujets → réponses), avis sur
le logiciel (ouverts avec les premiers utilisateurs) et partenaires (publicité
de serveur Discord, payante et modérée).

```sh
# démarrer le hub sur le port 8787 (admin facultatif)
cargo run -p hub-rs -- 8787
VX_ADMIN_USER=monadmin VX_ADMIN_PASS=mot-de-passe-suffisamment-long cargo run -p hub-rs -- 8787
```

Serveur HTTP **écrit à la main** (aucune dépendance réseau), aléa et
PBKDF2 maison, stockage `database/hub.db` (séparé de l'index).

## Confidentialité & IA

Vertex est **local et souverain** : aucune IA n'a le droit d'exploiter vos données,
rien ne quitte la machine, aucun log n'est écrit. Voir **[POLICY.md](POLICY.md)**.

## Licence
MIT — voir `LICENSE`.
