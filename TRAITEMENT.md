# Traitement du veritable index : 19 octobre 2026

## Point de depart

Base : `database/vertex.db`, schema historique 2.
Lecture initiale : 4 626 292 URL en attente, une reservation interrompue,
41 123 pages indexees, 7 161 echecs et 26 URL ignorees.
La reservation interrompue est remise en file au redemarrage, pas perdue.

Une sauvegarde SQLite coherente a ete creee avant migration :
`database/backups/before-drain-20261019.db` (2 459 709 440 octets).
Les fichiers de donnees et de sauvegarde ne sont pas ajoutes a Git.

## Premier resultat verifie

La migration vers le schema 5 est terminee. Le 19 octobre a 01 h 18 (Paris),
la session 12 s'est terminee apres 62 secondes : 74 pages indexees,
14 URL ignorees et 2 echecs remis en tentative. La file contenait alors
4 626 205 URL en attente. La session suivante a demarre automatiquement.
Le nombre de pages a depasse les 41 123 initiales : le traitement n'est pas
une simple modification cosmetique des statuts. Ces chiffres sont un point
de controle historique, pas des compteurs mis a jour en temps reel.

## Mode actif

Le mode `--drain` traite uniquement la file existante. Il n'ajoute ni les liens
sortants, ni les sitemaps, ni de nouvelles seeds. Il ne remet pas les pages deja
indexees en file : la progression ne doit pas etre annulee par la decouverte.
Les URL presentes sont traitees quelle que soit leur profondeur historique.

Un index SQL partiel ordonne permet de reserver la prochaine URL sans retrier
des millions de lignes a chaque requete. La migration ajoute cet index et la
colonne de prochaine tentative, sans supprimer la file existante.

Configuration depuis la migration SSD du 19 octobre : binaire `release`,
jusqu'a 64 workers adaptes a la RAM disponible, sessions de 300 secondes,
pause de 1 seconde entre sessions. Politesse par domaine et robots.txt maintenus.
Les sessions drain n'ont plus de plafond artificiel de 10 000 reservations.
Les pages recues sont enregistrees et indexees FTS5 ; les erreurs, interdictions
et types de contenu non exploitables restent identifies separement.
Une URL traitee ne signifie pas toujours une page indexable ou accessible.

## Pilotage depuis la racine du projet

```powershell
Set-Location D:\Vertex
.\scripts\crawl-supervisor.ps1 -Action Status
.\scripts\crawl-supervisor.ps1 -Action Stop
.\scripts\crawl-supervisor.ps1 -Action Start -Workers 64 -Profile release
```

Le superviseur lance aussi le nettoyage de la file et adapte les workers en
cas de pression memoire. Il attend une recuperation de la RAM si necessaire.
Le statut donne le PID, les journaux et les derniers resultats de session.
L'arret utilise `database/supervisor.stop`, `cleanup.stop` et `drain.stop` : les requetes en cours finissent selon
leur timeout, les reservations non executees sont rendues a la file.
Ne pas supprimer les URL restantes pour faire baisser artificiellement le compteur.
Le traitement peut reprendre apres redemarrage avec `-Action Start`.
Il n'installe aucun service ni demarrage automatique a l'ouverture de session.

## Limites de ressources

Le lancement conserve au moins 50 Gio libres et arrete le processus si les
fichiers SQLite et WAL atteignent 80 Gio. Controle avant chaque nouvelle page ;
un petit depassement reste possible avec les requetes deja en cours.
Aucune donnee restante n'est effacee a l'atteinte de ces limites.

L'indexation de millions de pages peut exiger davantage de stockage. Le quota
est une protection du PC, pas une garantie que toutes les pages tiendront dans
80 Gio. Le temps necessaire depend des sites, de leurs delais et de leur acces.
Le PC doit rester allume et connecte ; aucun serveur distant n'a ete achete.

## Migration sur le SSD D

Projet actif, base, journaux, builds et fichiers temporaires de cette operation :
`D:\Vertex`. L'ancien chemin `C:\Users\Administrateur\Vertex` est une jonction
de compatibilite vers ce dossier, pas une seconde copie active.
La copie originale figee est conservee dans
`D:\Vertex-migration-backup-20261019-135151` ; ne pas l'utiliser comme base active.
Les outils globaux Rust et Node partages avec d'autres projets restent en place.

