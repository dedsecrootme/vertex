# Vertex

by ded_sec

Moteur de recherche personnel écrit en Rust : crawler multi-sites, contenu nettoyé,
recherche plein texte (SQLite FTS5), serveur web (axum) et application desktop native (Tauri).

## Composants
- `apps/crawler-rs` : crawler (lib) + CLI. Suit les liens externes, multi-thread, budget de temps.
- `apps/search-rs` : bibliothèque de recherche FTS5 (BM25) + récupération de contenu.
- `apps/api-rs` : serveur HTTP (axum) servant l'interface web et l'API.
- `apps/desktop` : app Windows native (Tauri) — interface + bouton Crawler en arrière-plan.
- `apps/web` : interface web (HTML/JS) double mode (Tauri ou navigateur).
- `apps/landing` : site public Vercel ; distribution R2 conservee pour une activation ulterieure.
- `database/` : base SQLite (`vertex.db`).

## Téléchargement
La version corrigee doit etre reconstruite et validee avant distribution.
Les anciens installateurs 0.1.0 ne sont pas la version corrigee.
- Nouvelle distribution : installateur compile et signe servi depuis R2, sans archive de sources.
- Migration, serveur continu et participation volontaire : voir `CLOUDFLARE.md`.

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

# validation du site Cloudflare, sans publication
npm run cloudflare:check
```

## Moteur

- **Crawler complet** : file d'URL persistante (statuts, profondeur, priorité,
  tentatives), concurrence multi-thread, délai de politesse par domaine, respect de
  `robots.txt`, découverte des `sitemap.xml`, reprise après arrêt.
- **Sûreté des liens** (100 % local) : blocklist embarquée + `database/blocklist.txt`
  + heuristiques (IP brute, punycode, raccourcisseurs, exécutables, affiliation…).
  Les liens signalés comme malveillants sont bloqués ; les heuristiques ne
  constituent pas une garantie d'innocuité des autres liens.
- **Visionneuse assainie** : chaque page est réindexée avec ses **liens complets**
  et ses **images**, et son HTML est nettoyé avec une liste de balises autorisées.
  Les scripts, attributs exécutables et paramètres de suivi connus sont retirés.
  Les images et favicons passent par un **proxy local**,
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

La base de l'app desktop est stockée dans `%APPDATA%\com.vertex.app` ; le crawl écrit
dans la même base.

Pour tous les composants locaux, `VERTEX_DB_DIR` permet de choisir un autre
dossier de base. `VERTEX_PORT` permet de changer le port local de l'API (3000 par
défaut). Le bouton de crawl reste désactivé pendant toute l'exécution.

Une base précédemment créée dans `database/` reste à cet emplacement. Pour la
réutiliser dans le desktop, définir `VERTEX_DB_DIR` sur ce dossier avant de
lancer l'application ; aucun déplacement de plusieurs gigaoctets n'est automatique.

Contrôles de qualité : `just test`, `just lint`, `just audit` et `just fmt`.
`just smoke` vérifie l'API sur une base temporaire (Node.js 24 ou plus récent).
Le workflow `.github/workflows/ci.yml` vérifie le workspace et le desktop sous
Windows. Le dernier audit détaillé est disponible dans [AUDIT.md](AUDIT.md).

## Version web publique

Site de presentation : **https://vertex-ded-sec.vercel.app**.
Le site statique est publie sur Vercel, sans base, panneau prive ni installateur.
L'index partage n'est pas encore ouvert ; aucun abonnement payant n'a ete ajoute.

Voir [PUBLICATION.md](PUBLICATION.md) pour le perimetre verifie, les limites et
la configuration HTTPS. Le mode local reste le comportement par defaut.
Le mode public exige `VERTEX_PUBLIC_ORIGIN` (HTTPS) et `VERTEX_ADMIN_TOKEN`
(secret aleatoire de 32 caracteres minimum). Il ne publie ni les signets du
serveur ni les messages de contact. Les signets visiteurs restent dans leur
navigateur. Utiliser une base dediee, jamais l'index personnel existant.

Rust 1.89 minimum, Node.js 24 minimum pour les tests web. Apres `npm ci` et
`npx playwright install chromium`, `npm run test:web` execute les parcours
sur trois tailles d'ecran ; `npm run test:api` teste les deux modes HTTP.

## Communauté (`apps/hub-rs`)

**vx-hub** : chat en direct, forums (catégories → sujets → réponses), avis sur
le logiciel (ouverts avec les premiers utilisateurs) et partenaires (publicité
de serveur Discord, payante et modérée).

```sh
# démarrer le hub sur le port 8787 (admin facultatif)
cargo run -p hub-rs -- 8787
VX_ADMIN_USER=monadmin VX_ADMIN_PASS=mot-de-passe-suffisamment-long cargo run -p hub-rs -- 8787
```

Serveur HTTP minimal avec limites de lecture et délais, stockage
`database/hub.db` (séparé de l'index). L'aléa, PBKDF2 et la comparaison des
empreintes utilisent des bibliothèques maintenues. Le hub écoute sur
une adresse loopback uniquement ; `VX_HUB_HOST` ne peut pas l'exposer sur Internet.
Le hub communautaire n'est pas inclus dans la version web publique. Il exige
une migration HTTP, une protection anti-abus et une gestion HTTPS des sessions
avant toute mise en ligne.

## Supervision locale

`scripts/panel.ps1` ouvre le serveur du panneau prive sur le port 3108,
avec la vraie base `database/vertex.db`. Le panneau est disponible sur `/panel`.
Le traitement de la file se pilote avec `scripts/crawl-index.ps1`.
Voir [SUPERVISION.md](SUPERVISION.md) pour les mesures, l'audit des millions
d'URL et les limites de verification et de stockage.

## Confidentialité

L'index et les signets de Vertex restent locaux. Le crawler et les proxys
contactent les sites demandés : ceux-ci voient l'adresse IP de la machine.
Les erreurs et résumés de crawl sont conservés dans SQLite ; aucun service de
télémétrie n'est intégré. Voir **[POLICY.md](POLICY.md)**.

## Licence
MIT — voir `LICENSE`.
