# Performances de Super Menu

## Version 0.6.16

Les jeux Steam sont une option désactivée par défaut, accessible dans **Paramètres → Jeux Steam installés**. Le parcours du registre déjà utilisé pour la présence et la désinstallation fournit aussi les entrées de lancement lorsqu’elle est activée. Désactivée, la branche retourne avant l’analyse des identifiants Steam et avant toute allocation de jeu ; les noms nécessaires à la désinstallation restent dans l’inventaire commun. Aucun SDK, fichier de bibliothèque Steam, réseau, thread, temporisateur ou dépendance supplémentaire n’est utilisé.

Les jeux gardent un identifiant stable `steam:<AppID>` et une cible `steam://run/<AppID>`. Les doublons entre vues du registre et raccourcis existants sont évités. L’icône est lue dans le registre uniquement à la demande d’un résultat visible, sur le worker d’icônes existant. Aucun champ n’est ajouté aux quelque 20 000 éléments de fichiers. Un booléen partagé protège les publications en cours : désactiver l’option retire les jeux et purge les icônes sans qu’un ancien worker puisse les réintroduire.

Les entrées générées ne sont jamais enregistrées dans le cache principal. Le cache des applications et fichiers reste compatible ; les jeux reviennent pendant la phase d’indexation des applications, après activation, sans bloquer l’ouverture du menu. Cela évite de désérialiser des jeux au démarrage lorsque l’option est désactivée.

Le binaire local stable fait **1 019 904 octets**, contre **1 015 808 octets** pour la 0.6.15 conservée sur cette machine : **4 096 octets supplémentaires (+0,40 %)**, soit 0,97 Mio. Le profil Release, la chaîne stable MSVC et les dépendances sont inchangés. Le code de l’option reste dans l’EXE même lorsqu’elle est désactivée ; son activation conditionne le travail et les données supplémentaires en mémoire.

### Mesures du menu, option désactivée

Même fixture de huit éléments, thème clair, DPI 96 et vraie peinture parent `WM_PAINT`. Chaque série comporte trois lancements, trente échauffements et trois cents mesures par opération. Une première séquence référence / 0.6.16 / 0.6.16 / référence a été suivie d’une séquence inverse 0.6.16 / référence / référence / 0.6.16 pour vérifier l’influence de l’ordre. Médiane / p95 en millisecondes :

| Série | Premier dessin, médiane interne | Peinture parent | Recherche puis peinture |
| --- | ---: | ---: | ---: |
| Référence A | 115,501 | 1,900 / 3,419 | 4,528 / 6,708 |
| 0.6.16 A | 90,561 | 2,097 / 3,111 | 5,598 / 9,084 |
| 0.6.16 B | 121,136 | 2,369 / 3,381 | 5,469 / 8,086 |
| Référence B | 104,566 | 2,041 / 2,764 | 5,099 / 7,981 |
| 0.6.16 C | 119,341 | 2,543 / 4,831 | 4,749 / 7,396 |
| Référence C | 118,231 | 3,032 / 4,307 | 5,192 / 7,872 |
| Référence D | 114,921 | 2,323 / 3,669 | 5,014 / 8,060 |
| 0.6.16 D | 113,411 | 2,278 / 3,556 | 5,299 / 7,833 |

La première comparaison donnait des recherches plus lentes pour la nouvelle version ; la séquence inverse montre des médianes qui se recouvrent et une série nouvelle plus rapide que les références. Le PC était fortement sollicité par d’autres compilations. Ces données ne montrent pas un décalage constant et ne garantissent pas une latence donnée. Le chemin de classement à chaque frappe n’a pas changé. Les premiers dessins sont des mesures internes avec caches Windows non contrôlés ; la présentation DWM et le dessin du contrôle EDIT enfant restent exclus.

Les compteurs restent à **28 objets GDI et 26 USER**. Les mémoires privées des fixtures se recouvrent : 2,58–2,68 Mo pour la référence et 2,60–2,64 Mo pour la nouvelle version. Ces fixtures vérifient le menu désactivé ; elles ne mesurent pas le coût d’une bibliothèque Steam complète.

### Vérification

**241 tests Rust**, un benchmark ignoré et **1 093 assertions Windows** passent, installateur inclus. Onze tests ajoutés couvrent le caractère facultatif du réglage, sa compatibilité et sa sauvegarde, l’absence d’allocation du vecteur de jeux désactivé, les identifiants bornés, les titres Unicode, les doublons, les chemins d’icône, le retrait des entrées et l’exclusion du cache.

Un contrôle supplémentaire avec données temporaires et lecture seule du vrai registre vérifie les deux états du dialogue natif et l’absence de jeux dans le cache. Artisan TD apparaît avec sa propre icône et la mention « Jeu Steam » uniquement après activation. Le dialogue a été annulé ; aucun jeu n’a été lancé ou désinstallé, aucun réglage personnel n’a été enregistré.

SHA-256 du binaire local mesuré :

```text
043F2343ECDC384437D96BCA9AEC3F862C9620F5625A6083E829422472B1A375
```

