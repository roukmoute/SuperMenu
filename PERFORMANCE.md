# Performances de Super Menu 0.6.1

Le binaire local mesuré fait **1 154 560 octets (1,10 Mio)**, contre **1 599 488 octets** pour la version 0.6.0 : **444 928 octets économisés, soit 27,8 %**. Les deux versions ont été reconstruites avec Rust 1.94, MSVC et les mêmes options de runtime C statique. La référence est le commit [`1d68820`](https://github.com/roukmoute/SuperMenu/commit/1d68820f2915c35ba8f84ffb39d034b6b86755df), plutôt qu'une archive compilée ailleurs.

SHA-256 du binaire 0.6.1 mesuré :

```text
298FBD2A13A63367B347B6B9404E2E20F4A4048ADF57A12E92E6ACBF3D29A8D7
```

## Changements retenus

Le profil Release utilise `opt-level = "z"`, LTO complet, une unité de compilation, la suppression des symboles et `panic = "abort"`. Les profils `3`, `s` et `z` ont été comparés. Seuls les deux helpers de classement `score` et `match_token` gardent `inline(always)` pour limiter le coût du profil orienté taille sur le chemin de recherche. L'exécutable n'est pas compressé et n'ajoute aucune dépendance.

Le moteur évite des scans et allocations inutiles pendant le classement. L'interface conserve ses fournisseurs fixes, ses textes traduits, ses clés d'icônes et ses tableaux UTF-16 préparés ; son dessin utilise les ressources GDI de Windows sans recréer une brosse pour chaque rectangle. Le formatage traduit emprunte son modèle au lieu de le recopier. La restauration du cache parse une vue de fichier en lecture seule, et sa sauvegarde sérialise les éléments empruntés sans copier tout l'index.

## Moteur de recherche

Même machine, index de **20 000 éléments**, neuf résultats, cent échauffements puis quatre cents échantillons par scénario. L'ordre des exécutions comparées est **référence / 0.6.1 / 0.6.1 / référence**. Chaque cellule indique la **médiane / p95**, en millisecondes ; les quatre séries sont présentées sans choisir seulement le meilleur passage.

| Scénario | Référence A | 0.6.1 A | 0.6.1 B | Référence B |
| --- | ---: | ---: | ---: | ---: |
| Index générique | 0,391 / 2,872 | 0,518 / 3,869 | 0,391 / 2,394 | 0,447 / 3,005 |
| Fichiers dans l'index mixte | 0,536 / 3,699 | 0,487 / 2,778 | 0,493 / 2,770 | 0,566 / 3,818 |
| Applications et actions de gestion en cache | 0,695 / 1,310 | 0,292 / 0,604 | 0,286 / 0,610 | 0,663 / 0,791 |

La moyenne des deux médianes passe de **0,679 à 0,289 ms** pour les applications (−57,4 %), et de **0,551 à 0,490 ms** pour les fichiers (−11 %). L'index générique passe de **0,419 à 0,455 ms** (+8,5 %) : le compromis taille/vitesse n'accélère donc pas tous les cas. Les essais exploratoires ont aussi montré une variabilité supérieure ; ces chiffres ne garantissent aucune latence absolue. Le diagnostic mesure uniquement le classement en mémoire, hors indexation, accès disque, dessin et activation d'un logiciel.

## Cache

Benchmark Release sur **20 000 entrées**, **4 417 969 octets de JSON**, deux échauffements et seize échantillons par opération. Les données et le format restent identiques.

| Opération | Médiane | p95 |
| --- | ---: | ---: |
| Restaurer par lecteur tamponné | 81,586 ms | 92,541 ms |
| Restaurer par vue de fichier en lecture seule | 47,410 ms | 65,791 ms |
| Sérialiser après copie profonde | 34,300 ms | 51,359 ms |
| Sérialiser les éléments empruntés | 17,636 ms | 23,471 ms |

Sur ce test, la restauration réduit sa médiane de **41,9 %**, la sérialisation de **48,6 %**. La seconde comparaison mesure la préparation du JSON, hors écriture et synchronisation du disque. Le fichier mappé évite une copie supplémentaire du JSON sur le tas ; la sauvegarde évite de doubler temporairement les données de l'index.

## Menu natif

Deux séries voisines par version, trois lancements chacune, puis trente échauffements et trois cents mesures par opération. Fixture isolée de huit éléments, français, thème sombre, DPI 96, fenêtre 720 × 680, Windows 11 build 26200. Les plages ci-dessous couvrent les médianes des deux séries.

| Mesure | Référence | 0.6.1 |
| --- | ---: | ---: |
| Premier dessin, horloge interne | 116,6–157,2 ms | 71,8–96,7 ms |
| Dessin seul | 0,171–0,172 ms | 0,119–0,149 ms |
| Recherche puis dessin | 7,114–9,645 ms | 5,206–8,006 ms |

Les p95 ont varié davantage : l'un des passages 0.6.1 a un p95 de dessin de **2,511 ms**, contre **0,537–0,652 ms** pour la référence. Il serait incorrect d'en déduire une amélioration systématique des délais d'affichage. Les premiers dessins étaient généralement plus rapides dans ces séries, sans garantie de démarrage à froid.

`first_frame_ms` commence dans `ui::run` : il exclut la création du processus, le chargement initial du binaire et la présentation des pixels par DWM. Les lancements répétés utilisent généralement les caches Windows déjà chauds. Les séries incluent les contrôles de propriété des fenêtres, les échanges Win32 synchrones entre processus et `GdiFlush` côté appelant ; `WM_PRINTCLIENT` dessine hors écran et exclut le contrôle EDIT enfant. La mémoire de cette petite fixture ne représente pas celle d'un index réel : aucune comparaison n'est faite avec les anciennes mesures du PC complet.

## Reproduire et vérifier

Les commandes et les limites des outils sont décrites dans [TESTING.md](TESTING.md#mesurer-les-performances). Construire les deux versions avec la même chaîne de compilation, puis conserver les rapports JSON séparément :

```powershell
.\scripts\measure-performance.ps1 -Executable .\target\release\supermenu.exe -Report .\artifacts\performance.json
cargo test --release --locked cache_performance_comparison -- --ignored --nocapture
```

Le rapport natif conserve la taille et l'empreinte du binaire, la fixture, les paramètres et chaque échantillon. `--benchmark` produit les mesures du moteur. Les comparaisons dépendent de la machine, du compilateur, du DPI et de sa charge ; aucun seuil matériel fixe n'est imposé en CI.

Validation fonctionnelle : **174 tests Rust**, **un benchmark de cache facultatif** et **755 assertions Windows** (306 menu, 278 À propos, 171 traductions) passent. Les tests utilisent leurs fichiers, registres et fenêtres isolés ; aucune application personnelle n'a été installée ou désinstallée pour ces mesures.