Avant bascule : checkpoint WAL, comparaison de 38 945 fichiers et empreintes
SHA-256 de 252 fichiers non generes, dont la base et sa sauvegarde historique.
Rapports : `database/ssd-migration-verification.json`, `migration-state.json`
et `before-ssd-migration.json`. Apres bascule, le controle semantique FTS5
`integrity-check rank=1` a reussi sur les 84 267 pages presentes a cet instant
(`database/fts-after-migration.json`). Ce n'est pas une certification globale
de securite ni de disponibilite reseau.

Le superviseur empeche la veille du PC pendant son fonctionnement, tout en
laissant les ecrans s'eteindre. Aucun service systeme n'est desactive et aucun
demarrage automatique apres redemarrage de Windows n'est installe.

## Optimisations et mesure du debit

La file utilise un cache borne a 4 096 candidats, au plus 32 par hote, et une
pagination par cle sur l'index SQL existant. Un hote occupe ne monopolise plus
la tete de file ; les candidats sont revalides atomiquement a la reservation.
L'enregistrement de la page, des liens, de FTS5 et du statut termine constitue
une seule transaction. Les requetes d'insertion des liens/images sont reutilisees.
La construction des gros DOM HTML est bornee separement des workers reseau.
La reservation maximale de workers n'est pas un nombre de workers garantis.

Test local du 19 octobre a 14 h 19 : 1 000 documents HTML de 12,6 ko environ,
parsing, nettoyage, liens et index FTS5 sur une base neuve du SSD D :
230,8 pages/s en 4,33 secondes, 1 000 resultats recherches et coherence FTS validee.
Rapport : `test-results/index-bench-20261019-141906/benchmark.json`.
Ce test exclut le reseau, DNS, robots.txt, les delais et la reservation dans
la vraie file : il ne prouve pas un crawl Internet a 50 pages/s.

Le panneau `/panel` mesure la croissance reelle du nombre de pages sur environ
une minute. Les exclusions du nettoyage ne sont pas comptees comme des pages.
Le nettoyage reprend son curseur, ne vise que les URL pending exclues par la
politique Wikimedia existante et ne supprime ni pages ni signets. Son passage
est borne a 24 heures ; le crawl supervise continue tant que les ressources
et la file le permettent. Arret propre si le quota disque est atteint.

Validation apres modifications : 70 tests Rust cibles reussis, 57 tests Chrome
sur trois formats d'ecran, 3 tests du nettoyage (dont verrou ecrivain prolonge),
smoke HTTP local et public, Clippy sans avertissement et format Rust verifie.
La fixture TCP Windows fixe explicitement son mode bloquant ; les protections
reseau reelles ne sont pas assouplies pour faire passer les tests.

Une contention d'ecriture avec le nettoyage a ete observee sur la vraie base
(`database is locked`). Les sauvegardes atomiques utilisent maintenant
`BEGIN IMMEDIATE` et les sauvegardes/reservations disposent de quatre essais
bornes pour les erreurs SQLite BUSY/LOCKED uniquement. Les autres erreurs ne
sont pas masquees. Le nettoyage laisse 125 ms entre groupes d'exclusions.
Un test tient un verrou d'ecriture externe et controle ensuite l'unique page,
son statut et la coherence FTS5. Un verrou permanent reste une erreur a traiter,
pas une garantie de fonctionnement sans interruption.

Le premier releve apres les tests a constate 124 pages nouvelles en 44 secondes,
soit environ 2,8 pages/s, avec adaptation de 8 vers 4 workers sous pression RAM.
Ce point est une mesure ponctuelle, pas un engagement de debit constant.
Les releves debut/fin et le debit de la fenetre complete sont conserves dans
`database/crawl-measurement-start.json`, `crawl-measurement-end.json` et
`crawl-measurement.json`. L'objectif de 50 pages/s sur Internet n'est pas atteint.

Apres le correctif des verrous : sept releves sur 90 secondes, 4 workers sur 4
en ligne a chaque releve, 136 pages nouvelles (1,51 page/s) et 5 708 exclusions
de nettoyage. Aucun nouveau message d'erreur SQLite dans le journal sur cette
fenetre. Rapport : `database/crawl-after-lock-fix.json`.
Ce controle court n'est pas une garantie de disponibilite permanente.
