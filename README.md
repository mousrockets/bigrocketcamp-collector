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

Attends le tableau final avant de quitter un match : sans lui, aucun résultat n'est envoyé.

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

- Le corps est exactement celui que le pont web envoie aujourd'hui, champ pour champ et dans le même ordre (voir `src/capture.rs`, porté de `src/lib/live-stats.ts`).
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
