# Tester Super Menu sous Windows

Les tests sont versionnés avec le projet. La même commande s'utilise sur un PC de développement et dans GitHub Actions :

```powershell
.\scripts\test.ps1
```

Prérequis : Windows x64, PowerShell 7, Rust MSVC avec `rustfmt` et `clippy`, et les outils C++ de Visual Studio Build Tools. Le script vérifie le formatage et Clippy, exécute les tests Rust, compile le binaire Release, puis teste les vrais contrôles Win32 du programme. Une erreur d'assertion fait échouer la commande.

## Couverture

| Suite | Comportements vérifiés |
| --- | --- |
| `cargo test --locked` | Classement et normalisation de recherche, indexation et cache, configuration, catalogue et correspondances de paquets, plans de nettoyage et préservation des données, minuteurs, état des lancements, icônes, collecte système et contrôles natifs isolés. |
| `tests/windows/launcher.ps1` | Recherche sur un index fixe, applications par défaut et fichiers explicites, absence de résultats Telegram lors d'une recherche Steam, clavier du menu, préfixe et curseur, suppression de mots/sélection avec Ctrl+Retour arrière et annulation. |
| `tests/windows/about.ps1` | Ouverture et contenu de la fenêtre À propos dans les deux thèmes, version et compteurs, fermetures par Échap/croix/bouton, réouverture de la même fenêtre, redimensionnement, défilement, boutons restant accessibles et ressources GDI/USER bornées. |

Les suites Windows conduisent l'exécutable construit, observent ses fenêtres et ses résultats réels, et échouent sur une régression. Elles ne recalculent pas le classement dans le script. Elles vérifient aussi les noms et résultats de tous les contrôles `--smoke-test` ; un simple code de sortie nul ne suffit pas.

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
```

Un ancien exécutable publié peut ne pas contenir les diagnostics ajoutés depuis sa publication : compiler le checkout testé. `--smoke-fixture`, le message de snapshot des résultats et le diagnostic Ctrl+Retour arrière sont réservés au mode de test. Le diagnostic de clavier modifie seulement l'état de la file d'entrée du thread UI de ce processus, puis le restaure ; il ne simule pas une touche globale.

## Ajouter une régression

Pour un bug du moteur, ajouter un test Rust près du module concerné avec le cas qui échouait. Pour un bug d'interaction, ajouter une assertion dans la suite Windows appropriée et une donnée dans la fixture si nécessaire. Vérifier que le test échoue lorsque le comportement est volontairement régressé, puis restaurer le code. Garder les tests indépendants du profil Windows, des applications réelles et du temps d'exécution du PC.
