# AGENTS.md

Règles pour tout agent IA qui travaille sur ce dépôt. Elles valent aussi pour les humains.

## Ce que fait ce dépôt

Big Rocket Camp Collector est une petite app Windows native en Rust, sans WebView. Elle lit le Stats API local de Rocket League (`ws://localhost:49124/`), garde chaque tableau de fin de match et l'envoie à `/api/live` avec un jeton obtenu par association. Le contrat d'API est dans la section « API attendue » du README.

- `src/capture.rs` et `src/bridge.rs` sont des portages de `src/lib/live-stats.ts` et `src/lib/live-bridge.ts` de l'app web (`mousrockets/bigrocketcamp`). Le corps envoyé doit rester identique à celui du pont web : un changement de règle se fait des deux côtés, avec les mêmes cas de test.
- `src/rocket_league.rs` trouve le jeu (Epic, Steam) et règle son Stats API.
- `src/win/` contient tout le code Windows : fenêtre, icône de notification, DPAPI, registre.
- `src/update.rs` trouve une nouvelle release sur GitHub et vérifie son installateur ; la mise à jour ne se lance qu'au clic du joueur, et jamais pendant un match.
- L'adresse de l'instance est fixée à la compilation : variable `APP_URL`, par défaut `https://mousrockets.com`. En CI, c'est la variable de dépôt `APP_URL`.

## Avec qui tu travailles

- Un contributeur débutant : c'est toi qui prends les décisions techniques, dans le cadre de ce fichier et de la doc. Demande-lui ce qu'il veut obtenir, jamais de choisir entre des options techniques, et explique ce que tu as fait avec des mots simples.
- Un contributeur développeur : donne-lui le détail technique, présente les options et leurs compromis, et laisse-le décider.
- Le vocabulaire et les questions de la personne te disent à qui tu parles. Dans le doute, demande-lui.

## Langues

- Anglais : code, noms, commentaires, commits, branches.
- Français : issues, PR (titre et description), docs, textes affichés au joueur, et tes réponses.

## Commandes

- `cargo test` : les tests (capture, file, réglage du jeu, état affiché). Ils tournent aussi sur Mac et Linux.
- `cargo fmt` puis `cargo clippy --all-targets -- -D warnings` : doivent passer avant toute PR.
- `cargo build --release` : construit le collecteur (sous Windows). L'installateur se construit avec Inno Setup (`installer/collector.iss`), présent sur les runners Windows de GitHub : voir `.github/workflows/ci.yml`.
- Depuis un Mac, `cargo clippy --target x86_64-pc-windows-msvc` vérifie le code Windows ; il demande un `llvm-rc` dans le `PATH` pour les ressources.

## Workflow

1. Pars toujours du dernier `main`.
2. Une demande = une issue = une branche = une PR. Propose de découper une demande qui mélange plusieurs sujets.
3. Issue : contexte, résultat attendu, critères d'acceptation, assignée à la personne qui la demande. Si tu ne peux pas créer l'issue, mets ces points dans la PR.
4. Branche courte en anglais : `feat/…`, `fix/…`, `docs/…`, `refactor/…`, `chore/…`.
5. Change le minimum utile. Un comportement nouveau arrive avec son test ; un bug corrigé, avec le test qui l'aurait détecté.
6. Commits au format Conventional Commits (`fix: keep the queue after a crash`).
7. PR : titre et description en français (quoi, pourquoi, comment vérifier), et `Closes #N`.
8. Un mainteneur fusionne la PR quand la CI est verte.

## Publier une version

1. Monte `version` dans `Cargo.toml` (semver : `X.Y.Z`), par PR. `Cargo.lock` suit avec `cargo check`.
2. Après la fusion et une CI verte sur `main`, pousse le tag : `git tag vX.Y.Z origin/main && git push origin vX.Y.Z`.
3. La CI vérifie que le tag correspond à `Cargo.toml`, construit l'installateur, génère `SHA256SUMS` et publie la release `vX.Y.Z` avec ces deux fichiers. Vérifie-le avec `gh release view vX.Y.Z`.
4. Ne remplace jamais les fichiers d'une release publiée : publie une nouvelle version.

Les collecteurs installés proposent la mise à jour d'eux-mêmes : ils lisent `releases/latest` de ce dépôt (`repository` dans `Cargo.toml`), et n'installent qu'un fichier dont l'empreinte correspond à `SHA256SUMS`.

## Stack

- Rust stable, édition 2024. Dépendances minimales : `windows-sys` (Win32), `tungstenite` (WebSocket local), `ureq` avec le TLS de Windows, `serde_json`.
- Ni WebView, ni runtime asynchrone, ni JavaScript.
- Les icônes viennent de `assets/logo.png` et sont rangées dans `assets/icons` ; les identifiants de ressources sont dans `build.rs` et `src/win/ui.rs`.
- Toujours les versions stables les plus récentes. Dependabot (cargo, GitHub Actions) ouvre les PR de mise à jour ; pas de Renovate.
- CI : GitHub Actions officielles, épinglées par commit, et la CLI `gh`. Aucune action tierce.

## Performances

Les joueurs jouent à haut niveau : le collecteur ne doit jamais se faire sentir.

- Rien ne tourne en boucle : chaque thread attend un message (socket, canal, file de messages Windows) ou dort.
- Priorité basse et mode efficacité au démarrage (`stay_out_of_the_way`).
- Écriture sur le disque seulement quand un match entre dans la file ou en sort.
- Un seul envoi réseau par fin de match, plus les nouveaux essais en cas d'échec.
- Toute nouvelle dépendance se justifie dans la PR, avec la taille du binaire avant et après (affichée par la CI).

## Sécurité et données

- Le jeton est chiffré avec DPAPI, jamais écrit en clair, jamais affiché ni journalisé.
- Le collecteur ne journalise rien.
- Le réglage de Rocket League ne touche que les trois clés du Stats API, après une copie `.bak` du fichier d'origine.
- La désinstallation supprime le jeton, la file, les réglages et l'entrée de démarrage.

## Validation d'un mainteneur

Seuls `.github/`, `installer/`, `assets/collector.manifest` et ce fichier l'exigent (`.github/CODEOWNERS`).

## Jamais

- Un secret, un jeton ou une donnée réelle de joueur dans le code, les tests, les logs ou une PR. Les exemples utilisent des joueurs fictifs.
- Une donnée propre à une équipe dans le code, en dehors de l'adresse par défaut de l'instance.
- Contourner une protection : anti-bot, CAPTCHA, limite de requêtes, anti-triche du jeu.
- Pousser sur `main`, réécrire l'historique, désactiver un test pour faire passer la CI.

## Code

Lisible sans commentaire : noms explicites, fonctions courtes. Un commentaire dit pourquoi, jamais quoi.
