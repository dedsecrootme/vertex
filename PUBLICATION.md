# Publication du site - 20 octobre 2026

Le site de presentation est publie sur **https://vertex-ded-sec.vercel.app**.
Le deploiement contient uniquement `apps/landing/public`, avec les exclusions
de `.vercelignore`. Aucun index, signet, message, secret, panneau prive ou ancien
installateur n'est publie. La recherche partagee et le telechargement ne sont
pas ouverts : leur page indique cette limite explicitement.

La production est publique ; la protection Vercel est conservee pour les
previsualisations. Aucun serveur, stockage ou abonnement n'a ete achete.
Le projet Vercel se nomme `vertex` dans l'equipe deja connectee.

Pour republier depuis `apps/landing`, avec le compte proprietaire deja connecte :

```powershell
vercel deploy --prod --non-interactive
vercel alias set URL_DU_NOUVEAU_DEPLOIEMENT vertex-ded-sec.vercel.app
```

Reverifier ensuite l'URL publique, sans connexion Vercel :

```powershell
node scripts/verify-public-site.mjs https://vertex-ded-sec.vercel.app
```

Le panneau prive est accessible par le raccourci de bureau
`Vertex - Supervision`. Il lance l'API locale si necessaire, sans exposer le PC.
Les commandes de supervision Start/Stop sont exclusivement locales et bornees.

## Verification historique du 17 octobre

Mise a jour du 18 octobre : architecture Cloudflare, partage d'index, crawl
continu et consentement PC prepares dans `CLOUDFLARE.md`. Les resultats et
instructions historiques ci-dessous ne constituent pas un deploiement de cette
nouvelle architecture. Aucun serveur achete, aucun installateur publie.

## Verdict et perimetre

Le code du moteur web dispose d'un mode public distinct du mode personnel.
La recherche, la lecture assainie, les signets navigateur et le contact sont
verifies sur des bases temporaires. Aucune base existante n'a ete migree ou
reconstruite par les essais. Aucun deploiement public n'a ete effectue.

Ce n'est pas une attestation de securite absolue ni un test de charge Internet.
L'ouverture effective reste conditionnee aux controles d'exploitation ci-dessous.
Le hub communautaire et les anciens installateurs ne font pas partie de cette
validation. Le desktop est compile et controle sous Windows, pas teste comme
application installee. La visionneuse est un lecteur sans scripts, pas un
navigateur generaliste executant les applications JavaScript des sites tiers.

## Corrections de cette verification

- Mode public explicite : origine HTTPS valide et jeton administrateur aleatoire
  de 32 caracteres minimum obligatoires ; configuration invalide refusee.
  VERTEX_DB_DIR doit etre fourni explicitement pour eviter le partage par defaut
  de la base personnelle du workspace.
- Commandes de crawl, signets serveur et boite de contact authentifies. Le jeton
  reste cote exploitant ; aucune saisie ni sauvegarde de ce secret dans le site.
- Statistiques publiques et resultats sans divulgation des signets personnels.
  Signets visiteurs limites a 500 et stockes dans leur propre navigateur.
- Verification de Host/Origin, rejet des appels API cross-site ; navigation
  publique depuis un autre site permise. Aucune confiance dans X-Forwarded-For.
- Proxy limite aux medias deja indexes, DNS et redirections publics verifies,
  types raster seulement, corps et durees bornes. 24 images maximum chargees
  automatiquement par lecture.
- 600 requetes/minute et 5 soumissions contact/minute au total par instance,
  32 requetes actives, delai de traitement HTTP de 25 secondes, corps de 16 Kio,
  champs bornes et boite publique limitee a 10 000 messages.
- Requetes SQLite executees hors des threads async, erreurs internes non
  divulguees, CSP sans JavaScript inline, no-store, nosniff, permissions et HSTS.
- Interface : erreurs visibles, courses entre recherches ignorees, bon retrait
  des signets apres rechargement, liens non HTTP(S) refuses, gestion du focus,
  fermeture clavier, onglets accessibles et contenus longs sans debordement.
- Crawler : verrou de fichier interprocessus, reprise sans voler une session
  vivante ; robots en cache avec initialisation unique par origine ; verification
  et politesse pour chaque redirection et les fichiers annexes.
- 429/503 : Retry-After en secondes ou date HTTP, reprise persistante planifiee,
  maximum de trois tentatives. Un Retry-After depassant la session reste en base
  pour une execution suivante. Les autres echecs reseau restent terminaux ; une
  remise en file manuelle reinitialise leur compteur.
- Valeurs par defaut prudentes : 4 workers, 1 seconde par domaine, 10 000 pages
  par session ; au plus 32 workers, 24 heures et profondeur 20 meme si l'appelant
  demande des valeurs excessives. Parsing borne a 2 000 liens et 500 images.
- Six avis sur les anciennes versions verrouillees resolus par Ammonia 4.2.4
  et rustls 0.23.46 ; changements dans les deux Cargo.lock.
- Page de presentation : capture reelle, texte corrige, anciens telechargements
  retires de l'offre, redirection des anciennes URL apres deploiement Vercel,
  correction de l'en-tete qui nommait les MSI et checksums comme des EXE.
