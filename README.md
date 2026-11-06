# Big Rocket Camp Direct

Le petit outil Windows qui envoie tes matchs Rocket League à [Big Rocket Camp](https://mousrockets.com).

Tu le lances en même temps que le jeu. À la fin de chaque match, il récupère le tableau final sur ton PC et l'envoie au club : plus besoin de garder un onglet de navigateur ouvert sur la page Direct. Si Internet coupe, les matchs attendent et partent au retour de la connexion.

## Télécharger

[**Télécharger l'installateur**](https://github.com/mousrockets/bigrocketcamp-direct/releases/latest/download/BigRocketCampDirect-setup.exe) (dernière version, Windows 10 ou 11).

Le dépôt est privé pour l'instant : connecte-toi d'abord à GitHub avec un compte membre de l'organisation mousrockets. Toutes les versions sont dans les [releases](https://github.com/mousrockets/bigrocketcamp-direct/releases/latest).

## Installer

1. Ouvre `BigRocketCampDirect-setup.exe`.
2. Windows affiche « Windows a protégé votre ordinateur » : l'outil n'est pas encore signé, c'est normal. Clique sur **Informations complémentaires**, puis **Exécuter quand même**.
3. Suis l'installation. Aucun mot de passe administrateur n'est demandé. Laisse cochée la case qui crée le raccourci sur le bureau.

L'outil s'installe pour ton compte Windows seulement, avec un raccourci « Big Rocket Camp Direct » sur le bureau et dans le menu Démarrer. Pour le désinstaller : Paramètres → Applications.

## Premier lancement

1. **Connecte-toi** avec ton adresse e-mail, puis le code à 6 chiffres reçu. Tu restes connecté ensuite, même après un redémarrage.
2. **Rocket League** : au lancement, l'outil cherche le jeu (Epic Games ou Steam) et active son Stats API. S'il a changé quelque chose, il te le dit : relance alors Rocket League. S'il n'a pas pu (dossier protégé, jeu introuvable), suis les étapes « À régler une seule fois sur le PC » affichées dans la fenêtre.
3. Clique sur **Activer le direct sur ce PC**. L'outil s'en souvient pour les fois suivantes.
4. **Garde la fenêtre ouverte pendant tes parties**, réduite si tu veux, et attends le tableau final avant de quitter un match.

Si la fenêtre affiche une erreur de connexion (PC lancé sans Internet), appuie sur F5.

## Pour les développeurs

Les règles du projet et les commandes sont dans [AGENTS.md](AGENTS.md). Une autre instance de Big Rocket Camp construit son propre outil en définissant `APP_URL` à la compilation (variable de dépôt `APP_URL` en CI).

Licence MIT.
