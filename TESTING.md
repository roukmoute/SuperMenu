# Tester Super Menu sous Windows

Les tests sont versionnés avec le projet. La même commande s'utilise sur un PC de développement et dans GitHub Actions :

```powershell
.\scripts\test.ps1
```

Prérequis : Windows x64, PowerShell 7, Rust MSVC avec `rustfmt` et `clippy`, et les outils C++ de Visual Studio Build Tools. Le script vérifie le formatage et Clippy, exécute les tests Rust, compile le binaire Release, puis teste les vrais contrôles Win32 du programme. Une erreur d'assertion fait échouer la commande.

Les tests des pictogrammes vérifient aussi les pixels rendus hors écran : neuf symboles de l’accueil, centrés à un pixel près, aux échelles Windows 100 %, 125 %, 150 % et 200 %. Ils contrôlent les contours asymétriques, les rectangles invalides et la restauration des paramètres GDI.

## Couverture

| Suite | Comportements vérifiés |
| --- | --- |
| `cargo test --locked` | Classement et normalisation de recherche, indexation et cache, configuration Steam facultative, déduplication des jeux et exclusion du cache, couverture des 236 entrées WinUtil, éditions et architectures distinctes, commandes Microsoft Store simulées, preuve d'absence de Chrome malgré une inscription restante, réinstallation ciblée et confirmation de l'exécutable, plans de nettoyage et préservation des données, minuteurs, état des lancements, icônes, collecte système et contrôles natifs isolés. |
| `pwsh scripts/update-package-catalog.ps1 -Check` | Hash et taille du catalogue importé, régénération déterministe des identifiants et des sources, sans téléchargement ni installation. Également exécuté par les scripts de test et de distribution. |
| `tests/windows/launcher.ps1` | Recherche sur un index fixe, applications par défaut et fichiers explicites, installation de Chrome absent avec ou sans préfixe, absence de résultats Telegram lors d'une recherche Steam, maintien de la sélection après plusieurs notifications d’index (accueil défilé et recherche), vraies peintures WM_PAINT et redimensionnements avec texte/sélection EDIT préservés et ressources GDI/USER bornées, réinitialisation lors d’une nouvelle requête, clavier du menu, préfixe et curseur, suppression de mots/sélection avec Ctrl+Retour arrière et annulation. |
| `tests/windows/translations.ps1` | Menus et fenêtre À propos en anglais, identité inchangée des applications et paquets, vrai sélecteur français/anglais, réutilisation du cache sans réécriture. |
| `tests/windows/about.ps1` | Ouverture et contenu de la fenêtre À propos dans les deux thèmes, version et compteurs, fermetures par Échap/croix/bouton, réouverture de la même fenêtre, redimensionnement, défilement, boutons restant accessibles et ressources GDI/USER bornées. |
| `tests/windows/helper-console.ps1` | Fermeture automatique après une erreur d'installation ou de désinstallation, code de sortie conservé, rapport local, diagnostic sans attente et absence de sélection rapide bloquante. Les requêtes de test échouent avant tout appel à WinGet. |
| `tests/windows/setup.ps1` | Vraie installation NSIS, raccourcis et inscription Windows, options facultatives, mise à jour pendant que l'instance de test tourne, refus des processus inconnus, désinstallation et conservation des données. Chaque installateur utilise un nom et un dossier de test isolés. |

Les suites Windows conduisent l'exécutable construit, observent ses fenêtres et ses résultats réels, et échouent sur une régression. Elles ne recalculent pas le classement dans le script. Elles vérifient aussi les noms et résultats de tous les contrôles `--smoke-test` ; un simple code de sortie nul ne suffit pas.

Le test natif de l'icône de notification vérifie son rendu réel avec `DrawIconEx` à 16, 20, 24 et 32 pixels : fond transparent sur thèmes clair et sombre, grille centrée, masque de transparence et libération des ressources. Pour exporter ses rendus BMP, définir facultativement `SUPER_MENU_ICON_TEST_DIR` avant `cargo test` ; cette instrumentation reste absente du binaire distribué.

Le retour de lancement est également dessiné dans un bitmap natif : le test vérifie le centrage et la transparence de l'icône d'application dans les deux thèmes, sa propriété indépendante du cache, son remplacement sans prolongation et l'absence de réapparition après expiration. `SUPER_MENU_LAUNCH_TEST_DIR` permet d'exporter facultativement ces rendus pendant les tests.

