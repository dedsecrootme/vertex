# Controle web du 19 octobre 2026

## Perimetre et limites

Controle du site vitrine, du Worker de distribution, des parcours de recherche
et de supervision deja testes, et des verrous npm/Rust. Il ne s'agit pas d'un
pentest exhaustif ni d'une garantie d'absence de failles. Aucun serveur public
n'a ete provisionne ou teste. Les binaires en cours d'execution n'ont pas ete
remplaces ; le crawler et sa base existante restent en fonctionnement.

## Resultats verifies

- `npm audit --json` apres installation : zero avis de vulnerabilite signale.
  Versions exactes Motion 14.3.0 et esbuild 0.28.2, verrou npm conserve.
- `cargo audit --json` : 269 dependances, zero vulnerabilite signalee, zero
  avertissement dans le workspace principal. Base RustSec du 17 octobre 2026.
- Verrou desktop : 550 dependances, zero vulnerabilite classee comme telle par
  RustSec, mais cinq avertissements non ignores, detailles ci-dessous.
- 54 tests navigateur : recherche, contenu hostile assaini, refus API,
  supervision authentifiee, galerie, clavier et mise en page sur trois formats.
- 8 tests Worker : manifeste strict, reprise partielle, refus des ecritures,
  redirections imposees par la configuration, remplacement concurrent R2,
  taille du manifeste et en-tetes de securite, y compris sur erreurs.
- Controle du vrai site d'apercu sur cinq formats de 320 a 1920 px : images
  chargees, pas de debordement, pas d'erreur JavaScript, pas de violation CSP,
  pas de requete a un domaine tiers. Assets Motion servis par Vertex.
- Controle des textes des deux interfaces publiques et conservation des
  licences tierces obligatoires. Les notes de conception privees ne sont
  pas incluses dans la publication.
- Ajout au serveur d'apercu d'un controle Host/Origin contre les acces par
  nom de domaine externe ; traitement des URL de requete mal formees sans
  arret du processus. Le serveur reste lie a 127.0.0.1, pas a Internet.

## A traiter avant publication

### SQLite embarque

`libsqlite3-sys` 0.28.0 embarque SQLite 3.45.0, confirme dans sqlite3.h.
Un resultat RustSec vide ne couvre pas exhaustivement le code C embarque.
La liste officielle SQLite comporte notamment des correctifs ulterieurs pour
CVE-2025-6965 et CVE-2025-7709. Leurs preconditions sont respectivement du SQL
arbitraire ou le controle du contenu de la base ; il ne faut pas deduire une
exploitation distante simplement parce que la version est ancienne. Les
recherches et identifiants utilisateurs inspectes sont passes comme parametres,
et le site ne permet pas de televerser une base. Cela ne remplace pas une mise
a niveau du moteur et un essai de compatibilite FTS5 sur copie coherente de
l'index avant remplacement des processus vivants. Aucune correction de ces
CVE n'est revendiquee par cette modification graphique.

Reference : https://www.sqlite.org/cves.html

### Desktop et Linux

Avertissements conserves sans exclusion :

- glib 0.18.5 : RUSTSEC-2024-0429, comportement memoire incorrect dans
  VariantStrIter, corrige a partir de glib 0.20.0.
- proc-macro-error 1.0.4 : RUSTSEC-2024-0370, non maintenu.
- zbus_names 4.3.4 : RUSTSEC-2026-0371, non maintenu.
- zvariant 5.15.0 : RUSTSEC-2026-0370, non maintenu.
- zvariant_derive 5.15.0 : RUSTSEC-2026-0372, non maintenu.

Ces dependances transitives demandent une validation par plateforme et une
migration des dependances amont, pas un changement aveugle du verrou. Les
alertes de maintenance ne sont pas toutes des CVE exploitables. Le bureau
Linux n'est pas certifie par ces essais navigateur Windows.

Reference : https://rustsec.org/advisories/RUSTSEC-2024-0429.html

### Exploitation publique

Restent a valider sur l'hebergement reel : TLS, configuration Cloudflare/Caddy,
secret administrateur neuf, limites par client au proxy, sauvegardes externes
et restauration, surveillance, installateur signe et verification du bundle
installe. Le dry-run Cloudflare ne publie rien et ne prouve pas ces points.

## Reproduction

`npm run build:icons` regenere les icones, le module Motion auto-heberge et
les notices de tous les composants embarques. `npm run build:motion` ne
reconstruit que Motion et les notices du site vitrine.

`npm run test:web`, `npm run test:cloudflare`, `npm run test:api`,
`npm audit --json`, `cargo audit --json`,
`cargo audit --file apps/desktop/src-tauri/Cargo.lock --json`.

Pour les captures privees de verification : `node scripts/check-premium.mjs`,
avec le serveur d'apercu deja actif sur 3110. Les images produites restent dans
test-results, ne sont pas publiees automatiquement et ne remplacent pas les
captures publiques deja verifiees.