## Version 0.6.15

Le menu prépare l’image complète dans un bitmap GDI réutilisable avant une seule copie vers la fenêtre. Le découpage du DC destination protège le contrôle EDIT et son curseur. Le tampon est alloué au premier dessin, réutilisé à dimensions constantes, remplacé au redimensionnement et libéré à la destruction. Un échec d’allocation ou de copie conserve un dessin fonctionnel. Aucun thread, temporisateur, dépendance ou DLL supplémentaire n’est ajouté.

Le binaire local stable fait **1 015 808 octets**, contre **1 014 784 octets** pour la 0.6.14 : **1 024 octets supplémentaires (+0,10 %)**. Les sections PE sont alignées sur 512 octets. La gestion commune de la première allocation et du redimensionnement a été simplifiée ; la taille finale reste identique. Les tests sont exclus du binaire distribué. Une surface de 720 × 680 pixels à 32 bits représente **1 958 400 octets**, environ 1,96 Mo de mémoire graphique gérée par Windows.

### Peinture réelle du menu

Le paramètre facultatif `scripts/measure-performance.ps1 -WindowPaint` utilise le vrai `WM_PAINT` parent via `RedrawWindow`, tampon et copie compris. Le mode historique `WM_PRINTCLIENT` reste disponible par défaut. Les deux modes excluent le dessin du contrôle EDIT enfant et la présentation des pixels par DWM. Les horloges incluent les échanges Win32 entre les processus et le travail de Windows ; les premiers dessins sont chronométrés dans `ui::run` et restent des lancements répétés avec caches non contrôlés.

Même chaîne stable, même profil Release, fixture de huit éléments, thème clair, DPI 96, fenêtre 720 × 680. Trois lancements, trente échauffements et trois cents mesures par opération dans chaque série, ordre **0.6.15 / référence / référence / 0.6.15**. Chaque cellule indique médiane / p95 en millisecondes.

| Mesure | 0.6.15 A | Référence A | Référence B | 0.6.15 B |
| --- | ---: | ---: | ---: | ---: |
| Peinture parent | 2,533 / 3,532 | 2,641 / 3,542 | 3,998 / 5,059 | 1,946 / 2,645 |
| Recherche puis peinture parent | 4,913 / 6,979 | 5,342 / 7,503 | 5,161 / 7,611 | 4,581 / 7,413 |
| Premier dessin, médiane interne | 107,313 | 93,413 | 92,653 | 85,946 |

Ces séries finales ne montrent pas de ralentissement de la peinture. La variabilité de Windows est importante : une première comparaison exploratoire donnait 6–9 ms pour recherche puis peinture, contre 4,6–5,3 ms dans la série finale. Ces mesures ne prouvent donc pas une accélération systématique ou une latence garantie. Le correctif vise à supprimer les images intermédiaires visibles.

Après échauffement, les compteurs GDI passent de **26 à 28**, USER reste à **26**. La mémoire privée observée est autour de 2,5 Mo ; elle ne comptabilise pas nécessairement les surfaces graphiques allouées par Windows. Les tests de redimensionnement vérifient que les ressources ne s’accumulent pas.

### Vérification

Quatre tests GDI natifs vérifient que le dessin du tampon reste invisible avant présentation, que les pixels copiés sont complets, que le découpage protège une zone de contrôle enfant, que le tampon est réutilisé et que ses handles sont réellement libérés. Le test de libération consulte `GetObjectW`, car `GetObjectType` conserve ici le type encodé d’un handle déjà supprimé. Les paramètres invalides sont refusés sans allocation.

La suite du lanceur impose vingt-quatre vraies peintures avec indexation simulée, notifications d’icônes, navigation et six changements de taille ; elle contrôle le texte, la sélection EDIT, le résultat choisi et les compteurs GDI/USER. Validation : **230 tests Rust**, un benchmark facultatif ignoré et **1 093 assertions Windows** (458 lanceur, 278 À propos, 171 traductions, 40 console, 146 installateur). Le lanceur est également vérifié dans le thème clair du GIF signalé.

SHA-256 du binaire local final mesuré :

```text
3D1DAC4EB45DF0A8B9B871CEE08A94850CE446E8FB74DB412AED0429646583B1
```

## Version 0.6.14

Correctif de navigation : les mises à jour automatiques restaurent l’identité du résultat sélectionné avant de demander les icônes visibles. Si le résultat disparaît, la position précédente est bornée à la nouvelle liste ; une vraie modification de requête sélectionne toujours la première ligne. Le chemin de frappe ne clone aucun identifiant et ne parcourt pas les résultats pour conserver la sélection. Les rafraîchissements utilisent un seul identifiant et un parcours borné par le nombre de résultats. Aucun thread, temporisateur, état persistant, import ou dépendance n’est ajouté.

Avec la même chaîne stable et le même profil que la mesure locale 0.6.13, le binaire fait **1 014 784 octets**, contre **1 014 272 octets** : **512 octets supplémentaires (+0,05 %)**. Le profil et les optimisations 0.6.13 sont conservés ; la variante nightly reste facultative et expérimentale. Cette comparaison de taille ne constitue pas une nouvelle mesure de latence.