Les tests du suivi de désinstallation utilisent des processus PowerShell fictifs et un faux navigateur GUI compilé pour la fixture : un lanceur se termine avant son enfant différé, qui doit écrire son marqueur avant que la commande rende la main. Ils vérifient aussi le code du lanceur, l'absence d'interférence avec un processus extérieur et l'exclusion du seul navigateur validé, sans le fermer. Le test de l'alias WinGet exécute uniquement --version. Le nettoyage StreamFab utilise un dossier de fixture, avec son nom daté, des binaires et des fichiers personnels : seuls les binaires attribuables deviennent candidats. Aucun StreamFab réel n'est lancé, arrêté ou désinstallé par ces tests.

## Données et isolation

`tests/fixtures/launcher.json` contient des applications et fichiers fictifs. `--smoke-test --smoke-fixture <fichier>` restaure leurs données de recherche et remplace l'indexation du PC. Le mode utilise les réglages par défaut et un historique de lancement vide. Il ne lit ni n'écrit l'index ou la configuration personnelle, n'installe pas de raccourci global et ne lance pas de surveillance de dossiers. Les activations de logiciels, fichiers, liens web, presse-papiers et actions système externes sont bloquées dans ce mode ; les tests utilisent les actions internes du menu.

Chaque processus et dossier temporaire appartient à son test. Le support vérifie le PID de chaque fenêtre avant de la contrôler et borne les délais des messages Windows. Il ne ferme pas une instance de Super Menu déjà ouverte. Aucun logiciel utilisateur n'est installé ou désinstallé. Certains tests Rust créent des fichiers ou clés de registre jetables, et vérifient la Corbeille avec leurs propres fichiers ; ils nettoient uniquement ces ressources.

Les tests natifs ne requièrent pas de clavier physique, de focus du bureau, de carte graphique particulière ou de nombre précis d'applications installées. Les informations système disponibles varient sur une machine virtuelle : une valeur `Indisponible` est acceptée. La mémoire totale et les résolutions ne sont pas figées dans les assertions.

## Rapports et diagnostics

Les rapports JSON se trouvent dans `artifacts/test-results/` et sont conservés comme artefact **SuperMenu-test-results** dans GitHub Actions, même après un échec. Ils détaillent les assertions, les résultats observés et les diagnostics. Ce dossier est ignoré par Git.

Pour exporter également des captures des fenêtres de test sur un bureau Windows interactif :

```powershell
.\scripts\test.ps1 -Capture
```

Les captures sont des aides au diagnostic. La CI ne compare pas les pixels, les polices ou les performances à un seuil dépendant du matériel. Un rapport de benchmark est produit séparément par `scripts/build.ps1`.

Pour reprendre un binaire construit depuis ce checkout, sans répéter les vérifications Rust :

```powershell
.\scripts\test.ps1 -Executable .\target\release\supermenu.exe -SkipRustChecks
```

On peut aussi exécuter séparément une suite :

```powershell
.\tests\windows\launcher.ps1 -Executable .\target\release\supermenu.exe
.\tests\windows\about.ps1 -Executable .\target\release\supermenu.exe
.\tests\windows\translations.ps1 -Executable .\target\release\supermenu.exe
.\tests\windows\helper-console.ps1 -Executable .\target\release\supermenu.exe
.\tests\windows\setup.ps1 -Executable .\target\release\supermenu.exe
```

Un ancien exécutable publié peut ne pas contenir les diagnostics ajoutés depuis sa publication : compiler le checkout testé. `--smoke-fixture`, le message de snapshot des résultats et le diagnostic Ctrl+Retour arrière sont réservés au mode de test. Le diagnostic de clavier modifie seulement l'état de la file d'entrée du thread UI de ce processus, puis le restaure ; il ne simule pas une touche globale.

## Mesurer les performances

Les mesures de latence sont facultatives et séparées des assertions fonctionnelles. Pour comparer deux exécutables sur le même PC :

```powershell
.\scripts\measure-performance.ps1 -Executable .\target\release\supermenu.exe -Report .\artifacts\performance.json
```

Le paramètre facultatif `-WindowPaint` mesure le vrai `WM_PAINT` parent via `RedrawWindow`, tampon et copie vers la fenêtre compris. Le mode par défaut conserve le diagnostic hors écran `WM_PRINTCLIENT`. Les deux modes excluent le dessin du contrôle EDIT enfant et la présentation DWM ; le rapport précise le mode utilisé.

```powershell
.\scripts\measure-performance.ps1 -Executable .\target\release\supermenu.exe -WindowPaint -Theme light -Report .\artifacts\window-paint.json
```

