# Changelog

Toutes les ÃƒÆ’Ã‚Â©volutions notables du projet Vertex sont documentÃƒÆ’Ã‚Â©es ici.

## [0.14.0] ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â 2026-08-31

### Recherche
- **SQLite FTS5** : recherche **plein texte** avec classement par pertinence
  (BM25), au lieu du `LIKE`. Table virtuelle `pages_fts` (contenu externe)
  maintenue automatiquement par des triggers.
- **Snippets** positionnÃƒÆ’Ã‚Â©s autour du terme (via `snippet()` FTS5).
- **Pagination** : paramÃƒÆ’Ã‚Â¨tre `offset` sur `/api/search`.

### Contenu
- Nettoyage renforcÃƒÆ’Ã‚Â© : suppression aussi de `video`, `audio`, `object`,
  `embed`, `canvas`, `picture`, `map` (en plus de la navigation, des scripts
  et des mÃƒÆ’Ã‚Â©tadonnÃƒÆ’Ã‚Â©es dÃƒÆ’Ã‚Â©jÃƒÆ’Ã‚Â  retirÃƒÆ’Ã‚Â©s).

### Interface web
- **Highlight** des termes recherchÃƒÆ’Ã‚Â©s dans les titres et extraits.
- Bouton **Ãƒâ€šÃ‚Â« Charger plus Ãƒâ€šÃ‚Â»** (pagination).
- Affichage du **nombre de rÃƒÆ’Ã‚Â©sultats** et du lien de chaque rÃƒÆ’Ã‚Â©sultat.
- Design revu (en-tÃƒÆ’Ã‚Âªte dÃƒÆ’Ã‚Â©gradÃƒÆ’Ã‚Â©, cartes, ÃƒÆ’Ã‚Â©tat vide).

## [0.13.0] ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â 2026-08-31

### Crawler web (`crawler-rs`)
- **Crawl multi-sites** : plusieurs seeds et **suivi des liens externes**
  (on ne reste plus limitÃƒÆ’Ã‚Â© ÃƒÆ’Ã‚Â  un seul domaine).
- **Limite de durÃƒÆ’Ã‚Â©e** : le crawl s'arrÃƒÆ’Ã‚Âªte aprÃƒÆ’Ã‚Â¨s **300 secondes** (au lieu
  d'un plafond de pages), pour explorer un maximum de sites.
- **Crawler multi-threads** : 6 workers partagent la file SQLite, pour un
  dÃƒÆ’Ã‚Â©bit nettement supÃƒÆ’Ã‚Â©rieur.
- **Contenu nettoyÃƒÆ’Ã‚Â©** : l'extraction retire les **mÃƒÆ’Ã‚Â©tadonnÃƒÆ’Ã‚Â©es** (`head`,
  `meta`, `title`), la **navigation** (`nav`, `header`, `footer`, `aside`),
  les **scripts / analytics** (`script`, `style`, `noscript`), les
  **formulaires de cookies / consentement** (`form`), les `iframe`,
  `svg`, `template`ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â¦ puis **compresse les espaces**. Le site affiche donc
  uniquement le texte utile.
- **Statistiques** en fin de crawl : pages traitÃƒÆ’Ã‚Â©es, ÃƒÆ’Ã‚Â©checs, sites
  (domaines) distincts, URLs restant en file.
- Stockage du **contenu textuel** des pages (colonne `content`).

### Serveur web (`api-rs`)
- Serveur HTTP **axum** (tout Rust, un seul binaire) :
  - `GET /` : interface de recherche (HTML/JS) embarquÃƒÆ’Ã‚Â©e.
  - `GET /api/search?q=ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â¦&offset=ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â¦` : rÃƒÆ’Ã‚Â©sultats (JSON).
  - `GET /api/page?url=ÃƒÂ¢Ã¢â€šÂ¬Ã‚Â¦` : contenu complet d'une page (JSON).
  - `GET /health` : ÃƒÆ’Ã‚Â©tat du serveur.

### Correctifs prÃƒÆ’Ã‚Â©cÃƒÆ’Ã‚Â©dents
- Suppression du code mort (`main_v012_backup.rs`, `queue.rs`, `visited.rs`)
  et des dÃƒÆ’Ã‚Â©pendances inutilisÃƒÆ’Ã‚Â©es.
- Correction du doublon d'enregistrement des erreurs.
- Chemin de base rÃƒÆ’Ã‚Â©solu depuis le manifest (indÃƒÆ’Ã‚Â©pendant du rÃƒÆ’Ã‚Â©pertoire courant).
- `resolver = "3"` pour l'ÃƒÆ’Ã‚Â©dition 2024.
- Fichiers `README`, `LICENSE`, `.gitignore` renseignÃƒÆ’Ã‚Â©s.
### Build release et installeurs
- **`cargo tauri build` opérationnel** (`tauri-cli v2.11.4` installé).
- Produits (target/release/bundle) :
  - `Vertex_0.1.0_x64-setup.exe` (5,6 Mo, installeur NSIS.
  - `Vertex_0.1.0_x64_en-US.msi` (8,3 Mo.
  - `desktop.exe` (24,6 Mo, exécutable natif release.
- Le premier run a échoué sur `windows 0.61.3` (`opt-level=3`, crash du compilateur
  → contourné par `[profile.release]` : `opt-level =  ​2`, `codegen-units =  ​16`.