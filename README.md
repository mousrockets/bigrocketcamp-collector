# Big Rocket Camp Collector

Le petit outil Windows qui envoie tes matchs Rocket League à [Big Rocket Camp](https://mousrockets.com).

Une fois réglé, tu ne le vois plus : il démarre avec Windows, reste près de l'horloge et envoie chaque tableau de fin de match au club. Si Internet coupe, les matchs attendent sur le PC et partent au retour de la connexion.

## Télécharger

[**Télécharger l'installateur**](https://github.com/mousrockets/bigrocketcamp-collector/releases/latest/download/BigRocketCampCollector-setup.exe) (dernière version, Windows 10 ou 11). Toutes les versions sont dans les [releases](https://github.com/mousrockets/bigrocketcamp-collector/releases).

## Installer

1. Ouvre `BigRocketCampCollector-setup.exe`.
2. Windows affiche « Windows a protégé votre ordinateur » : l'outil n'est pas encore signé, c'est normal. Clique sur **Informations complémentaires**, puis **Exécuter quand même**.
3. L'installation se fait pour ton compte Windows, sans mot de passe administrateur. Elle crée un raccourci sur le bureau et dans le menu Démarrer.

Pour le désinstaller : Paramètres → Applications → Big Rocket Camp Collector. Ses données sur le PC partent avec lui.

## Premier lancement

1. **Associe le collecteur** : dans Big Rocket Camp, ouvre ton profil et clique « Associer le collecteur ». Recopie le code à 8 caractères dans la fenêtre du collecteur, puis **Associer**.
2. **Rocket League** est réglé tout seul au lancement (Epic Games ou Steam). Si le collecteur te dit qu'il l'a réglé, relance le jeu. S'il n'y arrive pas, le bouton **Aide** explique le réglage à la main.
3. **C'est fini.** Ferme la fenêtre avec la croix : le collecteur reste dans la zone de notification, près de l'horloge, et démarre ensuite avec Windows.

La pastille et l'icône changent de couleur :

- **verte** : en direct, le jeu est connecté et les matchs partent à la fin ;
- **grise** : en attente de Rocket League ;
- **rouge** : un problème, expliqué en une phrase dans la fenêtre.

Un clic sur l'icône rouvre la fenêtre. Un clic droit propose : Ouvrir, Aide, Mettre à jour, Rechercher les mises à jour, Lancer au démarrage de Windows, Quitter.

Un match compte dès que le jeu annonce le gagnant, forfait compris : les joueurs déjà partis ou remplacés par un bot ne figurent simplement pas dans le tableau envoyé. Si tu quittes avant la fin du match, aucun résultat n'est envoyé.

## Mises à jour

- Le collecteur regarde s'il existe une nouvelle version au lancement, puis toutes les 6 heures. **Rechercher les mises à jour** (dans la fenêtre ou le menu de l'icône) vérifie tout de suite.
- Quand une version est disponible, un bouton **Mettre à jour (vX.Y.Z)** apparaît dans la fenêtre et dans le menu, et l'icône porte un point bleu. Rien ne s'ouvre tout seul.
- La mise à jour ne se fait qu'au clic. Elle télécharge l'installateur, vérifie son empreinte SHA-256 avec le fichier `SHA256SUMS` de la release, l'installe sans questions puis relance le collecteur.
- Pendant un match, elle attend la fin du match : la fenêtre affiche « Mise à jour prévue à la fin du match », et une notification Windows le confirme, puis annonce le début de la mise à jour. Ces notifications ne prennent pas le focus et Windows les retient pendant un jeu en plein écran.
- L'association et les matchs en attente sont conservés.

## Léger pendant les parties

- Priorité basse et mode efficacité de Windows : le jeu passe toujours en premier.
- Aucune boucle active : le collecteur attend les messages du jeu et de Windows.
- Un seul envoi réseau par fin de match, et aucune écriture sur le disque pendant une partie.

## Ce que le collecteur garde sur le PC

Dans `%LOCALAPPDATA%\BigRocketCampCollector`, supprimé à la désinstallation :

- `token.bin` : le jeton d'association, chiffré avec DPAPI pour ton seul compte Windows ;
- `queue.json` : les matchs pas encore reçus par le club (20 au plus, moins d'un jour) ;
- `settings.json` : le nom du joueur associé et ton choix de démarrage avec Windows.

Il modifie aussi trois réglages du Stats API dans le dossier de Rocket League (`TAStatsAPI.ini` ou `DefaultStatsAPI.ini`), après en avoir gardé une copie `.bak`.

## API attendue

Le collecteur parle à l'instance définie à la compilation (`APP_URL`, par défaut `https://mousrockets.com`). Le contrat de référence est `docs/collecteur.md` dans `mousrockets/bigrocketcamp` ; voici ce que le collecteur en utilise. Il ne suit aucune redirection et s'annonce avec `User-Agent: bigrocketcamp-collector/<version>`.

### Associer : `POST /api/collector/pair`

Dans l'app web, le membre clique « Associer le collecteur » dans son profil et reçoit un code de 8 caractères (`ABCD EFGH`), valable 10 minutes et utilisable une fois.

```http
POST /api/collector/pair
Content-Type: application/json

{"code":"AB12CD34"}
```

- Le collecteur envoie le code en majuscules, sans espaces ni tirets. Aucun cookie, aucun en-tête `Authorization`.
- `200` : `{"token":"…","player":"joueur-un"}`. `player` est le nom affiché dans la fenêtre.
- `400` (code mal formé) ou `401` (code inconnu, expiré, déjà utilisé ou bloqué) : le collecteur affiche `error`, sinon « Code invalide ou expiré. ».

### Envoyer un match : `POST /api/live`

```http
POST /api/live
Authorization: Bearer <token>
Content-Type: application/json

{"state":{…},"end":{…},"startedAt":…,"endedAt":…,"seenIds":[…]}
```

- Le corps garde le format du pont web, champ pour champ et dans le même ordre (voir `src/capture.rs`, porté de `src/lib/live-stats.ts`).
- `state.Players` ne contient que les joueurs qui ont un identifiant valide : au moins un, au plus le nombre de joueurs de la playlist, et la moitié au plus par équipe. Les bots et les joueurs partis après un forfait en sont absents.
- `seenIds` contient les identifiants de ces mêmes joueurs, dans l'ordre où le collecteur les a vus.
- Le gagnant vient de `MatchEnded` : `Game.bHasWinner` vaut `true` et `Game.Winner` est le nom de l'équipe `WinnerTeamNum`.
- `Game.TimeSeconds` et `Game.bOvertime` sont l'horloge de la dernière mise à jour avant `MatchEnded` : du temps restant hors prolongation, c'est un forfait des perdants. Une mise à jour après la fin ne fait baisser le temps restant que s'il était d'une seconde au plus (le passage à 0:00 arrivé en retard) : le jeu peut remettre l'horloge à 0:00 après un forfait. Absents si le jeu ne les envoie pas.
- `clockTrace` (depuis 0.4.0) : les six derniers changements de l'horloge avant `MatchEnded` et les six premiers après, chacun avec `TimeSeconds`, `bOvertime` et `ended`. L'API les garde tels quels pour vérifier comment le jeu termine un forfait. Absent si le jeu n'envoie pas d'horloge.
- Démos et poteaux (depuis 0.6.0) : chaque joueur de `state.Players` porte `Demos` (ses démolitions, telles que le jeu les donne, absent s'il ne les donne pas), puis `CrossbarHits` : le nombre d'événements `CrossbarHit` (ballon sur la barre ou un poteau) reçus avant `MatchEnded` dont il était le dernier à toucher le ballon, retrouvé par `BallLastTouch.Player.Shortcut` et son équipe. Les départs portent les mêmes deux champs.
- `departures` (depuis 0.5.0) : les joueurs identifiés vus avant `MatchEnded` et absents de la dernière mise à jour reçue avant `MatchEnded`, dans l'ordre où le collecteur les a vus, huit au plus. Chacun garde ses dernières valeurs : `PrimaryId`, `Name`, `TeamNum`, `Score`, `Goals`, `Assists`, `Saves`, `Demos`, `CrossbarHits`. L'API compte une défaite au joueur du club parti avant la fin, et un forfait gagné quand tous les adversaires sont partis. Un joueur revenu avant la fin n'y est pas, ni un joueur parti pendant le podium. Absent si personne n'est parti.
- Départ avant la fin (depuis 0.5.0) : quand le joueur quitte un match suivi depuis sa création avant tout `MatchEnded` (`MatchDestroyed` de ce match, ou un autre match qui commence), le collecteur envoie `{"left":true,"state":{…},"startedAt":…,"leftAt":…,"seenIds":[…]}`. `state` est le dernier tableau connu du match, hors replay et avec au moins un joueur identifié, avec les mêmes champs de joueurs et d'équipes que le tableau de fin, mais sans gagnant : `Game` ne contient que `PlaylistId`, `Teams` et `bReplay`. Ni `end` ni `endedAt` ; `leftAt` est l'heure du départ, en millisecondes. Rien n'est envoyé pour un replay, une playlist privée ou inconnue, ni un match rejoint en cours, ni quand le jeu se ferme ou que la connexion au Stats API se coupe : rien ne dit alors que le joueur est parti, et un départ compté à tort serait une fausse défaite. L'API n'en retient que le départ du membre du jeton, compté comme une défaite. La réponse attendue est la même que pour un match, et un tableau de fin du même match encore dans la file n'est jamais remplacé par un départ.
- Le collecteur envoie le tableau dès qu'il reçoit deux fois de suite le même après `MatchEnded`. Sinon, il envoie le dernier tableau connu du match quand le joueur le quitte, quand un autre match commence ou quand le jeu se ferme.
- Sans cookie ni en-tête `Origin` : avec un jeton `Bearer`, la route ne doit pas exiger la même origine. Elle vérifie toujours que le tableau contient un identifiant de jeu du membre du jeton.
- `200` : `{"ok":true,"match":"<MatchGuid sans espaces, en minuscules>"}`. Sans cette confirmation exacte, le match reste dans la file.
- `400`, `403` (aucun joueur du membre dans le tableau) ou `413` : refus définitif, le match est retiré de la file.
- `401` : jeton révoqué. Le collecteur l'efface et redemande un code.
- Autre réponse, ou site injoignable : nouvel essai 10 secondes plus tard. `error` est affiché s'il est présent.

### Vérifier l'association : `GET /api/collector/status`

```http
GET /api/collector/status
Authorization: Bearer <token>
```

- Appelé une fois au lancement.
- `200` : `{"player":"joueur-un"}`.
- `401` : jeton révoqué, le collecteur redemande un code.

## Publier une version

Tout passe par la CI, sans manipulation à la main :

1. Monter `version` dans `Cargo.toml` (par exemple `0.3.0`), par PR, et la fusionner une fois la CI verte.
2. Pousser le tag sur `main` : `git tag v0.3.0 && git push origin v0.3.0`.
3. La CI vérifie que le tag correspond à `Cargo.toml`, construit et mesure le collecteur, crée l'installateur, puis publie la release `v0.3.0` avec `BigRocketCampCollector-setup.exe` et `SHA256SUMS`.
4. Les collecteurs installés voient la nouvelle version au plus tard 6 heures après (ou tout de suite avec « Rechercher les mises à jour »).

Une release sans `SHA256SUMS` n'est jamais proposée en mise à jour.

## Pour les développeurs

Les règles du projet et les commandes sont dans [AGENTS.md](AGENTS.md). Une autre instance de Big Rocket Camp construit son propre collecteur en définissant `APP_URL` à la compilation (variable de dépôt `APP_URL` en CI).

Licence MIT.
