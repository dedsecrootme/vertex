# Audit Vertex - 5 octobre 2026

Dernier point de controle : [20 octobre 2026](AUDIT-2026-10-20.md).

Rapport historique. Pour les corrections et verifications du 17 octobre 2026,
voir [PUBLICATION.md](PUBLICATION.md). Les versions et avertissements ci-dessous
decrivent l'etat du 5 octobre, pas le verdict actuel de publication.

Audit du code local, des deux Cargo.lock, du navigateur et des chemins de stockage.
La base de production existante n'a pas ete modifiee pour les essais.
Les essais HTTP et navigateur utilisent exclusivement des bases temporaires.

## Problemes confirmes et corriges

| Priorite | Probleme constate | Correction |
|---|---|---|
| Haute | Les proxys acceptaient les adresses locales ; IPv6 et redirections pouvaient contourner la validation du crawler | Politique partagee dans `network.rs` : HTTP(S) public, identifiants refuses, DNS valide utilise par le connecteur, verification des redirections, IPv4/IPv6 reserves bloques |
| Haute | Le nettoyage HTML reposait sur des expressions regulieres, laissant des variantes actives possibles | Ammonia avec liste de balises autorisees ; pas de styles ni attributs executables ; images inertes avant proxy ; anciennes pages nettoyees a la lecture |
| Haute | Le desktop passait les URL a `cmd /c start` | Ouverture avec le plugin Tauri opener et validation de l'URL |
| Haute | h2 0.4.15 et rustls 0.23.43 etaient vulnerables | h2 0.4.19 et rustls 0.23.45 dans le verrou du workspace |
| Haute | Le coffre signalait son expiration sans refuser les secrets | Expiration verifiee a l'acces, destruction de la cle et effacement des buffers possedes avec zeroize ; bornes sur le cout PBKDF2 |
| Haute | La source d'alea du hub pouvait produire des zeros hors Windows | getrandom sur toutes les plateformes prises en charge ; PBKDF2 et comparaison constante confies a des bibliotheques maintenues |
| Moyenne | Les corps HTTP etaient integralement lus avant le controle de taille | Lectures bornees pendant le telechargement, meme sans Content-Length |
| Moyenne | Une regle Allow courte pouvait gagner contre un Disallow plus long | Regle du plus long motif, fusion des groupes correspondants, chemins encodes et Request-rate pris en compte |
| Moyenne | COUNT(*) sur la table FTS externe masquait un index vide | integrity-check avec verification du contenu externe et reconstruction en cas de desynchronisation |
| Moyenne | Une erreur lors de l'enregistrement des images pouvait laisser une page partiellement mise a jour | Transaction pour page, liens, images et declencheurs FTS |
| Moyenne | La reservation des URL n'etait atomique qu'a travers un verrou de processus | UPDATE ... RETURNING reserve l'URL dans SQLite |
| Moyenne | Les compteurs du crawl incluaient l'historique et le plafond de pages pouvait etre depasse | Compteurs de session, budget reserve avant travail, arret quand la file est vide, attente borne par la duree de session |
| Moyenne | Les erreurs de stockage des workers etaient ignorees | Erreurs remontees dans CrawlSummary ; execution marquee en echec et sortie CLI non nulle |
| Moyenne | Plusieurs clics sur Crawler lancaient des moteurs simultanes | Verrou de lancement dans API et desktop, statut interroge par l'interface, bouton desactive durant le crawl |
| Moyenne | Le desktop utilisait un chemin grave lors de la compilation | Dossier utilisateur Tauri et surcharge explicite VERTEX_DB_DIR |
| Moyenne | L'API locale n'imposait pas d'origine ni de Host local | Verification des en-tetes et protections CSP, nosniff et no-referrer |
| Moyenne | Le hub lisait des en-tetes non bornes et tronquait silencieusement les gros corps | Limites de lignes/en-tetes/corps, refus des requetes ambigues, delais et plafond de 64 connexions actives |
| Moyenne | Le hub ecoutait sur toutes les interfaces par defaut | Loopback par defaut ; VX_HUB_HOST pour une exposition explicite |
| Faible | Le parseur HTML utilisait fxhash abandonne | scraper 0.27 retire cette dependance des deux verrous |
| Faible | Le decodage base64 acceptait des donnees invalides | Bibliotheque base64 avec verification du padding et des bits inutilises |
| Faible | Pagination avec conversion susceptible de deborder, titres historiques NULL | Conversion bornee et COALESCE dans les resultats |
| Faible | Doublon du schema FTS dans indexer-rs | L'indexer reutilise l'initialisation et le controle communs |
| Faible | Aucun controle continu, fmt et clippy en echec, dependance desktop inutilisee | Workflow Windows, commandes just, formatage, lints corriges, serde_json inutilise retire |

Les nouvelles empreintes de mots de passe et les nouveaux coffres utilisent
600 000 iterations PBKDF2-HMAC-SHA256. Les anciennes valeurs restent lisibles.

## A garder