- Hub : ecoute non-loopback refusee, Host/Origin controles, liens Discord
  verifies par un parseur d'URL et taille des mots de passe bornee.

## Verifications reproductibles

Resultat local : 67 tests Rust, 18 tests navigateur et les deux suites HTTP
passent. Clippy strict passe sur le workspace et le desktop Windows.
Les captures ont ete inspectees sur ordinateur et mobile.

```powershell
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml -- --check
cargo audit
cargo audit --file apps/desktop/src-tauri/Cargo.lock
cargo build -p api-rs --locked
npm ci
npx playwright install chromium
npm run test:api
npm run test:web
```

Les tests HTTP et navigateur creent leurs propres index temporaires. Les tests
navigateur couvrent la presentation, la recherche, le lecteur piege, le focus,
les signets, le contact, les refus API, les erreurs et les contenus longs a
1280, 390 et 320 px. Le proxy du test simule la terminaison TLS mais ne prouve
pas le fonctionnement d'un vrai certificat ou du domaine de production.
Sous Windows, `PLAYWRIGHT_CHANNEL=chrome` permet de tester avec Chrome installe.

Audit Rust du jour : aucune entree de vulnerabilite dans les deux verrous.
Le workspace principal n'a aucun avertissement. Le desktop conserve cinq avis
transitifs : glib (unsound), proc-macro-error, zbus_names, zvariant et
zvariant_derive (non maintenus). Ne pas les masquer. zvariant est absent de
l'arbre Windows verifie ; pas de validation de distribution Linux.

## Configuration publique

`.env.example` documente les variables, mais n'est pas charge automatiquement.
Dans PowerShell, avec des valeurs propres a votre installation :

```powershell
$env:VERTEX_DB_DIR = 'C:\Vertex\public-data'
$env:VERTEX_PORT = '3000'
$env:VERTEX_PUBLIC_ORIGIN = 'https://search.votre-domaine.fr'
$env:VERTEX_HOST = 'search.votre-domaine.fr'
$env:VERTEX_ADMIN_TOKEN = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32))
cargo run -p api-rs --release --locked
```

Conserver le jeton dans un gestionnaire de secrets, pas dans Git ou une commande
partagee. L'exemple le genere en memoire et ne l'affiche pas. L'API reste sur
127.0.0.1. Dans un second terminal configure avec le meme VERTEX_HOST :

```powershell
caddy validate --config deploy/Caddyfile --adapter caddyfile
caddy run --config deploy/Caddyfile --adapter caddyfile
```

`deploy/Caddyfile` preserve Host et ne route pas vers le hub. Seuls les ports
80/443 de Caddy doivent etre ouverts ; ne pas ouvrir les ports 3000/8787.
Il faut d'abord installer Caddy et configurer le DNS du domaine. Ces operations
et l'obtention d'un certificat public n'ont pas ete effectuees pendant cet audit.

Administration depuis un terminal de confiance :

```powershell
$headers = @{ Authorization = "Bearer $env:VERTEX_ADMIN_TOKEN" }
Invoke-RestMethod "$env:VERTEX_PUBLIC_ORIGIN/api/contact" -Headers $headers
Invoke-RestMethod "$env:VERTEX_PUBLIC_ORIGIN/api/crawl" -Method Post -Headers $headers -ContentType 'application/json' -Body '{"duration":300}'
```

## Conditions avant ouverture reelle

1. Choisir un domaine et un hebergement, installer/valider Caddy, verifier le
   certificat, la redirection HTTP vers HTTPS et les controles via l'URL finale.
2. Creer un index public dedie de contenus autorises. Ne jamais reutiliser votre
   index personnel ni la base communautaire comme jeu de publication.
3. Ajouter une protection par client au reverse proxy/WAF. Les budgets applicatifs
   sont globaux : un client peut les consommer et penaliser les autres. La limite
   de requetes seule n'est pas une protection DDoS ou un quota d'octets sortants.
4. Tester la charge cible, surveiller erreurs/latences/espace disque, definir
   sauvegarde SQLite coherente et restauration testee, retention des contacts,
   rotation du jeton et procedure de retour arriere. Le controle FTS a l'ouverture
   peut etre long sur un gros index ; tester le demarrage avec le volume reel.
5. Renseigner l'identite et le contact de l'exploitant, l'information de
   confidentialite et la procedure de suppression des messages. Le formulaire
   enregistre en base : il n'envoie pas d'email et le mode local reste local.
6. Avant une distribution Windows, produire un nouvel installateur, verifier
   installation/desinstallation et conservation des donnees, signer le binaire
   et publier ses checksums. Ne pas reutiliser les anciens binaires 0.1.0.

## References

- [RFC 9309](https://www.rfc-editor.org/rfc/rfc9309.html)
- [OWASP REST Security](https://cheatsheetseries.owasp.org/cheatsheets/REST_Security_Cheat_Sheet.html)
- [Configuration Playwright](https://playwright.dev/docs/test-configuration)
- [Reverse proxy Caddy](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy)