Le test natif échoue sur l’EXE officiel 0.6.13 au premier rafraîchissement et passe sur le correctif. Il vérifie une sélection en bas de l’accueil, une deuxième application sélectionnée, trois notifications d’index successives dans chaque cas, puis le retour à la première ligne lors d’un changement de requête. Trois tests Rust couvrent le réordonnancement, la disparition ou limitation des résultats et la liste vide. Validation locale : **226 tests Rust**, un benchmark facultatif ignoré et **1 017 assertions Windows** (382 lanceur, 278 À propos, 171 traductions, 40 console, 146 installateur).

SHA-256 du binaire local mesuré :

```text
9052964F0D8A194F03A2BADB0DFAB6B4EC7BC97090DD4D1D358A8656272420F4
```

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

La normalisation Win32 est identique au crate pour chaque caractère des blocs Latin-1 Supplément, Latin étendu A et B, grec, cyrillique et Latin étendu additionnel, ainsi que pour les titres de la fixture. Les divergences restantes sont documentées par des tests : les marques d'espacement indiennes et birmanes que Windows ne classe ni comme diacritiques ni comme voyelles sont conservées, de même que les marques combinantes ajoutées après Unicode 5.1 et les marques hors plan multilingue de base ; trois caractères reclassés par Unicode sont encore supprimés. Les non-caractères, acceptés par NTFS mais refusés par `NormalizeString`, sont remplacés par U+FFFD avant l'appel. Le chemin ASCII, majoritaire pour les noms d'applications et de fichiers, ne fait ni allocation ni appel Win32. Pour un titre accentué, la version Win32 coûte **1,3 µs** contre 0,95 µs pour le crate (20 000 titres accentués : 27 ms contre 19 ms, sur le thread d'indexation) ; pour une requête accentuée, **0,7 µs** contre 0,2 µs, à comparer aux 3 ms d'une recherche suivie d'un dessin. La table des imports ne contient aucune nouvelle bibliothèque : `NormalizeString` est résolue par `kernel32.dll`, et `oleaut32.dll` ainsi que les tubes nommés de `ntdll.dll` utilisés par `std::process` ne sont plus importés.

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

### Démarrage complet et requêtes accentuées

`first_frame_ms` exclut la création du processus et le chargement des DLL. Un chronomètre externe mesure ici le délai entre `CreateProcess` et la première fenêtre `SuperMenuWindow` visible, en mode fixture, quinze lancements alternés par version, caches Windows chauds :

| Démarrage externe | Référence | 0.6.13 |
| --- | ---: | ---: |
| Médiane | 66,3 ms | 64,0 ms |
| p95 | 145,2 ms | 90,6 ms |
| Minimum | 61,1 ms | 54,7 ms |

Les benchmarks précédents n'utilisent que des requêtes ASCII, qui ne passent pas par la normalisation remplacée. La même mesure du menu a donc été reprise avec `tests/fixtures/launcher-accents.json` (vingt entrées dont douze titres accentués, grecs et cyrilliques) et des requêtes accentuées (`@ éditeur`, `@ caméra`, `@ télé`, `@ désinst`, `@ ångs`, `@ no-match`, `/ vidéo`, `@ é`), deux séries par version en ordre référence / 0.6.13 / 0.6.13 / référence :

| Mesure | Référence | 0.6.13 |
| --- | ---: | ---: |
| Premier dessin, horloge interne | 65,8–76,4 ms | 72,7–75,4 ms |
| Dessin seul | 0,062–0,063 ms | 0,066–0,070 ms |
| Recherche accentuée puis dessin, médiane | 2,33–2,78 ms | 2,39–2,51 ms |
| Recherche accentuée puis dessin, p95 | 5,47–5,67 ms | 6,10–6,21 ms |

Les médianes se recouvrent ; les p95 de la 0.6.13 sont supérieurs d'environ 0,5 ms sur ces deux séries, alors que les séries ASCII précédentes les plaçaient tantôt au-dessus, tantôt en dessous de la référence. Le coût mesuré de la normalisation Win32 (0,5 µs de plus par requête accentuée) ne peut pas produire un tel écart ; il relève du bruit d'ordonnancement de la machine, et ces chiffres ne garantissent pas une latence donnée.

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

Validation fonctionnelle : **223 tests Rust**, dont les tests de parité de la normalisation Unicode, du vrai raccourci `.lnk` et de la Corbeille. Sur le binaire mesuré ici, **851 assertions Windows** ont été rejouées localement (362 lanceur, 278 À propos, 171 traductions, 40 console d'aide). Les **146 assertions de l'installateur** demandent une vraie installation NSIS et n'ont pas été rejouées sur ce poste ; elles ont été exécutées par l'intégration continue sur le commit publié ([exécution 38255429297](https://github.com/roukmoute/SuperMenu/actions/runs/38255429297)), ce qui porte le total validé à 997. Clippy sans avertissement et `rustfmt` sont imposés.

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
