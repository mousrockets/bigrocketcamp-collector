# AGENTS.md

Règles pour tout agent IA qui travaille sur ce dépôt. Elles valent aussi pour les humains.

## Ce que fait ce dépôt

Big Rocket Camp Direct est une petite app Windows (Tauri 2, Rust et WebView2). Elle ouvre une seule fenêtre sur la page `/direct` de l'app web Big Rocket Camp, qui lit le Stats API local de Rocket League (`ws://localhost:49124/`) et envoie les tableaux de fin de match à `/api/live` avec le cookie de session du joueur.

- L'app ne fait rien d'autre que montrer cette page, garder sa session (profil WebView2) et régler une fois le Stats API de Rocket League.
- La logique du direct reste dans l'app web (`mousrockets/bigrocketcamp`) : une correction du direct se fait là-bas, pas ici.
- L'adresse de l'instance est fixée à la compilation : variable `APP_URL`, par défaut `https://mousrockets.com`. En CI, c'est la variable de dépôt `APP_URL`.

## Avec qui tu travailles

- Un contributeur débutant : c'est toi qui prends les décisions techniques, dans le cadre de ce fichier et de la doc. Demande-lui ce qu'il veut obtenir, jamais de choisir entre des options techniques, et explique ce que tu as fait avec des mots simples.
- Un contributeur développeur : donne-lui le détail technique, présente les options et leurs compromis, et laisse-le décider.
- Le vocabulaire et les questions de la personne te disent à qui tu parles. Dans le doute, demande-lui.

## Langues

- Anglais : code, noms, commentaires, commits, branches.
- Français : issues, PR (titre et description), docs, messages affichés au joueur, et tes réponses.

## Commandes

- `npm install` : installe la CLI Tauri et Biome (Node, voir `.node-version`). Rust stable via rustup.
- `npm run dev` : lance l'app (sous Windows ; sur Mac, la fenêtre s'ouvre mais le jeu n'y tourne pas).
- `npm run check` : Biome, `cargo fmt`, `cargo clippy` et `cargo test`. Doit passer avant toute PR.
- `npm run fmt` : formate le code.
- `npm run build` : construit l'installateur (sous Windows seulement).
- `npm run icons` : régénère les icônes depuis `assets/logo.png`. Ne garde ensuite que celles listées dans `src-tauri/tauri.conf.json`.

## Workflow

1. Pars toujours du dernier `main`.
2. Une demande = une issue = une branche = une PR. Propose de découper une demande qui mélange plusieurs sujets.
3. Issue : contexte, résultat attendu, critères d'acceptation, assignée à la personne qui la demande. Si tu ne peux pas créer l'issue, mets ces points dans la PR.
4. Branche courte en anglais : `feat/…`, `fix/…`, `docs/…`, `refactor/…`, `chore/…`.
5. Change le minimum utile. Un comportement nouveau arrive avec son test ; un bug corrigé, avec le test qui l'aurait détecté.
6. Commits au format Conventional Commits (`fix: keep the session after an update`).
7. PR : titre et description en français (quoi, pourquoi, comment vérifier), et `Closes #N`.
8. Un mainteneur fusionne la PR quand la CI est verte.

## Publier une version

1. Monte la version dans `src-tauri/Cargo.toml` (l'app et l'installateur la reprennent), par PR.
2. Après la fusion, un mainteneur pousse le tag `vX.Y.Z` sur `main`.
3. La CI vérifie que le tag correspond à la version, construit l'installateur et le publie dans la release du tag (`BigRocketCampDirect-setup.exe`).

## Stack

- Rust stable, édition 2024, dans `src-tauri`. Tauri 2 et ses plugins officiels uniquement.
- Pas de JavaScript : l'interface est la page de l'app web. Les fichiers JSON passent par Biome.
- La CLI npm `@tauri-apps/cli` et la crate `tauri` restent sur la même version mineure.
- Toujours les versions stables les plus récentes. Dependabot (npm, cargo, GitHub Actions) ouvre les PR de mise à jour ; pas de Renovate.
- CI : GitHub Actions officielles, épinglées par commit, et la CLI `gh`. Aucune action tierce.

## Sécurité

- La fenêtre ne navigue que sur l'origine de `APP_URL` ; tout autre lien s'ouvre dans le navigateur habituel.
- La page distante n'a accès à aucune commande Tauri : pas de capability, pas de `withGlobalTauri`. Ne pas en ajouter sans validation d'un mainteneur.
- L'app ne stocke aucun jeton : la session est le cookie du site, gardé dans le profil WebView2.
- Les arguments WebView2 (`BROWSER_ARGS`) gardent les contrôles d'accès au réseau local désactivés, pour que la page joigne le jeu sans invite.
- Le réglage de Rocket League ne touche que les clés du Stats API, après une copie `.bak` du fichier d'origine.

## Validation d'un mainteneur

Seuls `.github/`, `src-tauri/tauri.conf.json` et ce fichier l'exigent (`.github/CODEOWNERS`).

## Jamais

- Un secret, un jeton ou une donnée réelle de joueur dans le code, les tests, les logs ou une PR. Les exemples utilisent des joueurs fictifs.
- Une donnée propre à une équipe dans le code, en dehors de l'adresse par défaut de l'instance.
- Contourner une protection : anti-bot, CAPTCHA, limite de requêtes, anti-triche du jeu.
- Rendre public le dépôt ou une release sans l'accord de Dimitri.
- Pousser sur `main`, réécrire l'historique, désactiver un test pour faire passer la CI.

## Code

Lisible sans commentaire : noms explicites, fonctions courtes. Un commentaire dit pourquoi, jamais quoi.