- Rust, Cargo.lock versionnes et le workspace : base technique coherente.
- SQLite WAL et FTS5/BM25 : adaptes a un index personnel sans serveur externe.
- Modules distincts pour parsing, stockage, crawl et recherche.
- Le chemin explicite de base et les signets/contact separes des pages.
- Le User-Agent identifiable, robots.txt, les limites et la politesse par domaine.
- La cryptographie CNG du coffre avec les vecteurs NIST deja presents.
- Le frontend partage entre navigateur et desktop, avec tests des parcours reels.

## Travail restant, par ordre de priorite

1. **Coordination entre programmes.** Le verrou de lancement protege chaque API
   ou desktop. Il reste a empecher une CLI, une API et un desktop distincts de
   lancer des crawls sur la meme base. reset_crawling peut reprendre le travail
   d'un autre processus vivant ; utiliser un verrou de session avec proprietaire.
2. **Reprise et politesse completes.** Ajouter backoff/Retry-After, nombre maximal
   de tentatives et reprise planifiee pour 429/503. Le cache robots peut effectuer
   plusieurs telechargements au premier acces concurrent ; les requetes annexes
   et les destinations redirigees doivent partager la politesse et les regles
   robots avant chaque nouvelle requete. La politique ne revendique plus une
   conformite RFC 9309 complete.
3. **Hub public.** Ajouter limitation des inscriptions/connexions par client,
   HTTPS et gestion du cookie Secure derriere un proxy de confiance. Le serveur
   HTTP artisanal conserve une surface de maintenance importante ; preferer axum,
   deja utilise par l'API, lors d'une evolution du service.
4. **Alertes Linux du desktop.** Le verrou conserve glib 0.18.5 (RUSTSEC-2024-0429,
   unsound) et proc-macro-error 1.0.4 (RUSTSEC-2024-0370, non maintenu), via GTK.
   glib n'apparait pas dans l'arbre Windows x86_64 verifie. Suivre la migration
   GTK/Tauri en amont avant de proposer une version Linux ; ne pas masquer les
   alertes par une exclusion generale.
5. **Echelle du stockage.** La base actuelle et son WAL representent plusieurs
   gigaoctets. Prevoir sauvegarde SQLite coherente, retention, limites de file,
   maintenance FTS planifiee et requetes hors des threads async de l'API.
   Le controle FTS integral a l'ouverture peut etre couteux a cette taille.
6. **Coffre et secret en memoire.** L'expiration est appliquee a l'acces et au
   drop, pas par un minuteur autonome. Un appelant ne doit pas garder une
   reference de secret indefiniment. Le chiffrement du coffre ne chiffre pas
   l'index, les signets ou la base communautaire.
7. **Modules incomplets.** vx-mail contient des helpers base64/quoted-printable ;
   multipart, composition, IMAP/SMTP et transport TLS ne sont pas implementes.
   vx-sys est Windows uniquement. Ces limites sont maintenant indiquees.
8. **Distribution.** Refaire un bundle Tauri, verifier l'application installee,
   la reprise des donnees existantes et l'ouverture des liens dans le navigateur
   systeme. La compilation native et Clippy passent ; aucun nouvel installateur
   n'a ete publie durant cet audit.

Les versions majeures plus anciennes de reqwest, rusqlite et sha2 ne sont pas
automatiquement obsoletes. Le verrou a ete audite ; leurs migrations majeures
doivent faire l'objet d'une validation dediee plutot que d'une mise a jour aveugle.

## Verification

- 57 tests Rust passent, contre 28 avant les corrections.
- Clippy strict sur workspace et desktop.
- Formatage verifie pour les deux workspaces.
- cargo-machete ne trouve plus de dependance directe inutilisee.
- cargo-audit : aucune vulnerabilite dans les deux verrous ; aucune alerte dans
  le workspace principal ; les deux alertes transitives Linux ci-dessus restent.
- Test HTTP `tests/api-smoke.mjs` : recherche, visionneuse historique assainie,
  signets, contact/lecture, refus des origines/Host externes et des proxys locaux.
- Navigateur : recherche, lecture et signets fonctionnels ; HTML piege sans
  execution ; controles desktop et ecrans de 390 et 320 px sans debordement.
- Les tests utilisent des dossiers temporaires ; la base existante n'est ni
  migree ni reconstruite par les essais.

Commandes : `just test`, `just lint`, `just audit`, `just smoke` et
`cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --locked -- -D warnings`.

## References

- [Regles et groupes robots.txt - RFC 9309](https://www.rfc-editor.org/info/rfc9309/)
- [SQLite FTS5 : controle d'integrite et tables de contenu externe](https://sqlite.org/fts5.html#the_integrity_check_command)
- [Avis de securite h2](https://github.com/hyperium/hyper/security/advisories/GHSA-q83h-524g-xf6h)
- [Avis de securite rustls](https://github.com/rustls/rustls/security/advisories/GHSA-2mjx-qc3c-rqvc)
- [OWASP : stockage des mots de passe](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html)
