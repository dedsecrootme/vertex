# Vertex : serveur continu et distribution Cloudflare

Etat au 18 octobre 2026 : implementation preparee, pas de migration de production.
Le compte Cloudflare est accessible via Wrangler. Le bucket existant `uploads`
a ete identifie sans etre modifie. Aucun serveur n'a ete achete, aucun bucket
n'a ete cree, aucun installateur n'a ete envoye. Le partage PC reste desactive.

## Architecture

- Site et distribution : Cloudflare Workers + assets statiques.
- Installateurs : bucket R2 prive, expose uniquement via `/downloads/windows`.
- Recherche : API Rust persistante, SQLite FTS5 et volume dedie sur serveur.
- Crawl central : processus Rust continu, independant des visiteurs du site.
- Application Windows : recherche dans l'index partage configure, puis index
  local si aucun resultat ; signets toujours conserves localement.
- Les donnees de recherche ne vont pas dans R2. R2 sert les fichiers, pas les
  connexions SQLite ni un processus de crawl permanent.

```text
Site Cloudflare -- /api/release et /downloads/windows --> R2 prive
Application et navigateur -- HTTPS --> API Rust --> SQLite FTS5
Crawler central -------------------------------> file persistante
PC volontaire -- proposition d'URL consentie --> API, puis verification serveur
```

## Recherche et donnees communes

`VERTEX_SHARED_API` designe une origine HTTPS publique, sans chemin ni secret.
Elle peut etre fixee avant compilation du desktop ou dans son environnement
d'execution. Aucun jeton administrateur n'est inclus dans l'application.
Les redirections du client d'index sont refusees, pour ne pas transmettre les
requetes ou propositions a une autre destination.

Une recherche lit l'index sans demarrer de nouveau crawler. Une URL publique
absente peut etre proposee via `POST /api/discover` si le serveur active
`VERTEX_ALLOW_DISCOVERY=1`. Les URL sont normalisees et dedupliquees. Une page
indexee depuis moins de 24 heures donne `ready`, sans remise en file.
Les mots-cles seuls ne permettent pas de decouvrir instantanement des sites
inconnus : les nouvelles pages viennent de l'exploration continue et des URL
proposees, pas d'un fournisseur de recherche externe cache.

La file est plafonnee et le budget de propositions est global : 30/minute.
Avant ouverture a une audience importante, ajouter une limitation equitable
par client au frontal, des quotas d'exploitation et une moderation des domaines.

## Crawl continu

```powershell
$env:VERTEX_DB_DIR = 'C:\Vertex\public-data'
cargo run -p crawler-rs -- --continuous
```

Le processus garde un verrou exclusif pendant son execution. Il conserve les
delais par domaine entre sessions, recharge les robots chaque heure et remet
en file les pages obsoletes. Une file vide attend au lieu de boucler activement.
Les echecs reseau ont des tentatives bornees et un delai avant reprise.
Les robots, destinations privees, limites de taille et Retry-After restent verifies.
Ctrl+C demande un arret propre ; SIGTERM est aussi traite sur Linux.

Le quota central par defaut est de 8 Gio pour les fichiers SQLite et WAL.
Il est controle entre sessions : c'est une limite souple, pas un quota disque.
Le superviseur Docker assure la reprise apres un arret du processus ou du serveur.
Une panne de courant, une coupure reseau ou un volume plein ne peuvent pas etre
elimines par une promesse de fonctionnement "sans jamais s'arreter".

## Participation PC

Trois reglages explicites dans l'application : exploration sur ce PC, poursuite
apres fermeture, proposition d'URL publiques. Tous sont desactives initialement.
Le crawl PC utilise un worker, un delai minimal de 3 secondes par domaine,
50 pages au maximum par session et un quota souple de 512 Mio.
Fermer la fenetre arrete le worker, sauf autorisation de poursuite. Il n'y a
pas de tache planifiee, de service Windows ni de demarrage automatique au login.
Rouvrir l'application permet de suspendre le processus. L'arret d'une requete
en cours reste borne par son timeout.

Le partage supplementaire doit afficher et enregistrer la destination HTTPS
acceptee. Seules les nouvelles URL explorees apres consentement sont proposees,
sans query string. Aucun HTML, signet ou historique anterieur n'est envoye.
Le serveur crawle lui-meme ses copies : il ne fait pas confiance au contenu ou
au classement annonce par un client modifie.

Limite importante : cette version propose des decouvertes, mais n'attribue pas
encore des travaux exclusifs aux PC. Un vrai reseau distribue optimisant les
requetes de tous les participants demande encore identites de noeuds, baux de
travail, quotas globaux par site et controle d'abus. Ne pas le presenter comme
un maillage distribue finalise.

