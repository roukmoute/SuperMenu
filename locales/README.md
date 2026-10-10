# Traductions de Super Menu

Le français est le texte de secours intégré au programme. Les autres langues sont des fichiers JSON séparés : aucun pack n’est inclus dans l’exécutable ou l’archive portable. Le choix d’une langue télécharge uniquement son fichier lorsqu’il n’existe pas encore dans le cache local. Les lancements suivants utilisent ce cache sans accès réseau.

## Format d’un pack

Un fichier `locales/en.json` contient un objet JSON UTF-8 :

```json
{
  "schema_version": 1,
  "language": "en",
  "messages": {
    "Minuteur": "Timer",
    "Lancement de {name}…": "Launching {name}…"
  }
}
```

Les clés de `messages` sont les textes français exacts employés dans le code, y compris les accents, espaces, ponctuation et retours à la ligne. Les valeurs contiennent leur traduction. Une clé absente affiche le français, ce qui permet au programme de continuer à fonctionner avec un pack plus ancien.

Les paramètres entre accolades, tels que `{name}`, doivent conserver leur nom et leur nombre d’occurrences dans la traduction. Leur ordre peut changer. Ils sont remplacés par du texte littéral ; une valeur contenant des accolades n’est pas interprétée comme un autre modèle. Ne traduisez pas les noms de paramètres.

Le lecteur refuse les clés dupliquées, les champs de schéma inconnus, une version différente de `1`, un code `language` incompatible, les caractères de contrôle non autorisés et les paramètres manquants ou ajoutés. Un pack doit être non vide, faire au maximum 1 Mio, contenir au maximum 4 096 messages et limiter chaque clé ou valeur à 16 384 octets. Utilisez les échappements JSON habituels, notamment `\n` pour un retour à la ligne.

## Ajouter une langue

1. Copiez `en.json` vers `locales/<code>.json`, avec un code explicite comme `de`. Modifiez `language` et traduisez toutes les valeurs sans changer les clés françaises.
2. Ajoutez une entrée `Language { code: "de", name: "Deutsch" }` dans `LANGUAGES`, au début de `src/i18n.rs`. Cette liste alimente le sélecteur et la normalisation des réglages ; aucun autre fichier de configuration ne doit définir une liste indépendante.
3. Conservez exactement le même code, en minuscules, dans l’entrée Rust, le nom de fichier et le champ JSON. Lisez et testez le code canonique exact ; les packs et leurs chemins ne sont pas recherchés sous d’autres variantes de casse.
4. Exécutez `.\scripts\test.ps1` sous Windows, puis vérifiez le menu et la fenêtre « À propos » dans la nouvelle langue, y compris les libellés longs. Ajoutez les scénarios utiles dans `tests/windows/translations.ps1`.
5. Publiez le fichier JSON et l’entrée Rust ensemble. Le téléchargement lit `locales/<code>.json` depuis la branche `main` de ce dépôt ; il ne consulte pas de service de traduction externe.

## Ajouter un texte dans le code

Utilisez `crate::i18n::tr("Texte français")` pour un libellé simple. Pour un texte paramétré, utilisez un modèle français et des valeurs nommées :

```rust
crate::i18n::fmt(
    "Lancement de {name}…",
    &[("name", application_name.to_string())],
)
```

Ajoutez la même clé française à chaque pack maintenu. Les identifiants d’actions, préfixes de recherche, commandes WinGet, clés de fichiers de configuration et noms réels des logiciels restent indépendants de la langue. Les chaînes saisies par l’utilisateur ne sont pas traduites.

Le helper d’installation et de désinstallation charge uniquement le pack déjà enregistré : il ne télécharge rien. Les tests Rust emploient le français de secours et des packs de test isolés, sans charger les réglages personnels du développeur.
