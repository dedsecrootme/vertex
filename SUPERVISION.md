# Supervision et stockage Vertex

## Panneau

Le raccourci de bureau `Vertex - Supervision` ouvre le panneau sur
`http://127.0.0.1:3108/panel`, apres lancement de l'API si necessaire.
`scripts/open-panel.ps1` refuse de reutiliser un port appartenant a une autre
application. `scripts/panel.ps1` active explicitement les commandes locales.
Demarrer/Arreter pilotent le superviseur, avec un maximum de 64 workers et
un ajustement automatique selon la RAM. Un arret conserve les URL en attente.
Ces commandes sont refusees en mode public, meme avec un jeton administrateur.
Le raccourci ne demarre pas le crawler sans action et n'installe pas de service
Windows au demarrage. Apres un redemarrage du PC, ouvrir le panneau et relancer.

Le serveur API fournit `/panel`, branche sur son propre `VERTEX_DB_DIR`.
Il ne s'agit pas d'une simulation. En mode public, les routes
`/api/admin/monitor` et `/api/admin/verify` exigent le jeton administrateur.
Le panneau conserve ce jeton en memoire uniquement, jamais dans l'URL ou
le stockage du navigateur. Ne pas exposer l'API locale directement sur Internet.

Le crawler publie un heartbeat par seconde, avec la presence reelle des threads,
leur phase et le domaine public traite, sans le contenu des pages.
Un worker est declare en ligne seulement si le verrou du crawler est actif
et si son heartbeat date de moins de 10 secondes. Le maximum configure
de 64 workers n'est donc pas automatiquement presente comme 64 workers en ligne.
Les phases reseau incluent l'attente des limites de chaque site.

Le panneau presente les compteurs, les 20 dernieres sessions, les erreurs,
la consommation disque, le chemin de la base, les limites, le nettoyage et
l'audit historique de la file. Le debit courant mesure la croissance du nombre
de pages, pas les exclusions de la file ; une moyenne des sessions terminees
sert de repli avant d'avoir cinq secondes de mesures fraiches.
Les echecs de tentative dans une session peuvent etre remis en attente ;
ils ne representent pas toujours des echecs definitifs d'URL.

## Pourquoi autant d'URL

Analyse exhaustive en lecture seule du 19 octobre 2026, en 45,7 secondes :

- 4 674 603 URL, 75 409 domaines ; aucun fragment ni URL syntaxiquement invalide.
- 3 804 989 URL ajoutees le 28 septembre 2026.
- 805 031 URL sur en.wikipedia.org, 443 412 sur github.com.
- 1 113 902 URL avec parametres ; `action` present sur 585 169 URL,
  `redlink` sur 311 604 et `section` sur 199 250. Ces ensembles se recoupent.
- 4 634 750 URL ont une profondeur historique de zero.
- La contrainte UNIQUE empeche les doublons textuellement identiques, pas
  les variantes de parametres qui pointent vers une meme ressource.

Le volume vient surtout d'un gros stock historique et d'une exploration
tres large avec beaucoup de variantes. L'ancienne base ne stocke pas la source
de chaque decouverte : les nombres sont prouves, l'origine exacte de chaque
URL ne peut pas etre reconstituee. Une profondeur zero ne prouve pas qu'une
URL etait une graine volontaire. Les anciennes versions autorisaient la
decouverte externe sans plafond global comparable a la limite actuelle.

Le filtre actuel exclut les interfaces Wikimedia d'edition, de suppression,
d'historique et les liens rouges connus. Il conserve les articles et leurs
parametres de lecture utiles. Les anciennes URL ne sont pas effacees : lors
de leur traitement, les exclusions portent une raison explicite.
Le mode drain n'ajoute toujours aucun nouveau lien.

Relancer l'analyse, sans toucher aux pages :

```powershell
node scripts/audit-queue.mjs
```

Le resultat est ecrit dans `database/queue-audit.json`, pas dans Git.
Les etats de cette analyse sont historiques ; les compteurs du panneau sont
les compteurs courants. Le scan ne bloque pas les ecritures du crawler.

## Optimisations appliquees

SQLite WAL + FTS5 reste le moteur embarque, sans service externe payant.
Le schema 6 ajoute des compteurs transactionnels pour les pages et les etats
des URL, ainsi que des references par domaine. Les triggers suivent aussi
les ecritures directes et les annulations de transaction. L'initialisation
calcule ces compteurs une fois ; le panneau ne rescane pas 4,6 millions de
lignes toutes les cinq secondes.

La reservation utilise l'index partiel ordonne `idx_urls_claim`, sans tri de
toute la file. Le demarrage utilise `idx_urls_safety` pour les URL malveillantes
et `idx_urls_retry_exhausted` pour les anciennes tentatives epuisees.
Cache SQLite borne a 32 Mio par connexion principale, journal recycle apres
checkpoint a 64 Mio lorsque possible, analyse des statistiques bornee.
La reconstruction FTS complete n'est plus repetee a chaque ouverture normale
d'une base deja migree et verifiee. Une verification n'est pas une reparation
automatique et ne supprime aucune URL pour rendre les compteurs rassurants.

## Verification et limites

Point de controle du 19 octobre 2026 a 01 h 47 (Paris), sur la vraie base :
20 tables controlees, structure SQLite `ok`, zero ecart de compteurs,
index de reservation present. Duree : 168 107 ms. Le crawler est reste actif
pendant le controle ; plus de 42 600 pages etaient alors indexees.
Les controles automatises ont aussi valide 63 tests Rust cibles, les deux
modes HTTP, 30 tests Chrome et le controle Clippy sans avertissement.
Captures et mesures reelles sur 1366, 390 et 320 pixels : aucun debordement,
12 workers confirmes lors des captures et graphique non vide.

Le bouton de verification lance un travail asynchrone avec une connexion en
lecture seule et un snapshot coherent. Il controle les tables ordinaires et
les tables physiques FTS, compare les compteurs pages/statuts avec les vraies
lignes et verifie la presence de l'index de reservation.
Il ne certifie ni la coherence semantique FTS5, ni toutes les contraintes
d'un `integrity_check` complet, ni la disponibilite des sites distants.
Le panneau indique ces limites au lieu d'afficher une certification globale.
La verification peut augmenter temporairement les lectures disque et le WAL.

Pour un futur index central ecrit par plusieurs serveurs, PostgreSQL constitue
une cible a evaluer avec une migration et des mesures dediees. Ce panneau
n'installe pas PostgreSQL, ne commande pas de serveur et n'active aucun
partage entre utilisateurs. Une base parfaite ou un stockage illimite ne
peuvent pas etre garantis. Le quota actuel de 80 Gio peut etre atteint avant que
les millions de pages soient traitees ; la file restante sera conservee.

References techniques :

- https://www.sqlite.org/whentouse.html
- https://www.sqlite.org/pragma.html#pragma_quick_check
- https://www.sqlite.org/pragma.html#pragma_optimize