Le script utilise `tests/fixtures/launcher.json`, des cibles fictives `fixture://` et le même support de fenêtres isolées que les tests natifs. Il lance neuf processus successifs, puis mesure le dessin seul et la recherche suivie du dessin dans le dernier processus : trente échauffements et trois cents échantillons pour chaque série. `-StartupRuns`, `-WarmupRounds`, `-Rounds`, `-Theme`, `-Fixture` et `-Queries` permettent de reproduire une autre configuration ; une fixture est limitée à 256 entrées fictives. Les requêtes par défaut sont ASCII ; `tests/fixtures/launcher-accents.json` ajoute douze titres accentués, grecs et cyrilliques à la fixture standard, et `-Queries '@ éditeur','@ caméra','@ télé','@ désinst','@ ångs','@ no-match','/ vidéo','@ é'` mesure le chemin de normalisation non ASCII. La première requête de contrôle reste `@ steam`, donc la fixture doit conserver les entrées Steam. Chaque HWND est vérifié par son PID avant l'envoi d'un message. Aucun logiciel n'est activé, installé ou désinstallé, et aucun téléchargement de langue n'est demandé.

Le chronométrage des séries se fait dans une boucle C#, hors du coût de dispatch de PowerShell. `WM_SETTEXT` traverse le vrai contrôle EDIT et actualise les résultats ; `WM_PRINTCLIENT` dessine le menu dans un bitmap hors écran réutilisé, suivi d'un `GdiFlush` côté appelant. Un snapshot vérifie aussi la requête réellement traitée. Le rapport JSON contient la taille et le SHA-256 du binaire, l'empreinte de la fixture, le thème, la langue, le DPI, les dimensions, les échantillons, leurs médianes et p95, la mémoire privée et résidente et les compteurs GDI/USER.

Pour interpréter le rapport :

- `first_frame_ms` vient de l'horloge interne démarrée dans `ui::run`, jusqu'au premier `WM_PAINT` terminé. Il exclut la création du processus, le chargement initial du binaire et la présentation des pixels par DWM.
- Les lancements répétés utilisent généralement les caches de fichiers et DLL de Windows déjà chauds. Leur état n'est pas contrôlé ; ce n'est pas une mesure de démarrage à froid du PC.
- Les séries du menu incluent les contrôles de propriété des fenêtres et les messages Win32 synchrones entre processus. Le dessin hors écran exclut le contrôle EDIT enfant et la présentation à l'écran par DWM.
- La petite fixture mesure le travail du menu. Le diagnostic `--benchmark` mesure séparément le classement sur un index de 20 000 éléments ; il ne mesure pas l'ouverture d'un logiciel ou les accès disque.

Comparer des exécutables avec la même fixture, les mêmes options, le même DPI et des conditions machine proches. Les outils n'imposent aucun seuil arbitraire de performance et ne sont pas exécutés systématiquement par la CI.

Un benchmark supplémentaire mesure la restauration et la sérialisation du cache sur 20 000 entrées. Il compare la lecture JSON via lecteur tamponné à la vue de fichier en lecture seule, puis la sérialisation d'une copie profonde à celle d'éléments empruntés :

```powershell
cargo test --release --locked cache_performance_comparison -- --ignored --nocapture
```

Ce test marqué `#[ignore]` utilise un fichier temporaire, deux échauffements et seize échantillons par opération. Il est absent des exécutions ordinaires de `cargo test`, et son code n'est pas inclus dans l'exécutable distribué. Ses chiffres restent sensibles au compilateur, au profil de compilation, à la machine et aux caches Windows. Il ne remplace pas les tests fonctionnels du format, de la compatibilité et de la libération du cache.

## Ajouter une régression

Pour un bug du moteur, ajouter un test Rust près du module concerné avec le cas qui échouait. Pour un bug d'interaction, ajouter une assertion dans la suite Windows appropriée et une donnée dans la fixture si nécessaire. Vérifier que le test échoue lorsque le comportement est volontairement régressé, puis restaurer le code. Garder les tests indépendants du profil Windows, des applications réelles et du temps d'exécution du PC.

Les tests de traduction copient le pack anglais du dépôt dans leur cache temporaire. Ils ne contactent pas Internet. Les tests Rust valident aussi le vrai pack JSON, les paramètres de texte, les limites, la sauvegarde atomique, la récupération après erreur et l’absence de téléchargement quand le cache est valide. `--smoke-language` et `--smoke-language-dir` nécessitent une fixture ; le second désigne un cache absolu isolé. `--smoke-first-run-language` vérifie le sélecteur avant l’affichage du menu, sans enregistrer de réglages personnels.

Pour vérifier facultativement le téléchargement réel du pack depuis la version publiée sur `main` (accès Internet nécessaire, jamais exécuté par la CI) :

```powershell
.\tests\windows\translations.ps1 -Executable .\target\release\supermenu.exe -Download
```
