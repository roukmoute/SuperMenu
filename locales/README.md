# Super Menu translations

**English** · [Français](README.fr.md)

French is the fallback text built into the program. Other languages are separate JSON files: no pack is included in the executable or portable archive. Selecting a language downloads only its file when it is not yet in the local cache. Subsequent starts use this cache without network access.

## Pack format

A `locales/en.json` file contains a UTF-8 JSON object:

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

`messages` keys are the exact French texts used in the code, including accents, spaces, punctuation and line breaks. Values contain their translations. A missing key displays French, allowing the program to continue working with an older pack.

Parameters in braces, such as `{name}`, must retain their name and occurrence count in the translation. Their order may change. They are replaced with literal text; a value containing braces is not interpreted as another template. Do not translate parameter names.

The reader rejects duplicate keys, unknown schema fields, a version other than `1`, an incompatible `language` code, disallowed control characters, and missing or added parameters. A pack must be nonempty, no larger than 1 MiB, contain at most 4 096 messages, and limit every key or value to 16 384 bytes. Use standard JSON escapes, particularly `\n` for a line break.

## Adding a language

1. Copy `en.json` to `locales/<code>.json`, using an explicit code such as `de`. Change `language` and translate every value without changing the French keys.
2. Add a `Language { code: "de", name: "Deutsch" }` entry to `LANGUAGES` at the beginning of `src/i18n.rs`. This list drives the selector and settings normalization; no other configuration file should define an independent list.
3. Keep exactly the same lowercase code in the Rust entry, filename and JSON field. Read and test the exact canonical code; packs and paths are not looked up using other case variants.
4. Run `.\scripts\test.ps1` on Windows, then check the menu and “About” window in the new language, including long labels. Add useful scenarios in `tests/windows/translations.ps1`.
5. Publish the JSON file and Rust entry together. Downloading reads `locales/<code>.json` from this repository's `main` branch; it consults no external translation service.

## Adding text in code

Use `crate::i18n::tr("Texte français")` for a simple label. For parameterized text, use a French template and named values:

```rust
crate::i18n::fmt(
    "Lancement de {name}…",
    &[("name", application_name.to_string())],
)
```

Add the same French key to every maintained pack. Action identifiers, search prefixes, WinGet commands, configuration-file keys and actual software names stay independent of language. User-entered strings are not translated.

The installation and uninstallation helper loads only the already saved pack: it downloads nothing. Rust tests use French fallback and isolated test packs without loading the developer's personal settings.
