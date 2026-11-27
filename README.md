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
- Démos et poteaux (depuis 0.6.0) : chaque joueur de `state.Players` porte `Demos` (ses démolitions, telles que le jeu les donne, absent s'il ne les donne pas), puis `CrossbarHits` : le nombre d'événements `CrossbarHit` (ballon sur la barre ou un poteau) reçus avant `MatchEnded` dont il était le dernier à toucher le ballon, retrouvé par `BallLastTouch.Player.Shortcut` et son équipe. Depuis 0.6.1, seul compte le cadre du but adverse (`BallLocation.Y` positif pour l'équipe 0, négatif pour l'équipe 1), une seule fois par toucher de balle (`BallHit`), et rien entre `GoalScored` et le coup d'envoi suivant (`CountdownBegin`, `RoundStarted`), car le ralenti du but renvoie les mêmes contacts. Les départs portent les mêmes deux champs.
- Toutes les stats du jeu (depuis 0.7.0) : chaque joueur de `state.Players` et chaque départ portent, après `CrossbarHits` et dans cet ordre, les champs décrits plus bas. Les champs plus anciens gardent leur place.
- `departures` (depuis 0.5.0) : les joueurs identifiés vus avant `MatchEnded` et absents de la dernière mise à jour reçue avant `MatchEnded`, dans l'ordre où le collecteur les a vus, huit au plus. Chacun garde ses dernières valeurs : `PrimaryId`, `Name`, `TeamNum`, `Score`, `Goals`, `Assists`, `Saves`, `Demos`, `CrossbarHits`, puis depuis 0.7.0 `Shots`, `Touches`, `CarTouches`, `Statfeed`, `StatfeedSecondary`, `KickoffTouches`, `OwnGoals`, `AssistsTo`, `FastestGoal`, `LongestGoal`, `QuickestGoal`, `HardestHit`. L'API compte une défaite au joueur du club parti avant la fin, et un forfait gagné quand tous les adversaires sont partis. Un joueur revenu avant la fin n'y est pas, ni un joueur parti pendant le podium. Absent si personne n'est parti.
- Départ avant la fin (depuis 0.5.0) : quand le joueur quitte un match suivi depuis sa création avant tout `MatchEnded` (`MatchDestroyed` de ce match, ou un autre match qui commence), le collecteur envoie `{"left":true,"state":{…},"startedAt":…,"leftAt":…,"seenIds":[…]}`. `state` est le dernier tableau connu du match, hors replay et avec au moins un joueur identifié, avec les mêmes champs de joueurs et d'équipes que le tableau de fin, mais sans gagnant : `Game` ne contient que `PlaylistId`, `Teams` et `bReplay`. Ni `end` ni `endedAt` ; `leftAt` est l'heure du départ, en millisecondes. Rien n'est envoyé pour un replay, une playlist privée ou inconnue, ni un match rejoint en cours, ni quand le jeu se ferme ou que la connexion au Stats API se coupe : rien ne dit alors que le joueur est parti, et un départ compté à tort serait une fausse défaite. L'API n'en retient que le départ du membre du jeton, compté comme une défaite. La réponse attendue est la même que pour un match, et un tableau de fin du même match encore dans la file n'est jamais remplacé par un départ.
- Le collecteur envoie le tableau dès qu'il reçoit deux fois de suite le même après `MatchEnded`. Sinon, il envoie le dernier tableau connu du match quand le joueur le quitte, quand un autre match commence ou quand le jeu se ferme.
- Sans cookie ni en-tête `Origin` : avec un jeton `Bearer`, la route ne doit pas exiger la même origine. Elle vérifie toujours que le tableau contient un identifiant de jeu du membre du jeton.
- `200` : `{"ok":true,"match":"<MatchGuid sans espaces, en minuscules>"}`. Sans cette confirmation exacte, le match reste dans la file.
- `400`, `403` (aucun joueur du membre dans le tableau) ou `413` : refus définitif, le match est retiré de la file.

#### Les stats de chaque joueur (depuis 0.7.0)

Un joueur du tableau de fin, avec tous les champs possibles, dans leur ordre :

```json
{"Name":"joueur-un","PrimaryId":"Epic|0123456789abcdef0123456789abcdef|0","TeamNum":0,
 "Score":420,"Goals":2,"Assists":1,"Saves":3,"Demos":1,"CrossbarHits":1,
 "Shots":5,"Touches":38,"CarTouches":4,
 "Statfeed":{"Save":3,"Demolish":1},"StatfeedSecondary":{"Demolish":2},
 "KickoffTouches":3,"OwnGoals":0,"AssistsTo":{"Steam|76561198000000000|0":1},
 "FastestGoal":87.3,"LongestGoal":3162,"QuickestGoal":12.5,"HardestHit":1450.2}
```

Les noms d'événements de l'exemple sont les deux que cite la doc du jeu (`Save`, `Demolish`) ; les intitulés plus bas sont illustratifs.

Règles communes :

- Le joueur visé par un événement est retrouvé par son `Shortcut` et son équipe, comme pour les poteaux. Un événement qui ne vise aucun joueur identifié du tableau (un bot, un `Shortcut` absent) ne compte pour personne.
- Rien ne compte après `MatchEnded`.
- Touchers et buts (`BallHit`, `GoalScored`) : rien entre `GoalScored` et le coup d'envoi suivant (`CountdownBegin`, `RoundStarted`), car le ralenti du but renvoie les mêmes contacts.
- Fil de stats (`StatfeedEvent`) : rien pendant le ralenti d'un but, de `GoalReplayStart` à `GoalReplayEnd`, ou jusqu'au coup d'envoi suivant si le ralenti est passé. Les événements du but lui-même, envoyés entre `GoalScored` et `GoalReplayStart`, comptent.

Les champs :

- `Shots`, `Touches` (touchers de balle), `CarTouches` (touchers par le châssis de la voiture, selon la doc) : les valeurs de la dernière mise à jour du joueur, telles que le jeu les donne, si ce sont des entiers de 0 à 1 000 000. Absents sinon.
- `Statfeed` : toujours présent, `{}` si rien n'est compté. Pour chaque `EventName` de `StatfeedEvent`, le nombre d'événements dont le joueur est la cible principale (`MainTarget`). Le collecteur ne connaît aucun nom à l'avance et compte ce que le jeu envoie. Seuls comptent les noms de 1 à 32 caractères parmi `A-Z`, `a-z`, `0-9` et `_`, et 30 noms différents au plus par match : au-delà, un nom nouveau est ignoré, et les noms déjà vus continuent de compter. Les noms sont dans l'ordre du premier événement reçu pour ce joueur.
- `StatfeedSecondary` : la même chose pour les événements dont le joueur est la cible secondaire (`SecondaryTarget`, « le joueur détruit » d'une démolition selon la doc). Les démolitions subies sont `StatfeedSecondary.Demolish`, y compris celles d'un bot. Absent si rien n'est compté.
- `KickoffTouches` : toujours présent. Le nombre de manches où le joueur figure dans le premier `BallHit` après `RoundStarted`. Un contact à deux au coup d'envoi compte pour les deux.
- `OwnGoals` : toujours présent. Le nombre de `GoalScored` dont il est le `Scorer` et qui entrent dans le but de son équipe (`ImpactLocation.Y` : le bleu, équipe 0, défend le but en Y négatif). Si le jeu attribue un but contre son camp à un joueur de l'autre équipe, il n'est pas compté ici. À vérifier sur un vrai match.
- `AssistsTo` : pour chaque coéquipier identifié dont il a donné la passe décisive (`Assister` d'un `GoalScored` dans le but adverse), le nombre de passes, par `PrimaryId` du buteur, tel que le jeu l'écrit dans `state.Players`. Absent s'il n'en a donné aucune.
- `FastestGoal` : la plus grande `GoalSpeed` de ses buts dans le but adverse.
- `LongestGoal` : la plus grande distance de ses buts, en unités Unreal entières : du ballon à son dernier toucher (`BallHit.Ball.Location`) au milieu de la ligne de but adverse (X = 0, Y = ±5 120), au sol (X et Y, sans la hauteur). Seulement quand le dernier `BallHit` avant le but est le sien.
- `QuickestGoal` : le plus petit `GoalTime` de ses buts, en secondes : la durée de la manche, du coup d'envoi au but.
- `HardestHit` : la plus grande `Ball.PostHitSpeed` des `BallHit` où il figure dans `Players`. Un contact à deux compte pour les deux.
- Pour ces quatre champs : la valeur telle que le jeu l'envoie (la distance est calculée par le collecteur), seulement si elle est strictement positive et d'au plus 100 000 pour une vitesse, 10 800 pour une durée, 20 000 pour une distance. Absents sinon. Les vitesses sont en unités Unreal par seconde selon la doc (1 uu/s = 0,036 km/h). Attention : l'exemple de `GoalScored` dans la doc donne `"GoalSpeed": 87.3`, qui ressemble plus à des km/h. À vérifier sur un vrai match avant de convertir.
- `LongestGoal` suppose un terrain standard. Sur Hoops (playlist 27) ou Dropshot, il n'a pas de sens : à filtrer par `PlaylistId`.

#### Le boost, le mouvement, la possession et les zones de touche (depuis 0.8.0)

Après les champs précédents, un joueur gagne, quand le match a eu au moins une mise à jour de jeu :

```json
"AerialTouches":12,"DefensiveTouches":16,"OffensiveTouches":24,
"Possession":{"Samples":3651,"Owned":1781},
"Car":{"Samples":3716,"BoostSum":175458,"NoBoost":353,"FullBoost":296,"Supersonic":530,"OnGround":2587,"OnWall":315,
       "Boosting":844,"Powerslides":92,"Pickups":118,"BigPickups":25,"BoostCollected":2970}
```

Une mise à jour de jeu est un `UpdateState` hors ralenti (`bReplay` faux), entre `RoundStarted` et le but suivant, et avant `MatchEnded`. Le jeu en envoie environ dix par seconde (`PacketSendRate`). Les comptes de mises à jour se lisent comme une part du temps, quel que soit ce rythme.

- `AerialTouches`, `DefensiveTouches`, `OffensiveTouches` : parmi ses touchers comptés comme pour `HardestHit`, ceux où la balle est à plus de 300 unités de haut (`BallHit.Ball.Location.Z`), dans le tiers du terrain de son propre but, ou dans celui du but adverse (Y au-delà de ±5 120 / 3).
- `Possession` : `Samples`, le nombre de mises à jour de jeu où une équipe a touché la balle en dernier (`Game.Ball.TeamNum` 0 ou 1) ; `Owned`, celles où c'était la sienne. Les deux coéquipiers ont la même.
- `Car` : seulement pour une voiture que le jeu montre. Pendant le jeu, il ne montre que celles de l'équipe du joueur qui a lancé le collecteur ; les autres arrivent sans `bHasCar`. Absent si aucune mise à jour ne l'a montrée.
  - `Samples` : les mises à jour de jeu où la voiture est sur le terrain (`bHasCar` vrai, `bDemolished` faux).
  - `BoostSum` : la somme de leur `Boost`, de 0 à 100, arrondi. Un réservoir vide arrive sans `Boost`. `BoostSum / Samples` donne le boost moyen.
  - `NoBoost`, `FullBoost` : celles à 0 et à 100 de boost. `Supersonic`, `OnGround`, `OnWall`, `Boosting` : celles où le drapeau du même nom (`bSupersonic`…) est vrai. En l'air : `Samples − OnGround − OnWall`.
  - `Powerslides` : les dérapages commencés (`bPowersliding` qui passe à vrai).
  - `Pickups` : le jeu n'envoie aucun ramassage de boost. Chaque hausse du `Boost` d'une mise à jour à la suivante en compte un. Deux pastilles dans la même mise à jour n'en font qu'une : c'est une estimation. Sur un vrai match du 27 novembre 2026, 118 et 106 pour 119 et 104 au tableau du jeu. Après le coup d'envoi (33 de boost pour chacun) ou une démolition, la hausse suivante ne compte pas.
  - `BigPickups` : les hausses de plus de 12, le don d'une petite pastille. `BoostCollected` : la somme des hausses.

Au niveau du match, `state.Game` porte deux champs de plus, en dernier, dans le tableau de fin comme dans un départ avant la fin (`left`) :

```json
"Game":{"PlaylistId":11,"Teams":[…],"bHasWinner":true,"bReplay":false,"Winner":"Blue","TimeSeconds":0,"bOvertime":false,
        "Arena":"Stadium_P","StatfeedLabels":{"Save":"Arrêt","Demolish":"Démolition"}}
```

- `Arena` : le nom de la carte (`Game.Arena` de `UpdateState`), s'il fait de 1 à 64 caractères parmi `A-Z`, `a-z`, `0-9` et `_`. Absent sinon.
- `StatfeedLabels` : l'intitulé déjà traduit par le jeu (`Type`) de chaque nom compté dans `Statfeed` ou `StatfeedSecondary`, pour afficher un événement nouveau sans liste en dur. L'intitulé est retiré de ses espaces au début et à la fin, et gardé s'il fait de 1 à 64 caractères sans caractère de contrôle. Sinon le nom reste sans intitulé jusqu'à un événement du même nom qui en porte un lisible ; l'app affiche alors `EventName`. Absent si aucun événement n'est compté.

Taille du corps : l'API refuse un corps de plus de 64 000 caractères (`413`), et le match serait perdu. Si le corps dépasse cette limite, le collecteur retire `Statfeed`, `StatfeedSecondary` et `AssistsTo` de chaque joueur et départ, et `StatfeedLabels` du match ; les autres champs restent. Un vrai match n'y arrive pas : seize joueurs avec tout au maximum (30 noms donnés et subis, des compteurs à trois chiffres) font environ 51 000 caractères. Seuls des noms et identifiants de 128 caractères, tous échappés, dépassent la limite.

Écarté pour l'instant :

- Touches et buts aériens : `BallHit` ne donne que la position du ballon, pas celle de la voiture. Une hauteur seuil compterait aussi les touchers au mur ou au plafond et les simples sauts, et dépendrait du terrain. Si le jeu envoie un événement de but aérien dans son fil de stats, `Statfeed` le compte déjà.
- Possession (`Game.Ball.TeamNum` de `UpdateState`) : un échantillon à la fréquence d'envoi choisie, faussé par les ralentis, les comptes à rebours et les pauses.
- Événements reçus après `MatchEnded` (un éventuel MVP) : la demande est de les ignorer.
- `Loadout` (voiture et objets équipés) : de la personnalisation, pas une stat de jeu.
- Champs SPECTATEUR (boost, vitesse de la voiture, supersonique, `BoostPickup`) : jamais envoyés à un joueur.
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
