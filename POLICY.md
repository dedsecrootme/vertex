# Politique Vertex — données et confidentialité

Vertex propose un index personnel local ou un index public partage explicitement configure.
Cette politique distingue les objectifs du projet des protections présentes dans le code.

Le mode personnel reste local. En mode web public explicitement active,
l'exploitant partage un index dedie : les recherches et le contenu consulte
passent par son serveur. Les messages du formulaire y sont enregistres, sans
envoi automatique d'email. Les signets visiteurs restent dans leur navigateur.
Le jeton administrateur ne doit jamais etre transmis aux visiteurs.
Le crawl continu sur un PC est desactive par defaut. La poursuite apres fermeture
et la proposition d'URL sont deux autorisations distinctes. La destination du
partage est affichee avant accord ; seules les nouvelles URL explorees apres
cet accord peuvent etre proposees, sans parametres de requete, contenu de page,
signets ni historique anterieur. Le serveur refait ses propres verifications.
Ce mecanisme ne publie pas les pages envoyees par les clients et ne constitue
pas encore un reseau distribue avec attribution globale des travaux.
L'exploitant doit renseigner son identite, ses coordonnees, sa retention des
messages et la procedure de suppression avant publication ; voir PUBLICATION.md.

## 1. Aucun tiers n'a le droit d'exploiter vos données

- Le moteur de recherche ne transmet pas les signets ni les secrets du coffre a l'index partage.
  Cette politique ne peut pas empêcher un autre logiciel ayant accès à la
  machine de lire des fichiers locaux.
- L'index personnel et les signets restent sur la machine. En mode partage,
  les recherches et pages consultees passent par le serveur configure. Les
  demandes explicites d'indexation d'URL publiques y sont mises en file.
  Aucun endpoint de telemetrie, d'analytics ou de crash report n'est ajoute.
  Les requêtes du crawler et des proxys transmettent nécessairement leur URL et
  l'adresse IP de la machine aux sites contactés. Un proxy local ne rend pas
  ces requêtes anonymes.
- Le moteur local ne revend pas l'index ni les signets. Le service communautaire
  peut proposer les annonces payantes décrites dans la section 4.

## 2. Garanties techniques (vérifiables dans le code)

| Garantie | Mise en œuvre |
|---|---|
| Pas de télémétrie | Pas d'analytics ; le crawl continu local demande un accord explicite, revocable dans l'application |
| Réduction du suivi | Images/favicons servis par **proxy local** ; paramètres connus retirés ; `referrerpolicy=no-referrer`. Pas de garantie de détecter tous les trackers |
| Pas de publicité / affiliation | Les blocs publicitaires et paramètres d'affiliation sont supprimés du HTML affiché |
| Journalisation limitee | Sortie console ; erreurs, statuts et resumes de crawl dans SQLite ; diagnostic local du worker dans contributor-error.txt |
| Donnees privees | Signets locaux ; index partage sur un serveur dedie, sans acces visiteur aux tables privees |
| Coffre-fort | Secrets chiffrés sur disque ; accès refusé après inactivité ; effacement des buffers possédés à l'accès expiré ou à la fermeture |

## 3. Ce que Vertex s'interdit vis-à-vis du web

- **Identification honnête** : le crawler envoie un `User-Agent` explicite
  (`VertexCrawler/<version>`) — aucun déguisement en navigateur.
- **Prise en charge de `robots.txt`** : `Allow`/`Disallow` avec
  jokers `*` et `$`, matching par groupe d'agent, `Crawl-delay`, `Sitemap`.
- **Politesse** : delai par domaine, robots.txt et reprise differee sur
  `429`/`503`, avec un plafond de tentatives. Le mode continu renouvelle les
  regles et conserve les delais entre sessions.
- **Aucun contournement** de paywall, d'authentification ou de protection technique.

## 4. Services communautaires (`vx-hub`)

Le chat, les forums, les avis et les partenaires sont opérés par **vx-hub**, un
service **séparé et optionnel** du cœur local :

- On n'y transmet **que** ce que l'utilisateur publie volontairement (messages,
  sujets, avis, dépôt d'annonce) — **jamais** l'index ni le crawler.
- **Aucun journal de requêtes** : le serveur n'écrit pas de fichier de log ; les
  événements restent en mémoire volatile.
- **Aucune IP stockée**, aucun identifiant tiers, aucune télémétrie, aucune
  publicité tierce (les partenaires sont des liens vers des Discord approuvés à
  la main).
- Mots de passe : **PBKDF2-HMAC-SHA256, 600 000 itérations** pour les nouvelles
  empreintes, compatibilité avec les anciennes ; comparaison à
  temps constant ; jetons de session aléatoires 256 bits.
- Les avis sont fermés tant que le seuil des **premiers utilisateurs** n'est
  pas atteint (`REVIEWS_MIN_USERS`).
- La publicité Discord est **modérée et payante** : aucune annonce n'est
  publiée sans validation explicite de l'administrateur.

## 5. Usage

Cette politique s'applique à **toute** version de Vertex (desktop, web, API,
crawler) et à **tout** composant ajouté ultérieurement. Toute contribution qui
transgresse l'un de ces points est refusée par principe.

---
*Vertex conserve l'index localement et documente ses accès réseau.*