## Serveur a deployer plus tard

`deploy/compose.yaml` prepare API, crawler et terminaison HTTPS Caddy, avec
volume persistant, processus non root pour Rust, limites memoire et journaux
bornes. Seul Caddy expose des ports publics. Le conteneur API refuse l'ecoute
sur toutes les interfaces sans configuration publique authentifiee.

Fournir un domaine d'API dans `VERTEX_HOST` et un secret aleatoire d'au moins
32 caracteres dans `VERTEX_ADMIN_TOKEN`, hors Git. Ne jamais reutiliser un
secret d'exemple. Configurer DNS, pare-feu, sauvegarde chiffree et restauration.

```sh
docker compose --env-file .env -f deploy/compose.yaml config --quiet
docker compose --env-file .env -f deploy/compose.yaml up -d --build
```

La syntaxe Compose a ete validee avec des valeurs fictives. Le moteur Docker
local n'est pas demarre : construction de l'image Linux et fonctionnement des
conteneurs restent a verifier sur le serveur cible avant publication.

## Site et telechargements

`apps/landing/wrangler.jsonc` reference le futur bucket `vertex-downloads`.
Ce nom doit etre confirme ou remplace avant deployment. `VERTEX_APP_URL`
recevra l'origine du serveur de recherche reel. Sans configuration, `/search`
repond explicitement indisponible ; il ne simule pas une recherche operationnelle.

```sh
npm ci
npm run test:cloudflare
npm run cloudflare:check
# Apres validation du compte, bucket et domaine seulement :
npx wrangler deploy --config apps/landing/wrangler.jsonc
```

Le Worker valide un manifeste versionne et la taille de l'objet. Il sert des
flux R2 avec reprise par plages, HEAD, ETag et SHA-256 publie. Les objets sont
adresses par version et hash pour ne pas remplacer silencieusement un binaire
reference par un ancien manifeste. Aucune route d'upload, liste de bucket ou
archive de sources n'est exposee. Les anciens installateurs sont exclus des assets.

Le script `scripts/publish-release.ps1` exige une signature Authenticode valide
du distributeur attendu et une version correspondant a `tauri.conf.json`.
Il envoie le fichier puis seulement le manifeste. Il n'a pas ete execute.
Un certificat de signature et le build de diffusion sont encore necessaires.

## Sources et droits

Les liens vers les sources et le texte public "open source" ont ete retires
du site de distribution. Aucune attribution de fabrication demandee n'y figure.
Le fichier LICENSE du depot et les notices des dependances sont conserves.
Cela ne rend pas un depot public prive et ne modifie pas retroactivement les
droits des anciennes versions. La politique de licence des futures versions
et la visibilite du depot restent des decisions explicites du proprietaire.

## Conditions avant ouverture

- Domaine, serveur et volume persistant operationnels, avec sauvegarde restauree.
- Build Windows signe, installe/desinstalle sur un PC propre ; consentement,
  fermeture, suspension et redemarrage du worker verifies en situation reelle.
- Tests HTTPS publics, protections par client, moderation et monitoring actif.
- Identite de l'exploitant, retention et suppression des messages documentees.
- Choix de licence/distribution clarifie, sans retirer les notices de composants.

Ne pas qualifier cette preparation de service public deja deploye ou de
garantie de disponibilite a 100 %.

## Verification effectuee

- 73 tests Rust du workspace et 2 tests desktop : reussis.
- 24 parcours Playwright : reussis sur 1280, 390 et 320 pixels.
- 5 tests Worker/R2 simule : reussis ; reprise de telechargement, HEAD,
  manifeste invalide, absence de release et refus des routes non autorisees.
- Tests API locale et publique : reussis, avec bases temporaires dediees.
- Clippy strict workspace et desktop : reussi ; formatage verifie.
- Audit RustSec : aucune vulnerabilite signalee dans les fichiers verrouilles.
  Cinq avertissements transitifs desktop restent visibles : glib 0.18.5
  (RUSTSEC-2024-0429, unsound), proc-macro-error 1.0.4 et zbus_names,
  zvariant, zvariant_derive (non maintenus). Aucun avis n'a ete masque.
- Paquet Cloudflare valide en `--dry-run`, syntaxe Compose et script de
  publication valides. Ni publication ni execution du script d'upload.
- Capture de l'interface renouvelee ; aucune base personnelle existante utilisee.

Les tests simules ne remplacent pas l'installation Windows signee ni les essais
du futur hebergement Linux, du HTTPS public et de la sauvegarde/restauration.

References : [API R2](https://developers.cloudflare.com/r2/api/workers/workers-api-reference/),
[assets Workers](https://developers.cloudflare.com/workers/static-assets/binding/).
