# Super Menu

Un lanceur et moteur de recherche **natif Windows**, écrit en Rust et Win32. Une fenêtre discrète, un raccourci global, une recherche au clavier. Le projet reprend l'idée des fournisseurs et de la personnalisation de [Walker](https://github.com/abenz1267/walker), avec une implémentation indépendante conçue pour Windows.

![Super Menu](docs/screenshot.png)

Windows 10 et 11, x64. L'exécutable autonome de cette première version fait **914 432 octets, soit 0,87 Mio**.

Le programme utilise les contrôles et API de Windows : aucun navigateur embarqué, aucune installation de .NET, aucun service distant nécessaire pour rechercher les applications ou fichiers. La recherche travaille sur un index en mémoire ; sa reconstruction s'effectue en arrière-plan. Un cache local accélère les démarrages suivants.

Mesures locales de la version Release, avec 337 applications et 20 000 fichiers/dossiers : environ 25 Mio de mémoire privée et 54 Mio de mémoire résidente, bibliothèques Windows comprises. Le benchmark sur 20 000 éléments donne une médiane de 0,57 ms (p95 2,23 ms) pour le classement en mémoire, et 0,90 ms (p95 2,79 ms) pour les fichiers. Ces mesures dépendent de la machine ; le benchmark sépare le classement de l'indexation et de l'activation. Les 23 tests automatisés et les 8 contrôles du menu natif passent.

## Utiliser

Télécharger l'archive depuis les [releases](https://github.com/roukmoute/SuperMenu/releases), l'extraire et lancer l'exécutable. Au premier lancement, le menu apparaît. Ensuite, **Win + Espace** affiche ou masque le menu. Une icône dans la zone de notification permet aussi de l'ouvrir et de quitter le programme.

Windows utilise déjà Win + Espace pour changer la langue de saisie. Super Menu lui donne la priorité lorsqu'il fonctionne. Choisir un autre raccourci dans les réglages si ce changement de langue doit rester accessible. Le fichier JSON permet aussi un raccourci personnalisé, par exemple `Ctrl+Alt+Space`.

| Saisie | Fournisseur |
| --- | --- |
| `firefox` | Applications et résultats correspondants |
| `;` | Menu des fournisseurs |
| `@` | Applications |
| `/` | Fichiers et dossiers indexés |
| `= (15 + 5) * 3` | Calculatrice locale |
| `? rust windows` | Recherche Web dans le navigateur par défaut |
| `>` | Commandes personnalisées et exécution shell explicite |
| `:` | Historique du presse-papiers, si activé |
| `.` | Symboles à copier |
| `~` | Fenêtres ouvertes |
| `!` | Actions système |

Les flèches sélectionnent un résultat et **Entrée** l'active. **Échap** efface la recherche, puis masque le menu si elle est déjà vide. **Tab** ouvre le menu des fournisseurs. **Ctrl + Entrée** ouvre le dossier parent d'un fichier. **F1** affiche l'aide, **Ctrl + ,** ouvre les réglages et **F5** actualise l'index. Une commande shell ne s'exécute qu'après validation par Entrée. Les résultats de calcul, symboles et historique sont copiés dans le presse-papiers.

## Réglages et index

Les réglages se trouvent dans `%LOCALAPPDATA%\SuperMenu\settings.json`. Le fichier [settings.example.json](settings.example.json) présente les champs disponibles : raccourci, thème, couleur d'accent, largeur du menu, nombre de résultats, commandes et dossiers à indexer. Les réglages de la fenêtre sont appliqués avec « Enregistrer ». Après une modification directe du JSON, quitter et relancer Super Menu.

`file_roots` contient uniquement les dossiers choisis. Les variables comme `%USERPROFILE%` sont acceptées. Les notifications natives de Windows surveillent ces racines et les menus Démarrer : une création, suppression ou modification déclenche une reconstruction en arrière-plan, après regroupement des événements rapprochés. Le programme attend les notifications du système quand rien ne change. **F5** permet aussi une actualisation manuelle.

L'indexeur ignore les fichiers système, les jonctions, les liens symboliques et les dossiers `.git`, `node_modules`, `bin` et `obj`. Il refuse les racines de disques, les partages et les lecteurs réseau. Le maximum configuré inclut fichiers et dossiers ; sa limite absolue est de 100 000 éléments. Chaque parcours est aussi limité à 4 096 dossiers, 100 000 entrées examinées, 24 niveaux et un budget de cinq secondes vérifié entre les opérations de disque. Un très grand arbre produit donc un index partiel : choisir des racines ciblées pour inclure les fichiers utiles.

`index.json` conserve localement les chemins des applications et fichiers indexés. `usage.json` conserve des identifiants SHA-256 et compteurs de lancement pour améliorer le classement ; il ne conserve ni requêtes ni texte du presse-papiers. Ces fichiers restent dans le profil Windows de l'utilisateur.

L'historique du presse-papiers est **désactivé par défaut**, limité au texte et conservé uniquement en mémoire. Les textes supérieurs à 32 768 caractères et les éléments marqués comme exclus par Windows sont ignorés. Désactiver l'option ou fermer le programme efface les entrées de Super Menu. Les formats d'exclusion sont ceux documentés par [Microsoft](https://learn.microsoft.com/en-us/windows/win32/dataxchg/clipboard-formats#cloud-clipboard-and-clipboard-history-formats).

## Compiler sous Windows

Installer Rust avec la chaîne **MSVC** et les outils C++ de Visual Studio Build Tools, puis exécuter depuis le dépôt :

```powershell
.\scripts\build.ps1 -Archive
```

Le script vérifie le formatage, Clippy et les tests, compile en Release, lance le benchmark et prépare `artifacts\SuperMenu-windows-x64.zip`. L'archive inclut le rapport `benchmark.txt`, même si l'exécutable graphique n'affiche rien dans la console. Le runtime C est intégré à l'exécutable. La compilation utilise les options Release du dépôt : optimisation, LTO et suppression des symboles de débogage. L'exécutable distribué n'est pas compressé au lancement.

Pour développer et mesurer séparément :

```powershell
cargo test --locked
cargo run --release --locked
cargo run --release --locked -- --benchmark
cargo run --release --locked -- --smoke-test --report smoke.json --snapshot menu.bmp
```

Le benchmark mesure le moteur de recherche sur des catalogues de test ; ses résultats dépendent de la machine et ne constituent pas une garantie de latence du raccourci ou du disque. `--smoke-test` ouvre brièvement le menu pour vérifier son démarrage et peut exporter un rapport ainsi qu'une capture BMP du menu. `--background` démarre sans afficher le menu, utile avec un raccourci placé dans `shell:startup`.

GitHub Actions construit et vérifie le projet sur Windows à chaque pull request et mise à jour de `main`/`master`. Après une compilation réussie sur la branche principale, la version de `Cargo.toml` produit automatiquement une préversion téléchargeable, par exemple `v0.1.0`. Une version déjà publiée conserve son archive ; augmenter la version du paquet pour publier une nouvelle préversion. Un tag `v*` produit une release standard si cette version n'existe pas encore. Le code est sous [licence MIT](LICENSE) ; aucune portion du code de Walker n'a été copiée.
