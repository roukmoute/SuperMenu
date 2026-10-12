# Performances de Super Menu

## Version 0.6.13

Le binaire mesuré fait **1 014 272 octets (0,97 Mio)**, contre **1 210 368 octets** pour la version 0.6.12 reconstruite avec la même chaîne de compilation : **196 096 octets économisés, soit 16,2 %**. Les deux versions utilisent Rust 1.94.1, MSVC, le runtime C statique et le profil Release inchangé (`opt-level = "z"`, LTO complet, une unité de compilation, symboles supprimés, `panic = "abort"`). La référence est le commit [`143f449`](https://github.com/roukmoute/SuperMenu/commit/143f449).

SHA-256 du binaire 0.6.13 mesuré :

```text
A213D90ABDD4599CB9682BDD36142CCEAE9BD8CB06C32A58B7547235B8CCC5B4
```

### Méthode

`cargo bloat` sur une construction Release non dépouillée attribue 333 Kio de `.text` au code de Super Menu, 311 Kio à la bibliothèque standard et moins de 45 Kio aux dépendances ; les 363 Kio de `.rdata` (tables et chaînes) ne sont pas attribuables par symbole après LTO. Le coût marginal de chaque composant a donc été mesuré avec de petits exécutables compilés avec le même profil, contre un exécutable vide de 203 776 octets :

| Composant | Coût isolé | Décision |
| --- | ---: | --- |
| `unicode-normalization` (tables NFD) | 126 Kio | remplacé par Win32 |
| `std::process::Command` | 90 Kio | remplacé par `CreateProcessW` |
| Analyse et affichage des flottants | 36 Kio | conservé : calculatrice |
| `str::to_uppercase` | 17 Kio | conservé : identifiants persistants |
| `serde_json::Value` et `json!` | 11 Kio | remplacé par des structures typées |
| `sha2` | 9 Kio | conservé : chemin SHA-NI |
| Crate `windows` (COM) | 5 Kio | remplacé par des vtables sur `windows-sys` |

### Changements retenus

Chaque changement a été construit et mesuré isolément sur la version 0.6.12, puis relu par trois revues indépendantes (parité de comportement, latence de démarrage et de frappe, robustesse Win32/COM) avant intégration.

| Changement | Octets économisés |
| --- | ---: |
| Décomposition Unicode par `NormalizeString` et `GetStringTypeW` au lieu du crate `unicode-normalization` | 122 880 |
| Console d'aide et WinGet lancés par `CreateProcessW` au lieu de `std::process::Command` | 54 272 |
| Rapports JSON sérialisés par des structures typées au lieu de `json!` | 10 752 |
| `Debug` dérivé seulement pour les tests, corps non générique pour `Item::new`, tris instables sur éléments indistinguables | 3 072 |
| Liaisons COM manuelles sur `windows-sys` au lieu du crate `windows` | 3 072 |

Le total intégré (196 096 octets) dépasse légèrement la somme des mesures isolées, LTO éliminant davantage de code commun une fois les crates `windows-core`, `windows-strings`, `windows-result` et `unicode-normalization` absents du graphe de dépendances. Le graphe d'exécution se limite désormais à `serde`, `serde_json`, `sha2` et `windows-sys` ; `unicode-normalization` reste une dépendance de test pour vérifier la parité.

La normalisation Win32 est identique au crate pour chaque caractère des blocs Latin-1 Supplément, Latin étendu A et B, grec, cyrillique et Latin étendu additionnel, ainsi que pour les titres de la fixture. Les divergences restantes sont documentées par des tests : les marques d'espacement indiennes et birmanes que Windows ne classe ni comme diacritiques ni comme voyelles sont conservées, de même que les marques combinantes ajoutées après Unicode 5.1 et les marques hors plan multilingue de base ; trois caractères reclassés par Unicode sont encore supprimés. Les non-caractères, acceptés par NTFS mais refusés par `NormalizeString`, sont remplacés par U+FFFD avant l'appel. Le chemin ASCII, majoritaire pour les noms d'applications et de fichiers, ne fait ni allocation ni appel Win32. L'exécutable importe désormais `normaliz.dll`, chargée au démarrage comme les autres bibliothèques système.

### Compromis écartés

- Un SHA-256 en Rust pur (−5 120 octets) a été implémenté, relu et rejeté : `sha2` utilise les instructions SHA-NI des processeurs récents, et `Usage::snapshot_for` hache les 20 000 identifiants de l'index sur le thread de l'interface après chaque chargement d'index et chaque lancement. Sur cette machine sans SHA-NI, les deux implémentations sont à égalité (29 ms pour 20 000 identifiants) ; sur un processeur avec SHA-NI, la version logicielle ajouterait de l'ordre de 15 ms.
- Remplacer `to_uppercase` par `to_lowercase` (−16 896 octets) changerait la clé de dédoublonnage des dossiers, dont dérivent les identifiants persistants des fichiers et donc les clés de `usage.json` ; les compteurs d'utilisation existants auraient été perdus.
- Les tables mathématiques et de formatage du runtime C et de la bibliothèque standard (`pow`, `sin`, `log`, affichage et analyse des flottants, environ 40 Kio) servent la calculatrice et restent.
- Les options de l'éditeur de liens (`/OPT:REF,ICF`) sont déjà actives ; réduire l'alignement des sections ou désactiver la relocalisation dynamique romprait l'ASLR ou la compatibilité avec les outils de sécurité, et aucune compression de l'exécutable n'est ajoutée.

### Moteur de recherche

Même machine, index de **20 000 éléments**, neuf résultats, cent échauffements puis quatre cents échantillons par scénario, ordre **référence / 0.6.13 / 0.6.13 / référence**. Chaque cellule indique la **médiane / p95**, en millisecondes.

| Scénario | Référence A | 0.6.13 A | 0.6.13 B | Référence B |
| --- | ---: | ---: | ---: | ---: |
| Index générique | 0,332 / 2,329 | 0,342 / 2,325 | 0,339 / 2,240 | 0,312 / 2,176 |
| Fichiers dans l'index mixte | 0,621 / 2,968 | 0,467 / 3,083 | 0,497 / 2,772 | 0,390 / 2,683 |
| Applications et actions de gestion en cache | 0,519 / 0,990 | 0,501 / 0,973 | 0,500 / 0,906 | 0,493 / 0,895 |

Le classement n'a pas changé : les quatre séries se recouvrent, et les écarts restent dans la variabilité d'une série à l'autre. La normalisation Win32 n'intervient que pour les requêtes et titres non ASCII ; cette fixture n'en contient pas.

### Menu natif

Trois séries par version, en ordre alterné (référence, 0.6.13, 0.6.13, référence, 0.6.13, référence), chacune avec neuf lancements puis trente échauffements et trois cents mesures par opération. Fixture isolée de huit éléments, français, thème sombre, DPI 96, fenêtre 720 × 680, Windows 11 build 26200. Les plages couvrent les médianes des trois séries.

| Mesure | Référence | 0.6.13 |
| --- | ---: | ---: |
| Premier dessin, horloge interne | 64,9–78,4 ms | 63,2–75,9 ms |
| Dessin seul | 0,056–0,062 ms | 0,063–0,070 ms |
| Recherche puis dessin | 3,15–3,32 ms | 3,05–3,78 ms |

La première série de la 0.6.13 est la plus lente des six (recherche puis dessin 3,78 ms, p95 6,13 ms), les deux suivantes les plus rapides (3,05 et 3,09 ms) : aucune des deux versions ne se distingue au-delà du bruit de mesure. La mémoire privée (2,70 Mo), l'ensemble de travail (15,9 Mo) et les compteurs GDI et USER (26 chacun) sont identiques. Comme pour la version 0.6.1, ces chiffres n'indiquent pas une latence garantie et excluent le démarrage à froid.

### Construction nightly facultative

Une part importante du binaire reste la bibliothèque standard précompilée et la machinerie de panique, qu'un exécutable sans console ne peut pas afficher. Rust nightly permet de recompiler la bibliothèque standard avec le profil taille du projet et d'abandonner les paniques sans formater de message. Mesures sur la version 0.6.12, même machine :

| Construction | Taille |
| --- | ---: |
| Stable, profil Release du projet | 1 210 368 octets |
| Nightly, `-Zbuild-std` avec `optimize_for_size` | 999 936 octets |
| Nightly, `-Zbuild-std` et `-Cpanic=immediate-abort` | 966 656 octets |
| Nightly, `-Zbuild-std` avec `optimize_for_size` et `-Cpanic=immediate-abort` | 937 984 octets |

Sur la version 0.6.13, la même recette produit un exécutable de **780 288 octets (0,74 Mio)**, soit 35,5 % de moins que la version 0.6.12 stable. Elle est disponible avec `.\scripts\build.ps1 -Nightly` (toolchain `nightly` et composant `rust-src` installés par rustup) et n'est pas utilisée par l'intégration continue : elle dépend d'options instables, une panique n'écrit plus de diagnostic dans la console d'aide, et la variante `optimize_for_size` de la bibliothèque standard choisit des algorithmes plus compacts, parfois plus lents. Le seul benchmark relevé sur cette construction, exécuté par le script juste après la compilation, donne 0,503 ms de médiane sur l'index générique contre 0,312–0,342 ms pour la construction stable : une mesure isolée et bruitée, mais qui impose de comparer le moteur et le menu avec le protocole ci-dessus avant toute publication avec cette recette. Les tests Windows s'exécutent sur l'exécutable ainsi produit.

### Reproduire et vérifier

Les commandes et leurs limites sont décrites dans [TESTING.md](TESTING.md#mesurer-les-performances). Pour comparer la version 0.6.12 (`143f449`) et la version 0.6.13 avec la même chaîne de compilation :

```powershell
cargo build --release --locked
.\scripts\measure-performance.ps1 -Executable .\target\release\supermenu.exe -Report .\artifacts\performance.json
.\target\release\supermenu.exe --benchmark --report .\artifacts\benchmark.txt
```

Validation fonctionnelle : **223 tests Rust**, dont les tests de parité de la normalisation Unicode, des vecteurs FIPS, du vrai raccourci `.lnk` et de la Corbeille, et **997 assertions Windows** (362 lanceur, 278 À propos, 171 traductions, 40 console d'aide, 146 installateur). Clippy sans avertissement et `rustfmt` sont imposés.

## Version 0.6.1

Le binaire local mesuré fait **1 154 560 octets (1,10 Mio)**, contre **1 599 488 octets** pour la version 0.6.0 : **444 928 octets économisés, soit 27,8 %**. Les deux versions ont été reconstruites avec Rust 1.94, MSVC et les mêmes options de runtime C statique. La référence est le commit [`1d68820`](https://github.com/roukmoute/SuperMenu/commit/1d68820f2915c35ba8f84ffb39d034b6b86755df), plutôt qu'une archive compilée ailleurs.

SHA-256 du binaire 0.6.1 mesuré :

```text
298FBD2A13A63367B347B6B9404E2E20F4A4048ADF57A12E92E6ACBF3D29A8D7
```

### Changements retenus

Le profil Release utilise `opt-level = "z"`, LTO complet, une unité de compilation, la suppression des symboles et `panic = "abort"`. Les profils `3`, `s` et `z` ont été comparés. Seuls les deux helpers de classement `score` et `match_token` gardent `inline(always)` pour limiter le coût du profil orienté taille sur le chemin de recherche. L'exécutable n'est pas compressé et n'ajoute aucune dépendance.

Le moteur évite des scans et allocations inutiles pendant le classement. L'interface conserve ses fournisseurs fixes, ses textes traduits, ses clés d'icônes et ses tableaux UTF-16 préparés ; son dessin utilise les ressources GDI de Windows sans recréer une brosse pour chaque rectangle. Le formatage traduit emprunte son modèle au lieu de le recopier. La restauration du cache parse une vue de fichier en lecture seule, et sa sauvegarde sérialise les éléments empruntés sans copier tout l'index.

### Moteur de recherche

Même machine, index de **20 000 éléments**, neuf résultats, cent échauffements puis quatre cents échantillons par scénario. L'ordre des exécutions comparées est **référence / 0.6.1 / 0.6.1 / référence**. Chaque cellule indique la **médiane / p95**, en millisecondes ; les quatre séries sont présentées sans choisir seulement le meilleur passage.

| Scénario | Référence A | 0.6.1 A | 0.6.1 B | Référence B |
| --- | ---: | ---: | ---: | ---: |
| Index générique | 0,391 / 2,872 | 0,518 / 3,869 | 0,391 / 2,394 | 0,447 / 3,005 |
| Fichiers dans l'index mixte | 0,536 / 3,699 | 0,487 / 2,778 | 0,493 / 2,770 | 0,566 / 3,818 |
| Applications et actions de gestion en cache | 0,695 / 1,310 | 0,292 / 0,604 | 0,286 / 0,610 | 0,663 / 0,791 |

La moyenne des deux médianes passe de **0,679 à 0,289 ms** pour les applications (−57,4 %), et de **0,551 à 0,490 ms** pour les fichiers (−11 %). L'index générique passe de **0,419 à 0,455 ms** (+8,5 %) : le compromis taille/vitesse n'accélère donc pas tous les cas. Les essais exploratoires ont aussi montré une variabilité supérieure ; ces chiffres ne garantissent aucune latence absolue. Le diagnostic mesure uniquement le classement en mémoire, hors indexation, accès disque, dessin et activation d'un logiciel.

### Cache

Benchmark Release sur **20 000 entrées**, **4 417 969 octets de JSON**, deux échauffements et seize échantillons par opération. Les données et le format restent identiques.

| Opération | Médiane | p95 |
| --- | ---: | ---: |
| Restaurer par lecteur tamponné | 81,586 ms | 92,541 ms |
| Restaurer par vue de fichier en lecture seule | 47,410 ms | 65,791 ms |
| Sérialiser après copie profonde | 34,300 ms | 51,359 ms |
| Sérialiser les éléments empruntés | 17,636 ms | 23,471 ms |

Sur ce test, la restauration réduit sa médiane de **41,9 %**, la sérialisation de **48,6 %**. La seconde comparaison mesure la préparation du JSON, hors écriture et synchronisation du disque. Le fichier mappé évite une copie supplémentaire du JSON sur le tas ; la sauvegarde évite de doubler temporairement les données de l'index.

### Menu natif

Deux séries voisines par version, trois lancements chacune, puis trente échauffements et trois cents mesures par opération. Fixture isolée de huit éléments, français, thème sombre, DPI 96, fenêtre 720 × 680, Windows 11 build 26200. Les plages ci-dessous couvrent les médianes des deux séries.

| Mesure | Référence | 0.6.1 |
| --- | ---: | ---: |
| Premier dessin, horloge interne | 116,6–157,2 ms | 71,8–96,7 ms |
| Dessin seul | 0,171–0,172 ms | 0,119–0,149 ms |
| Recherche puis dessin | 7,114–9,645 ms | 5,206–8,006 ms |

Les p95 ont varié davantage : l'un des passages 0.6.1 a un p95 de dessin de **2,511 ms**, contre **0,537–0,652 ms** pour la référence. Il serait incorrect d'en déduire une amélioration systématique des délais d'affichage. Les premiers dessins étaient généralement plus rapides dans ces séries, sans garantie de démarrage à froid.

`first_frame_ms` commence dans `ui::run` : il exclut la création du processus, le chargement initial du binaire et la présentation des pixels par DWM. Les lancements répétés utilisent généralement les caches Windows déjà chauds. Les séries incluent les contrôles de propriété des fenêtres, les échanges Win32 synchrones entre processus et `GdiFlush` côté appelant ; `WM_PRINTCLIENT` dessine hors écran et exclut le contrôle EDIT enfant. La mémoire de cette petite fixture ne représente pas celle d'un index réel : aucune comparaison n'est faite avec les anciennes mesures du PC complet.

### Reproduire et vérifier

Les commandes et les limites des outils sont décrites dans [TESTING.md](TESTING.md#mesurer-les-performances). Construire les deux versions avec la même chaîne de compilation, puis conserver les rapports JSON séparément :

```powershell
.\scripts\measure-performance.ps1 -Executable .\target\release\supermenu.exe -Report .\artifacts\performance.json
cargo test --release --locked cache_performance_comparison -- --ignored --nocapture
```

Le rapport natif conserve la taille et l'empreinte du binaire, la fixture, les paramètres et chaque échantillon. `--benchmark` produit les mesures du moteur. Les comparaisons dépendent de la machine, du compilateur, du DPI et de sa charge ; aucun seuil matériel fixe n'est imposé en CI.

Validation fonctionnelle : **174 tests Rust**, **un benchmark de cache facultatif** et **755 assertions Windows** (306 menu, 278 À propos, 171 traductions) passent. Les tests utilisent leurs fichiers, registres et fenêtres isolés ; aucune application personnelle n'a été installée ou désinstallée pour ces mesures.
