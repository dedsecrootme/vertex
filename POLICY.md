# Politique Vertex — données, IA et confidentialité

Vertex est un moteur de recherche **personnel, local et souverain**.
Cette politique décrit des garanties **techniques**, pas seulement des intentions.

## 1. Aucun tiers n'a le droit d'exploiter vos données

- **Aucune IA** — locale ou distante — n'est autorisée à lire, copier, agréger,
  entraîner un modèle sur, revendre ou réutiliser les données de Vertex
  (pages crawlees, index, signets, messages, métadonnées).
- **Aucune donnée ne quitte la machine.** Il n'existe aucun endpoint de
  télémétrie, d'analytics, de « crash report » ou de synchronisation distante.
- **Aucune revente, aucun partage, aucune affiliation.** Vertex ne monétise rien
  et ne cède rien.

## 2. Garanties techniques (vérifiables dans le code)

| Garantie | Mise en œuvre |
|---|---|
| Pas de télémétrie | Aucun appel réseau non sollicité ; tout part d'une action utilisateur |
| Pas de trackers | Images/favicons servis par **proxy local** ; paramètres de suivi retirés ; `referrerpolicy=no-referrer` |
| Pas de publicité / affiliation | Les blocs publicitaires et paramètres d'affiliation sont supprimés du HTML affiché |
| Zéro journal persistant | Rien n'est écrit dans un fichier de log ; les événements restent en mémoire volatile |
| Données locales | Base et index stockés sur la machine de l'utilisateur, jamais ailleurs |
| Coffre-fort | Secrets dérivés d'une phrase maître, gardés en mémoire, effacés à la fermeture |

## 3. Ce que Vertex s'interdit vis-à-vis du web

- **Identification honnête** : le crawler envoie un `User-Agent` explicite
  (`VertexCrawler/<version>`) — aucun déguisement en navigateur.
- **Respect strict de `robots.txt`** (RFC 9309) : `Allow`/`Disallow` avec
  jokers `*` et `$`, matching par groupe d'agent, `Crawl-delay`, `Sitemap`.
- **Politesse** : délai par domaine, backoff en cas de `429`/`503`.
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
- Mots de passe : **PBKDF2-HMAC-SHA256, 200 000 itérations**, comparaison à
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
*Vertex — « rien ne sort, rien n'entre sans votre coffre-fort. »*
