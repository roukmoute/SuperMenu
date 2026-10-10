//! Local software management suggestions. Searching never runs WinGet or reads the registry.
//! IDs verified against https://github.com/microsoft/winget-pkgs on 2026-10-10.
//! A manifest lives at manifests/{publisher initial}/{dot-separated ID components}.
use crate::core::{self, Item, Kind};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

pub struct Package {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub aliases: &'static [&'static str],
}

macro_rules! package {
    ($id:literal, $name:literal, $category:literal, [$($alias:literal),* $(,)?]) => {
        Package { id: $id, name: $name, category: $category, aliases: &[$($alias),*] }
    };
}

pub const PACKAGES: &[Package] = &[
    package!(
        "Mozilla.Firefox",
        "Firefox",
        "Navigateur",
        ["mozilla firefox", "firefox esr", "firefoxesr", "ff"]
    ),
    package!("Google.Chrome", "Google Chrome", "Navigateur", ["chrome"]),
    package!("Microsoft.Edge", "Microsoft Edge", "Navigateur", ["edge"]),
    package!("Brave.Brave", "Brave", "Navigateur", ["brave browser"]),
    package!(
        "Vivaldi.Vivaldi",
        "Vivaldi",
        "Navigateur",
        ["vivaldi browser"]
    ),
    package!("Opera.Opera", "Opera", "Navigateur", ["opera browser"]),
    package!(
        "Microsoft.VisualStudioCode",
        "Visual Studio Code",
        "Éditeur de code",
        ["vscode", "vs code", "microsoft visual studio code"]
    ),
    package!(
        "Git.Git",
        "Git",
        "Développement",
        ["git for windows", "git bash", "git gui"]
    ),
    package!(
        "GitHub.GitHubDesktop",
        "GitHub Desktop",
        "Développement",
        ["githubdesktop", "gh desktop"]
    ),
    package!(
        "Notepad++.Notepad++",
        "Notepad++",
        "Éditeur de texte",
        ["notepad++", "npp"]
    ),
    package!(
        "Microsoft.PowerShell",
        "PowerShell 7",
        "Terminal",
        [
            "powershell",
            "pwsh",
            "powershell7",
            "powershell x64",
            "powershell x86"
        ]
    ),
    package!(
        "Microsoft.WindowsTerminal",
        "Windows Terminal",
        "Terminal",
        ["terminal", "windowsterminal"]
    ),
    package!(
        "Microsoft.PowerToys",
        "PowerToys",
        "Utilitaires Windows",
        ["microsoft powertoys", "power toys"]
    ),
    package!(
        "voidtools.Everything",
        "Everything",
        "Recherche de fichiers",
        ["voidtools everything"]
    ),
    package!("7zip.7zip", "7-Zip", "Compression", ["7zip", "7 zip"]),
    package!("RARLab.WinRAR", "WinRAR", "Compression", ["win rar"]),
    package!(
        "WinSCP.WinSCP",
        "WinSCP",
        "Transfert de fichiers",
        ["win scp"]
    ),
    package!("PuTTY.PuTTY", "PuTTY", "SSH et terminal", ["putty"]),
    package!("Rufus.Rufus", "Rufus", "Clé USB bootable", ["rufus usb"]),
    package!(
        "VideoLAN.VLC",
        "VLC",
        "Lecteur multimédia",
        ["vlc media player", "videolan vlc"]
    ),
    package!("Spotify.Spotify", "Spotify", "Musique", ["spotify music"]),
    package!(
        "OBSProject.OBSStudio",
        "OBS Studio",
        "Capture et streaming",
        ["obs", "obs studio"]
    ),
    package!(
        "Audacity.Audacity",
        "Audacity",
        "Édition audio",
        ["audacity audio"]
    ),
    package!(
        "HandBrake.HandBrake",
        "HandBrake",
        "Conversion vidéo",
        ["hand brake"]
    ),
    package!("GIMP.GIMP", "GIMP", "Retouche photo", ["gimp image editor"]),
    package!(
        "Inkscape.Inkscape",
        "Inkscape",
        "Dessin vectoriel",
        ["inkscape vector"]
    ),
    package!("KDE.Krita", "Krita", "Dessin et peinture", ["kde krita"]),
    package!(
        "BlenderFoundation.Blender",
        "Blender",
        "Création 3D",
        ["blender 3d"]
    ),
    package!(
        "ImageMagick.ImageMagick",
        "ImageMagick",
        "Traitement d'images",
        ["image magick"]
    ),
    package!("Discord.Discord", "Discord", "Messagerie", ["discord"]),
    package!(
        "Microsoft.Teams",
        "Microsoft Teams",
        "Visioconférence",
        ["teams"]
    ),
    package!(
        "SlackTechnologies.Slack",
        "Slack",
        "Messagerie professionnelle",
        ["slack"]
    ),
    package!(
        "Zoom.Zoom",
        "Zoom",
        "Visioconférence",
        ["zoom workplace", "zoom meetings"]
    ),
    package!(
        "Telegram.TelegramDesktop",
        "Telegram",
        "Messagerie",
        ["telegram desktop"]
    ),
    package!(
        "Bitwarden.Bitwarden",
        "Bitwarden",
        "Gestionnaire de mots de passe",
        ["bitwarden desktop"]
    ),
    package!(
        "KeePassXCTeam.KeePassXC",
        "KeePassXC",
        "Gestionnaire de mots de passe",
        ["keepass xc"]
    ),
    package!("Proton.ProtonVPN", "Proton VPN", "VPN", ["protonvpn"]),
    package!(
        "Tailscale.Tailscale",
        "Tailscale",
        "Réseau VPN",
        ["tailscale vpn"]
    ),
    package!("WireGuard.WireGuard", "WireGuard", "VPN", ["wire guard"]),
    package!("Valve.Steam", "Steam", "Jeux", ["steam client"]),
    package!(
        "EpicGames.EpicGamesLauncher",
        "Epic Games Launcher",
        "Jeux",
        ["epic games", "epic launcher"]
    ),
    package!(
        "GOG.Galaxy",
        "GOG Galaxy",
        "Jeux",
        ["gog galaxy", "goggalaxy"]
    ),
    package!(
        "TheDocumentFoundation.LibreOffice",
        "LibreOffice",
        "Bureautique",
        ["libre office"]
    ),
    package!(
        "SumatraPDF.SumatraPDF",
        "SumatraPDF",
        "Lecteur PDF",
        ["sumatra", "sumatra pdf"]
    ),
    package!("Obsidian.Obsidian", "Obsidian", "Notes", ["obsidian notes"]),
    package!(
        "Notion.Notion",
        "Notion",
        "Notes et organisation",
        ["notion desktop"]
    ),
    package!("Anki.Anki", "Anki", "Apprentissage", ["anki flashcards"]),
    package!("Joplin.Joplin", "Joplin", "Notes", ["joplin notes"]),
    package!(
        "Docker.DockerDesktop",
        "Docker Desktop",
        "Développement",
        ["docker"]
    ),
    package!(
        "DBeaver.DBeaver.Community",
        "DBeaver Community",
        "Bases de données",
        ["dbeaver", "dbeaver ce"]
    ),
    package!(
        "Postman.Postman",
        "Postman",
        "Développement API",
        ["postman api"]
    ),
    package!(
        "Microsoft.AzureCLI",
        "Azure CLI",
        "Développement cloud",
        ["microsoft azure cli", "azurecli"]
    ),
];

/// Only canonical, curated IDs are accepted. Returns the canonical spelling for execution.
pub fn find(id: &str) -> Option<&'static Package> {
    PACKAGES
        .iter()
        .find(|package| package.id.eq_ignore_ascii_case(id))
}

struct PreparedPackage {
    package: &'static Package,
    primary_name: String,
    names: Vec<String>,
    category: String,
    identifier: String,
}

fn prepared() -> &'static [PreparedPackage] {
    static PREPARED: OnceLock<Vec<PreparedPackage>> = OnceLock::new();
    PREPARED.get_or_init(|| {
        PACKAGES
            .iter()
            .map(|package| {
                let mut names: Vec<String> = std::iter::once(package.name)
                    .chain(package.aliases.iter().copied())
                    .map(normalize)
                    .collect();
                names.sort();
                names.dedup();
                PreparedPackage {
                    package,
                    primary_name: normalize(package.name),
                    names,
                    category: normalize(package.category),
                    identifier: normalize(package.id),
                }
            })
            .collect()
    })
}

fn normalize(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut spacing = true;
    // Product names are usually ASCII. Preserve '+' and '#' exactly as the
    // Unicode path does, while avoiding decomposition/category lookup per byte.
    if value.is_ascii() {
        for byte in value.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'#') {
                result.push(byte.to_ascii_lowercase() as char);
                spacing = false;
            } else if !spacing {
                result.push(' ');
                spacing = true;
            }
        }
        if spacing {
            result.pop();
        }
        return result;
    }
    for character in value.nfd().filter(|c| !is_combining_mark(*c)) {
        // Keep product punctuation: Notepad++ differs from Notepad, and C# differs from C.
        if character.is_alphanumeric() || matches!(character, '+' | '#') {
            result.extend(character.to_lowercase());
            spacing = false;
        } else if !spacing {
            result.push(' ');
            spacing = true;
        }
    }
    if result.ends_with(' ') {
        result.pop();
    }
    result
}

fn installed_match(name: &str, package: &PreparedPackage) -> bool {
    package.names.iter().any(|alias| {
        let Some(suffix) = name.strip_prefix(alias) else {
            return false;
        };
        if suffix.is_empty() {
            return true;
        }
        // Accept version/architecture suffixes, but never concatenate arbitrary app names.
        if !suffix.starts_with(' ') && !suffix.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return false;
        }
        suffix
            .split_whitespace()
            .all(|token| installed_suffix(package.package.id, token))
    })
}

fn installed_suffix(id: &str, token: &str) -> bool {
    if token.chars().next().is_some_and(|c| c.is_ascii_digit())
        || matches!(
            token,
            "x64"
                | "x86"
                | "amd64"
                | "arm64"
                | "aarch64"
                | "i386"
                | "i686"
                | "win64"
                | "win32"
                | "bit"
                | "bits"
                | "edition"
                | "version"
                | "release"
                | "windows"
                | "user"
                | "system"
                | "stable"
                | "portable"
                | "preview"
                | "en"
                | "fr"
                | "de"
                | "es"
                | "it"
                | "pt"
                | "ru"
                | "ja"
                | "ko"
                | "zh"
                | "pl"
                | "nl"
                | "sv"
                | "cs"
                | "tr"
                | "uk"
                | "us"
                | "gb"
                | "cn"
                | "tw"
                | "br"
                | "jp"
        )
    {
        return true;
    }
    match id {
        "Mozilla.Firefox" => matches!(
            token,
            "esr"
                | "beta"
                | "nightly"
                | "developer"
                | "private"
                | "browsing"
                | "navigation"
                | "privee"
        ),
        "Google.Chrome" => matches!(token, "beta" | "dev" | "canary"),
        "Brave.Brave" => matches!(token, "browser" | "beta" | "nightly"),
        "Opera.Opera" => matches!(token, "gx" | "browser"),
        "Microsoft.VisualStudioCode" => token == "insiders",
        "Git.Git" => matches!(token, "for" | "bash" | "gui"),
        "VideoLAN.VLC" => matches!(token, "media" | "player"),
        "ImageMagick.ImageMagick" => matches!(token, "q8" | "q16" | "q32" | "hdri"),
        "TheDocumentFoundation.LibreOffice" => matches!(
            token,
            "writer" | "calc" | "draw" | "base" | "impress" | "math"
        ),
        "SumatraPDF.SumatraPDF" => token == "reader",
        "DBeaver.DBeaver.Community" => matches!(token, "ce" | "community"),
        _ => false,
    }
}

fn package_item(package: &Package) -> Item {
    Item::new(
        format!("install:{}", package.id),
        package.name,
        crate::i18n::fmt(
            "Installer · {category} · WinGet · Entrée pour installer",
            &[("category", crate::i18n::tr(package.category))],
        ),
        "↓",
        Kind::Install,
        package.id,
    )
}

/// Recompute on catalog/registry refresh, then cache the result in the UI.
pub fn missing_items(
    index: &[Item],
    installed_names: &[String],
    completed: &HashSet<String>,
) -> Vec<Item> {
    let names: Vec<String> = index
        .iter()
        .filter(|item| item.kind == Kind::Application)
        .map(|item| normalize(&item.title))
        .chain(installed_names.iter().map(|name| normalize(name)))
        .collect();
    prepared()
        .iter()
        .filter(|package| {
            !completed
                .iter()
                .any(|id| id.eq_ignore_ascii_case(package.package.id))
                && !names.iter().any(|name| installed_match(name, package))
        })
        .map(|package| package_item(package.package))
        .collect()
}

// Presence detection intentionally groups channels to avoid advertising another browser
// installation. Destructive actions must keep those channels separate instead.
fn package_channel(name: &str, package: &PreparedPackage) -> &'static str {
    let has = |token: &str| name.split_whitespace().any(|word| word == token);
    match package.package.id {
        "Mozilla.Firefox" if has("esr") || name.starts_with("firefoxesr") => "esr",
        "Mozilla.Firefox" if has("nightly") => "nightly",
        "Mozilla.Firefox" if has("developer") => "developer",
        "Google.Chrome" if has("canary") => "canary",
        "Google.Chrome" if has("dev") => "dev",
        "Opera.Opera" if has("gx") => "gx",
        "Microsoft.VisualStudioCode" if has("insiders") => "insiders",
        "Microsoft.PowerShell"
            if name
                .split_whitespace()
                .find(|word| word.chars().next().is_some_and(|c| c.is_ascii_digit()))
                .is_some_and(|word| word.parse::<u32>().is_ok_and(|version| version <= 5)) =>
        {
            "legacy"
        }
        _ if has("beta") => "beta",
        _ if has("nightly") => "nightly",
        _ if has("preview") => "preview",
        _ if has("portable") => "portable",
        _ => "stable",
    }
}

fn uninstall_match(name: &str, package: &PreparedPackage) -> bool {
    package_channel(name, package) == "stable" && family_identity_match(name, package)
}

fn family_identity_match(name: &str, package: &PreparedPackage) -> bool {
    package.names.iter().any(|alias| {
        // Typing aliases are useful for search, but names such as FF/terminal are
        // insufficient evidence for a destructive action on a specific package.
        if alias != &package.primary_name
            && (alias.chars().count() <= 3
                || matches!(alias.as_str(), "terminal" | "pwsh" | "gh desktop"))
        {
            return false;
        }
        let Some(suffix) = name.strip_prefix(alias) else {
            return false;
        };
        if suffix.is_empty() {
            return true;
        }
        if !suffix.starts_with(' ') && !suffix.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return false;
        }
        suffix.split_whitespace().all(|token| {
            installed_suffix(package.package.id, token)
                && (!token.chars().next().is_some_and(|c| c.is_ascii_digit())
                    || token.chars().all(|c| c.is_ascii_digit()))
        })
    })
}

fn valid_registry_name(name: &str) -> bool {
    !name.is_empty()
        && name == name.trim()
        && name.chars().count() <= 2048
        && !name.chars().any(char::is_control)
}

/// Cleanup snapshots use the same exact identity and stable-channel policy as
/// the removal action. Search aliases never become registry ownership evidence.
pub fn registry_name_matches_target(target: &str, name: &str) -> bool {
    if !valid_registry_name(name) {
        return false;
    }
    if let Some(exact) = target.strip_prefix("name:") {
        return valid_registry_name(exact) && exact.eq_ignore_ascii_case(name);
    }
    let Some(id) = target.strip_prefix("id:") else {
        return false;
    };
    prepared().iter().any(|package| {
        package.package.id.eq_ignore_ascii_case(id) && uninstall_match(&normalize(name), package)
    })
}

fn removal_item(package: &Package) -> Item {
    Item::new(
        format!("uninstall:id:{}", package.id),
        package.name,
        crate::i18n::fmt(
            "Désinstaller et nettoyer · {category} · données personnelles conservées",
            &[("category", crate::i18n::tr(package.category))],
        ),
        "−",
        Kind::Uninstall,
        format!("id:{}", package.id),
    )
}

fn named_removal_item(name: &str, category: &str) -> Item {
    Item::new(
        format!("uninstall:name:{name}"),
        name,
        crate::i18n::fmt(
            "Désinstaller et nettoyer · {category} · données personnelles conservées",
            &[("category", crate::i18n::tr(category))],
        ),
        "−",
        Kind::Uninstall,
        format!("name:{name}"),
    )
}

/// Build this list on the catalog worker. Unknown software requires an exact registry
/// DisplayName: an arbitrary application shortcut or a similarly named file is insufficient.
/// Pass the removable half of installed_inventory() for the registry names.
pub fn removable_items(index: &[Item], installed_names: &[String]) -> Vec<Item> {
    let app_names: Vec<String> = index
        .iter()
        .filter(|item| item.kind == Kind::Application)
        .map(|item| normalize(&item.title))
        .collect();
    let registry: Vec<(&str, String)> = installed_names
        .iter()
        .filter(|name| valid_registry_name(name))
        .map(|name| (name.as_str(), normalize(name)))
        .collect();
    let mut items = Vec::new();
    let mut known = HashSet::new();
    for package in prepared() {
        let registered_stable = registry
            .iter()
            .any(|(_, name)| uninstall_match(name, package));
        let registered_variant = registry.iter().any(|(_, name)| {
            family_identity_match(name, package) && package_channel(name, package) != "stable"
        });
        // An ESR-only registry snapshot can still expose a shortcut simply called
        // Firefox. Prefer its exact DisplayName over guessing the stable package ID.
        if registered_stable
            || (!registered_variant && app_names.iter().any(|name| uninstall_match(name, package)))
        {
            known.insert(package.package.id);
            items.push(removal_item(package.package));
        }
    }
    let mut names = HashSet::new();
    for (original, normalized) in registry {
        if prepared().iter().any(|package| {
            known.contains(package.package.id) && uninstall_match(&normalized, package)
        }) {
            continue;
        }
        // Keep punctuation and version in the execution target. Only casing is ignored
        // for duplicate registry views; C++, C# and C are different products.
        if names.insert(original.to_lowercase()) {
            let category = prepared()
                .iter()
                .find(|package| family_identity_match(&normalized, package))
                .map_or("Logiciel installé", |package| package.package.category);
            items.push(named_removal_item(original, category));
        }
    }
    items.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.target.cmp(&right.target))
    });
    items
}

/// The UI uses both snapshots so an application shortcut cannot reintroduce a
/// catalog product whose registry entry forbids removal.
pub fn removable_items_with_inventory(
    index: &[Item],
    all_names: &[String],
    removable_names: &[String],
) -> Vec<Item> {
    removable_items(index, removable_names)
        .into_iter()
        .filter(|item| removal_is_allowed(&item.target, all_names, removable_names))
        .collect()
}

/// Validate against a fresh worker/helper snapshot before executing WinGet. The
/// target is a catalog ID or an exact DisplayName, never a search query.
pub fn removal_is_allowed(target: &str, all_names: &[String], removable_names: &[String]) -> bool {
    if let Some(name) = target.strip_prefix("name:") {
        return valid_registry_name(name)
            && removable_names
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(name));
    }
    let Some(id) = target.strip_prefix("id:") else {
        return false;
    };
    let Some(package) = prepared()
        .iter()
        .find(|package| package.package.id.eq_ignore_ascii_case(id))
    else {
        return false;
    };
    let mut stable_registered = false;
    let mut variant_registered = false;
    for name in all_names {
        let normalized = normalize(name);
        if uninstall_match(&normalized, package) {
            stable_registered = true;
            // Any protected version blocks the shared ID, even if another version
            // is eligible. Exact-name fallback must never weaken this restriction.
            if !removable_names
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(name))
            {
                return false;
            }
        } else if family_identity_match(&normalized, package) {
            variant_registered = true;
        }
    }
    // No registry entry remains valid for UWP applications found in the app index.
    // A registry containing only ESR/Canary must not be mistaken for the stable ID.
    stable_registered || !variant_registered
}

fn version_name_match(left: &str, right: &str) -> bool {
    let (shorter, longer) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let Some(suffix) = longer.strip_prefix(shorter) else {
        return false;
    };
    if suffix.is_empty() {
        return true;
    }
    if !suffix.starts_with(' ') && !suffix.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return false;
    }
    suffix.split_whitespace().all(|token| {
        installed_suffix("", token)
            && !matches!(token, "preview" | "portable")
            && (!token.chars().next().is_some_and(|c| c.is_ascii_digit())
                || token.chars().all(|c| c.is_ascii_digit()))
            || token.strip_prefix('v').is_some_and(|version| {
                !version.is_empty() && version.chars().all(|c| c.is_ascii_digit())
            })
    })
}

/// Relate a completed removal to application shortcuts without matching files or other
/// channels. This is temporary cache suppression, never an executable command.
pub fn application_matches_removal(item: &Item, target: &str) -> bool {
    if item.kind != Kind::Application {
        return false;
    }
    let title = normalize(&item.title);
    if let Some(id) = target.strip_prefix("id:") {
        return prepared().iter().any(|package| {
            package.package.id.eq_ignore_ascii_case(id) && uninstall_match(&title, package)
        });
    }
    let Some(name) = target
        .strip_prefix("name:")
        .filter(|name| valid_registry_name(name))
    else {
        return false;
    };
    let name = normalize(name);
    for package in prepared() {
        if family_identity_match(&title, package) && family_identity_match(&name, package) {
            return package_channel(&title, package) == package_channel(&name, package);
        }
    }
    version_name_match(&title, &name)
}

fn valid_removal(item: &Item) -> bool {
    if item.kind != Kind::Uninstall || !item.arguments.is_empty() {
        return false;
    }
    if let Some(id) = item.target.strip_prefix("id:") {
        return find(id).is_some();
    }
    item.target
        .strip_prefix("name:")
        .is_some_and(|name| valid_registry_name(name) && name == item.title)
}

fn named_relevance(name: &str, query: &str) -> Option<u32> {
    if name == query {
        return Some(1000);
    }
    if name.starts_with(query) {
        return Some(850);
    }
    let tokens = query.split_whitespace();
    (tokens.clone().next().is_some()
        && tokens.clone().all(|query_token| {
            name.split_whitespace()
                .any(|token| token.starts_with(query_token))
        }))
    .then_some(750)
}

fn removal_relevance(item: &Item, query: &str) -> Option<u32> {
    let title = normalize(&item.title);
    let named = named_relevance(&title, query);
    let package = if let Some(id) = item.target.strip_prefix("id:") {
        prepared()
            .iter()
            .find(|package| package.package.id.eq_ignore_ascii_case(id))
    } else {
        prepared()
            .iter()
            .find(|package| family_identity_match(&title, package))
    };
    match (named, package.and_then(|package| relevance(package, query))) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

fn ranked_removals(
    removals: &[Item],
    body: &str,
    limit: usize,
    weights: &HashMap<String, u32>,
    explicit: bool,
) -> Vec<Item> {
    let normalized = normalize(body);
    if normalized.is_empty() {
        return removals
            .iter()
            .filter(|item| valid_removal(item))
            .take(limit)
            .cloned()
            .collect();
    }
    let mut ordinary = core::rank_filtered(removals, body, limit, weights, valid_removal);
    // Curated aliases such as npp/ff are intentionally searchable even if they are
    // absent from the item title. No full app index is cloned here.
    for item in removals.iter().filter(|item| valid_removal(item)) {
        if removal_relevance(item, &normalized).is_some_and(|score| score >= 750)
            && !ordinary.iter().any(|existing| existing.id == item.id)
        {
            ordinary.push(item.clone());
        }
    }
    let mut scores: Vec<_> = ordinary
        .into_iter()
        .filter_map(|item| {
            let score = removal_relevance(&item, &normalized).unwrap_or(0);
            (explicit || score >= 750).then_some((score, item))
        })
        .collect();
    scores.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.title.cmp(&right.title))
    });
    scores
        .into_iter()
        .take(limit)
        .map(|(_, item)| item)
        .collect()
}

fn relevance(package: &PreparedPackage, query: &str) -> Option<u32> {
    if query == package.identifier {
        return Some(1000);
    }
    if package.names.iter().any(|name| name == query) {
        return Some(1000);
    }
    if package.names.iter().any(|name| name.starts_with(query)) {
        return Some(850);
    }
    // Cloning this small string iterator is allocation-free; collecting the same
    // query into a Vec for every curated package adds avoidable per-key churn.
    let tokens = query.split_whitespace();
    tokens.clone().next()?;
    let name_match = package.names.iter().any(|name| {
        tokens.clone().all(|query_token| {
            name.split_whitespace()
                .any(|token| token.starts_with(query_token))
        })
    });
    if name_match {
        return Some(750);
    }
    // Category words may accompany a product name, or be searched on their own.
    let all_match = tokens.clone().all(|query_token| {
        package.names.iter().any(|name| {
            name.split_whitespace()
                .any(|token| token.starts_with(query_token))
        }) || package
            .category
            .split_whitespace()
            .any(|token| token.starts_with(query_token))
    });
    if all_match {
        return Some(500);
    }
    // Keep the prepared French source category for compatibility, while also
    // accepting the category currently shown by the selected language pack.
    // This late fallback avoids translation lookups for exact product matches.
    let translated = crate::i18n::tr(package.package.category);
    if translated == package.package.category {
        return None;
    }
    let category = normalize(&translated);
    tokens
        .clone()
        .all(|query_token| {
            package.names.iter().any(|name| {
                name.split_whitespace()
                    .any(|token| token.starts_with(query_token))
            }) || category
                .split_whitespace()
                .any(|token| token.starts_with(query_token))
        })
        .then_some(500)
}

fn strongly_matches(title: &str, query: &str) -> bool {
    let title = normalize(title);
    title == query
        || title.starts_with(query)
        || title
            .split_whitespace()
            .any(|token| token.starts_with(query))
}

/// Merge ranked index results with a cached list of missing packages. No filesystem,
/// registry, network, subprocess, or WinGet access happens while searching.
pub fn search(
    index: &[Item],
    missing: &[Item],
    query: &str,
    limit: usize,
    weights: &HashMap<String, u32>,
) -> Vec<Item> {
    if limit == 0 {
        return Vec::new();
    }
    let query = query.trim();
    let (install_only, apps_only, body) = if let Some(body) = query.strip_prefix('+') {
        (true, false, body.trim())
    } else if let Some(body) = query.strip_prefix('@') {
        (false, true, body.trim())
    } else {
        (false, false, query)
    };
    let normalized = normalize(body);
    let limit = limit.min(100);
    let mut ordinary = if install_only {
        Vec::new()
    } else {
        core::rank_filtered(
            index,
            body,
            limit.saturating_mul(2).min(200),
            weights,
            |item| item.kind == Kind::Application,
        )
    };
    if install_only && normalized.is_empty() {
        return missing
            .iter()
            .filter(|item| item.kind == Kind::Install)
            .filter_map(|item| find(&item.target))
            .take(limit)
            .map(package_item)
            .collect();
    }
    if (!install_only && normalized.chars().count() < 2) || normalized.len() > 256 {
        ordinary.truncate(limit);
        return ordinary;
    }
    let relevant: Vec<(&PreparedPackage, u32)> = prepared()
        .iter()
        .filter_map(|package| relevance(package, &normalized).map(|score| (package, score)))
        .collect();
    let mut alias_apps = HashSet::new();
    if !install_only && relevant.iter().any(|(_, score)| *score >= 850) {
        // Exact/strong aliases also find installed apps such as 'npp' => Notepad++.
        // Broad category/unknown queries cannot match any strong alias; do not
        // normalize the entire application index when there is no such package.
        for item in index.iter().filter(|item| item.kind == Kind::Application) {
            let name = normalize(&item.title);
            if relevant
                .iter()
                .any(|(package, score)| *score >= 850 && installed_match(&name, package))
            {
                alias_apps.insert(item.id.clone());
                if !ordinary.iter().any(|existing| existing.id == item.id) {
                    ordinary.push(item.clone());
                }
            }
        }
    }
    let mut strong = Vec::new();
    let mut remaining = Vec::new();
    for item in ordinary {
        if item.kind == Kind::Application
            && (alias_apps.contains(&item.id) || strongly_matches(&item.title, &normalized))
        {
            strong.push(item)
        } else {
            remaining.push(item)
        }
    }
    if apps_only {
        strong.extend(remaining);
        strong.truncate(limit);
        return strong;
    }
    let allowed_prefix = query
        .chars()
        .next()
        .is_none_or(|character| !"/=?:.~!;>-".contains(character));
    let mut installs: Vec<(u32, &Package)> = if allowed_prefix {
        missing
            .iter()
            .filter(|item| item.kind == Kind::Install)
            .filter_map(|item| {
                relevant
                    .iter()
                    .find(|(package, _)| package.package.id.eq_ignore_ascii_case(&item.target))
                    .map(|(package, score)| (*score, package.package))
            })
            .collect()
    } else {
        Vec::new()
    };
    installs.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.name.cmp(right.name))
    });
    // A precise product suggestion should precede fuzzy app/file matches. Broad category
    // suggestions follow ordinary results so a generic query does not fill the launcher.
    let split = installs.partition_point(|(score, _)| *score >= 750);
    strong.extend(
        installs[..split]
            .iter()
            .map(|(_, package)| package_item(package)),
    );
    strong.extend(remaining);
    strong.extend(
        installs[split..]
            .iter()
            .map(|(_, package)| package_item(package)),
    );
    strong.truncate(limit);
    strong
}

/// Search precomputed software actions. Launchable applications with a direct name or
/// alias match lead; removal actions follow, then installation and fuzzy launch matches.
/// '+' remains missing software, '-' is removal only, and '@' is launch only.
pub fn search_with_removals(
    index: &[Item],
    missing: &[Item],
    removals: &[Item],
    query: &str,
    limit: usize,
    weights: &HashMap<String, u32>,
) -> Vec<Item> {
    let limit = limit.min(100);
    if limit == 0 {
        return Vec::new();
    }
    let query = query.trim();
    if let Some(body) = query.strip_prefix('-') {
        let body = body.trim();
        if body.len() > 256 {
            return Vec::new();
        }
        return ranked_removals(removals, body, limit, weights, true);
    }
    if query.starts_with(['+', '@'])
        || query
            .chars()
            .next()
            .is_some_and(|character| "/=?:.~!;>".contains(character))
    {
        return search(index, missing, query, limit, weights);
    }
    let normalized = normalize(query);
    if normalized.chars().count() < 2 || normalized.len() > 256 {
        return search(index, missing, query, limit, weights);
    }
    let ordinary = search(index, missing, query, limit.saturating_mul(2), weights);
    let relevant_packages: Vec<_> = prepared()
        .iter()
        .filter(|package| relevance(package, &normalized).is_some_and(|score| score >= 850))
        .collect();
    let mut direct_apps = Vec::new();
    let mut remaining = Vec::new();
    for item in ordinary {
        let direct = item.kind == Kind::Application
            && (strongly_matches(&item.title, &normalized)
                || relevant_packages
                    .iter()
                    .any(|package| installed_match(&normalize(&item.title), package)));
        if direct {
            direct_apps.push(item);
        } else {
            remaining.push(item);
        }
    }
    direct_apps.extend(ranked_removals(removals, query, limit, weights, false));
    direct_apps.extend(remaining);
    direct_apps.truncate(limit);
    direct_apps
}

#[cfg(test)]
pub fn suggestions(
    index: &[Item],
    installed_names: &[String],
    completed: &HashSet<String>,
    query: &str,
    limit: usize,
    weights: &HashMap<String, u32>,
) -> Vec<Item> {
    search(
        index,
        &missing_items(index, installed_names, completed),
        query,
        limit,
        weights,
    )
}

/// One read-only traversal of machine/user and 32/64-bit registry views yields both
/// presence and removal snapshots. The latter excludes NoRemove/SystemComponent.
/// Called on the catalog worker/helper, never in the typing path.
pub fn installed_inventory() -> (Vec<String>, Vec<String>) {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    inventory_from_roots(&[HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE])
}

/// Determine whether this exact removal target is installed for the machine. This
/// read-only HKLM scan is performed when an action starts, never while typing.
/// User-only, unknown, channel-mismatched and protected entries do not request elevation.
pub fn removal_requires_elevation(target: &str) -> bool {
    use windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    if !(target
        .strip_prefix("name:")
        .is_some_and(valid_registry_name)
        || target
            .strip_prefix("id:")
            .is_some_and(|id| find(id).is_some()))
    {
        return false;
    }
    let (all_names, removable_names) = inventory_from_roots(&[HKEY_LOCAL_MACHINE]);
    removal_requires_elevation_in_inventory(target, &all_names, &removable_names)
}

fn removal_requires_elevation_in_inventory(
    target: &str,
    machine_names: &[String],
    machine_removable_names: &[String],
) -> bool {
    if !removal_is_allowed(target, machine_names, machine_removable_names) {
        return false;
    }
    if target.starts_with("name:") {
        // removal_is_allowed already requires an exact, eligible machine DisplayName.
        return true;
    }
    let Some(id) = target.strip_prefix("id:") else {
        return false;
    };
    let Some(package) = prepared()
        .iter()
        .find(|package| package.package.id.eq_ignore_ascii_case(id))
    else {
        return false;
    };
    // Unlike allowance for UWP/app-index targets, elevation requires positive evidence
    // of the stable product in HKLM. An empty machine snapshot is never sufficient.
    machine_removable_names
        .iter()
        .any(|name| uninstall_match(&normalize(name), package))
}

fn inventory_from_roots(
    roots: &[windows_sys::Win32::System::Registry::HKEY],
) -> (Vec<String>, Vec<String>) {
    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        REG_EXPAND_SZ, REG_SZ,
    };
    let path = wide("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall");
    let display_name = wide("DisplayName");
    let mut names = Vec::new();
    let mut removable = Vec::new();
    let mut seen = HashSet::new();
    let mut removable_seen = HashSet::new();
    let mut blocked = HashSet::new();
    for &root in roots {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let mut key = std::ptr::null_mut();
            // SAFETY: all names/buffers remain valid for these synchronous, read-only calls.
            if unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ | view, &mut key) }
                != ERROR_SUCCESS
            {
                continue;
            }
            let key = RegistryKey(key);
            for index in 0..8192 {
                if names.len() >= 4096 {
                    removable.retain(|name: &String| !blocked.contains(&name.to_lowercase()));
                    return (names, removable);
                }
                let mut name = [0_u16; 256];
                let mut name_length = name.len() as u32;
                let result = unsafe {
                    RegEnumKeyExW(
                        key.0,
                        index,
                        name.as_mut_ptr(),
                        &mut name_length,
                        std::ptr::null(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                };
                if result == ERROR_NO_MORE_ITEMS {
                    break;
                }
                if result != ERROR_SUCCESS || name_length as usize >= name.len() {
                    continue;
                }
                name[name_length as usize] = 0;
                let mut child = std::ptr::null_mut();
                if unsafe { RegOpenKeyExW(key.0, name.as_ptr(), 0, KEY_READ | view, &mut child) }
                    != ERROR_SUCCESS
                {
                    continue;
                }
                let child = RegistryKey(child);
                let mut buffer = [0_u16; 2048];
                let mut byte_length = std::mem::size_of_val(&buffer) as u32;
                let mut kind = 0;
                let result = unsafe {
                    RegQueryValueExW(
                        child.0,
                        display_name.as_ptr(),
                        std::ptr::null(),
                        &mut kind,
                        buffer.as_mut_ptr().cast(),
                        &mut byte_length,
                    )
                };
                if result != ERROR_SUCCESS
                    || !matches!(kind, REG_SZ | REG_EXPAND_SZ)
                    || !byte_length.is_multiple_of(2)
                    || byte_length as usize > std::mem::size_of_val(&buffer)
                {
                    continue;
                }
                let capacity = byte_length as usize / 2;
                let length = buffer[..capacity]
                    .iter()
                    .position(|value| *value == 0)
                    .unwrap_or(capacity);
                let text = String::from_utf16_lossy(&buffer[..length])
                    .trim()
                    .to_string();
                if text.is_empty() {
                    continue;
                }
                let cannot_remove = registry_removal_blocked(child.0, "SystemComponent")
                    || registry_removal_blocked(child.0, "NoRemove");
                if cannot_remove {
                    blocked.insert(text.to_lowercase());
                }
                if !cannot_remove
                    && valid_registry_name(&text)
                    && removable_seen.insert(text.to_lowercase())
                {
                    removable.push(text.clone());
                }
                if seen.insert(text.to_lowercase()) {
                    names.push(text);
                }
            }
        }
    }
    removable.retain(|name| !blocked.contains(&name.to_lowercase()));
    (names, removable)
}

fn registry_removal_blocked(key: windows_sys::Win32::System::Registry::HKEY, name: &str) -> bool {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{RegQueryValueExW, REG_DWORD};
    let name = wide(name);
    let mut value = 0_u32;
    let mut kind = 0;
    let mut length = std::mem::size_of_val(&value) as u32;
    // SAFETY: the read-only key is open and this DWORD buffer remains valid until return.
    let result = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null(),
            &mut kind,
            (&mut value as *mut u32).cast(),
            &mut length,
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return false;
    }
    result != ERROR_SUCCESS
        || kind != REG_DWORD
        || length as usize != std::mem::size_of_val(&value)
        || value != 0
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
struct RegistryKey(windows_sys::Win32::System::Registry::HKEY);
impl Drop for RegistryKey {
    fn drop(&mut self) {
        // SAFETY: this wrapper is only constructed following a successful RegOpenKeyExW.
        unsafe {
            windows_sys::Win32::System::Registry::RegCloseKey(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> Item {
        Item::new(
            format!("app:{name}"),
            name,
            "Application",
            "",
            Kind::Application,
            "app.exe",
        )
    }
    fn results(index: &[Item], registry: &[String], query: &str) -> Vec<Item> {
        suggestions(index, registry, &HashSet::new(), query, 9, &HashMap::new())
    }

    fn managed_results(index: &[Item], registry: &[String], query: &str) -> Vec<Item> {
        search_with_removals(
            index,
            &missing_items(index, registry, &HashSet::new()),
            &removable_items(index, registry),
            query,
            20,
            &HashMap::new(),
        )
    }

    #[test]
    fn absent_firefox_is_a_plain_name_install_result() {
        let result = results(&[], &[], "Firefox");
        assert_eq!(result[0].title, "Firefox");
        assert_eq!(result[0].target, "Mozilla.Firefox");
        assert_eq!(result[0].kind, Kind::Install);
        assert!(result[0]
            .subtitle
            .starts_with("Installer · Navigateur · WinGet"));
    }

    #[test]
    fn firefox_file_does_not_count_as_an_installed_application() {
        let file = Item::new("file:firefox", "Firefox", "", "", Kind::File, "Firefox.txt");
        assert_eq!(results(&[file], &[], "Firefox")[0].kind, Kind::Install);
    }

    #[test]
    fn installed_app_and_registry_versions_suppress_install() {
        let installed = app("Firefox");
        let result = results(std::slice::from_ref(&installed), &[], "Firefox");
        assert_eq!(result[0].kind, Kind::Application);
        assert!(!result.iter().any(|item| item.target == "Mozilla.Firefox"));
        for name in ["Mozilla Firefox ESR (x64 fr)", "Firefox ESR", "FirefoxESR"] {
            assert!(!missing_items(&[], &[name.to_string()], &HashSet::new())
                .iter()
                .any(|item| item.target == "Mozilla.Firefox"));
        }
        assert!(
            !missing_items(&[], &["7-Zip 24.09 (x64 edition)".into()], &HashSet::new())
                .iter()
                .any(|item| item.target == "7zip.7zip")
        );
    }

    #[test]
    fn name_boundaries_do_not_confuse_other_apps() {
        assert!(missing_items(&[app("DiscordCanary")], &[], &HashSet::new())
            .iter()
            .any(|item| item.target == "Discord.Discord"));
        assert!(
            missing_items(&[app("GitHub Desktop")], &[], &HashSet::new())
                .iter()
                .any(|item| item.target == "Git.Git")
        );
        assert!(missing_items(&[app("Notepad")], &[], &HashSet::new())
            .iter()
            .any(|item| item.target == "Notepad++.Notepad++"));
        assert!(
            missing_items(&[app("Firefox Manager")], &[], &HashSet::new())
                .iter()
                .any(|item| item.target == "Mozilla.Firefox")
        );
    }

    #[test]
    fn aliases_and_prefixes_keep_installed_only_search_separate() {
        assert_eq!(
            results(&[], &[], "vscode")[0].target,
            "Microsoft.VisualStudioCode"
        );
        assert_eq!(results(&[], &[], "+ firefox")[0].target, "Mozilla.Firefox");
        assert!(results(&[], &[], "@ firefox").is_empty());
        assert!(results(&[], &[], "f").is_empty());
        assert!(results(&[], &[], "+")
            .iter()
            .all(|item| item.kind == Kind::Install));
        assert!(!results(&[], &[], "+").is_empty());
        let installed = app("Notepad++");
        let result = results(&[installed], &[], "@ npp");
        assert_eq!(result[0].title, "Notepad++");
        assert!(result.iter().all(|item| item.kind == Kind::Application));
    }

    #[test]
    fn product_normalization_preserves_unicode_reference_and_punctuation() {
        fn reference(value: &str) -> String {
            let mut result = String::new();
            let mut spacing = true;
            for character in value.nfd().filter(|c| !is_combining_mark(*c)) {
                if character.is_alphanumeric() || matches!(character, '+' | '#') {
                    result.extend(character.to_lowercase());
                    spacing = false;
                } else if !spacing {
                    result.push(' ');
                    spacing = true;
                }
            }
            if result.ends_with(' ') {
                result.pop();
            }
            result
        }
        for byte in 0..=127 {
            let character = char::from(byte);
            for value in [
                character.to_string(),
                format!("\t{character}A C++ / C#_Notepad++\r\n"),
                format!("ASCII{character}École / Straße / Cœur"),
            ] {
                assert_eq!(normalize(&value), reference(&value), "{value:?}");
            }
        }
        assert_eq!(normalize("Notepad++ / C# / C++"), "notepad++ c# c++");
        // Package identity intentionally differs from core search's ligature
        // aliases: do not broaden product matching during optimization.
        assert_eq!(
            normalize("Cœur Straße Æsir Øresund"),
            "cœur straße æsir øresund"
        );
    }

    #[test]
    fn non_catalog_queries_keep_apps_and_categories_without_strong_aliases() {
        let index = [app("Acme Writer"), app("Acme Reader"), app("Notepad++")];
        let missing = missing_items(&index, &[], &HashSet::new());
        let generic = search(&index, &missing, "@ acme writer", 9, &HashMap::new());
        assert_eq!(generic[0].title, "Acme Writer");
        assert!(generic.iter().all(|item| item.kind == Kind::Application));
        let alias = search(&index, &missing, "@ npp", 9, &HashMap::new());
        assert_eq!(alias[0].title, "Notepad++");
        let category = search(&index, &missing, "+ navigateur", 9, &HashMap::new());
        assert!(!category.is_empty());
        assert!(category.iter().all(|item| item.kind == Kind::Install));
    }

    #[test]
    fn relevant_apps_precede_install_and_install_precedes_fuzzy_matches() {
        let relevant = app("Firefox Manager");
        let fuzzy = app("Fire Fox Toolkit");
        let result = results(&[relevant, fuzzy], &[], "+firefox");
        assert_eq!(result[0].target, "Mozilla.Firefox");
        assert!(result.iter().all(|item| item.kind == Kind::Install));
        let unrelated = app("Fire Fox Toolkit");
        assert!(!core::rank(
            std::slice::from_ref(&unrelated),
            "Firefox",
            9,
            &HashMap::new()
        )
        .is_empty());
        let result = results(&[unrelated], &[], "Firefox");
        assert_eq!(result[0].kind, Kind::Install);
        let result = results(&[app("Firefox Manager")], &[], "Firefox");
        assert_eq!(result[0].title, "Firefox Manager");
        assert_eq!(result[1].target, "Mozilla.Firefox");
    }

    #[test]
    fn completed_install_and_arbitrary_ids_cannot_be_suggested() {
        let completed = HashSet::from(["Mozilla.Firefox".to_string()]);
        assert!(!missing_items(&[], &[], &completed)
            .iter()
            .any(|item| item.target == "Mozilla.Firefox"));
        assert!(find("Mozilla.Firefox; calc.exe").is_none());
        let fake = Item::new("fake", "Firefox", "", "", Kind::Install, "evil.exe");
        assert!(search(&[], &[fake], "Firefox", 9, &HashMap::new()).is_empty());
        for query in [
            "\"Firefox\"",
            "+ \"Firefox\"",
            "+ Firefox; calc.exe",
            "+ Firefox --id unknown",
        ] {
            for item in results(&[], &[], query)
                .iter()
                .filter(|item| item.kind == Kind::Install)
            {
                assert!(find(&item.target).is_some());
                assert!(item.arguments.is_empty());
                assert!(!item.target.contains([';', '"', ' ']));
            }
        }
    }

    #[test]
    fn curated_ids_are_unique_and_reasonably_bounded() {
        assert!((40..=70).contains(&PACKAGES.len()));
        let ids: HashSet<_> = PACKAGES
            .iter()
            .map(|package| package.id.to_ascii_lowercase())
            .collect();
        assert_eq!(ids.len(), PACKAGES.len());
    }

    #[test]
    fn files_are_excluded_from_default_and_at_app_search() {
        let file = Item::new("file:notes", "Notes", "", "", Kind::File, "Notes.txt");
        let index = [file, app("Notes App")];
        for query in ["Notes", "@ Notes", ""] {
            assert!(results(&index, &[], query)
                .iter()
                .all(|item| item.kind != Kind::File));
        }
        assert!(results(&index, &[], "/Firefox")
            .iter()
            .all(|item| item.kind != Kind::Install));
    }

    #[test]
    fn removal_targets_use_known_ids_or_exact_registry_names_only() {
        let file = Item::new("file:firefox", "Firefox", "", "", Kind::File, "Firefox.txt");
        assert!(removable_items(&[file, app("Acme Writer")], &[]).is_empty());
        let removals = removable_items(
            &[app("Firefox"), app("Firefox Private Browsing")],
            &[
                "Mozilla Firefox 145.0 (x64 fr)".into(),
                "7-Zip 24.09 (x64 edition)".into(),
                "Acme Writer 2.4 (x64)".into(),
                "Acme Writer 2.4 (x64)".into(),
                "Editor C++ \"Pro\" 2026".into(),
            ],
        );
        assert_eq!(removals.len(), 4);
        assert_eq!(
            removals
                .iter()
                .filter(|item| item.target == "id:Mozilla.Firefox")
                .count(),
            1
        );
        assert!(removals.iter().any(|item| item.target == "id:7zip.7zip"));
        assert!(removals
            .iter()
            .any(|item| item.target == "name:Acme Writer 2.4 (x64)"));
        assert!(removals
            .iter()
            .any(|item| item.target == "name:Editor C++ \"Pro\" 2026"));
        assert!(removals
            .iter()
            .all(|item| item.kind == Kind::Uninstall && item.arguments.is_empty()));
    }

    #[test]
    fn removal_channels_are_not_mapped_to_the_stable_id() {
        for (display, stable_id) in [
            ("Mozilla Firefox ESR (x64 fr)", "Mozilla.Firefox"),
            ("Google Chrome Canary", "Google.Chrome"),
            (
                "Microsoft Visual Studio Code Insiders",
                "Microsoft.VisualStudioCode",
            ),
            ("Opera GX Stable 120.0", "Opera.Opera"),
        ] {
            let removals = removable_items(&[], &[display.into()]);
            assert_eq!(removals.len(), 1);
            assert_eq!(removals[0].target, format!("name:{display}"));
            assert!(!application_matches_removal(
                &app(display),
                &format!("id:{stable_id}")
            ));
        }
        let esr_only = removable_items(&[app("Firefox")], &["Firefox ESR".into()]);
        assert_eq!(esr_only.len(), 1);
        assert_eq!(esr_only[0].target, "name:Firefox ESR");
        let both = removable_items(
            &[app("Firefox"), app("Firefox ESR")],
            &[
                "Mozilla Firefox 145.0 (x64 fr)".into(),
                "Mozilla Firefox ESR (x64 fr)".into(),
            ],
        );
        assert_eq!(both.len(), 2);
        assert!(both.iter().any(|item| item.target == "id:Mozilla.Firefox"));
        assert!(both
            .iter()
            .any(|item| item.target == "name:Mozilla Firefox ESR (x64 fr)"));
    }

    #[test]
    fn successful_removal_masks_only_its_app_identity() {
        assert!(application_matches_removal(
            &app("Firefox ESR"),
            "name:Mozilla Firefox ESR (x64 fr)"
        ));
        assert!(!application_matches_removal(
            &app("Firefox"),
            "name:Mozilla Firefox ESR (x64 fr)"
        ));
        assert!(application_matches_removal(
            &app("Firefox"),
            "id:Mozilla.Firefox"
        ));
        assert!(!application_matches_removal(
            &app("Firefox ESR"),
            "id:Mozilla.Firefox"
        ));
        assert!(!application_matches_removal(
            &app("Firefox Manager"),
            "id:Mozilla.Firefox"
        ));
        assert!(!application_matches_removal(
            &app("GitHub Desktop"),
            "id:Git.Git"
        ));
        assert!(application_matches_removal(
            &app("Acme Writer"),
            "name:Acme Writer 2.4 (x64)"
        ));
        assert!(!application_matches_removal(
            &app("Acme Writer Manager"),
            "name:Acme Writer"
        ));
        assert!(!application_matches_removal(
            &app("Editor C"),
            "name:Editor C++"
        ));
        assert!(!application_matches_removal(
            &app("Editor C"),
            "name:Editor C#"
        ));
        let file = Item::new("file:acme", "Acme Writer", "", "", Kind::File, "acme.txt");
        assert!(!application_matches_removal(&file, "name:Acme Writer"));
    }

    #[test]
    fn software_search_does_not_mix_steam_with_other_products_or_their_removal() {
        let index = [
            app("Steam"),
            app("Steam Support Center"),
            app("Stream Deck"),
            app("StreamFab (x64)"),
            app("Register GameSpy Arcade"),
            app("Désinstaller Telegram"),
            app("Désinstaller Alice Madness Returns - The Complete Collection"),
            app("Microsoft Teams"),
        ];
        let registry = [
            "Steam".into(),
            "Telegram Desktop".into(),
            "Elgato Stream Deck".into(),
            "Alice Madness Returns - The Complete Collection".into(),
        ];
        let hits = managed_results(&index, &registry, "steam");
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].title, "Steam");
        assert_eq!(hits[1].title, "Steam Support Center");
        assert_eq!(hits[2].kind, Kind::Uninstall);
        assert_eq!(hits[2].target, "id:Valve.Steam");
        let launch_only = managed_results(&index, &registry, "@ steam");
        assert_eq!(launch_only.len(), 2);
        assert!(launch_only
            .iter()
            .all(|item| item.kind == Kind::Application));
    }

    #[test]
    fn launch_results_lead_removal_results_then_fuzzy_launches() {
        let index = [app("Firefox"), app("Fire Fox Toolkit")];
        let results = managed_results(&index, &[], "Firefox");
        assert_eq!(results[0].title, "Firefox");
        assert_eq!(results[0].kind, Kind::Application);
        assert_eq!(results[1].target, "id:Mozilla.Firefox");
        assert_eq!(results[1].kind, Kind::Uninstall);
        assert_eq!(results[2].title, "Fire Fox Toolkit");
        assert!(results.iter().all(|item| item.kind != Kind::File));
        let registry_only = managed_results(
            &[app("Acme Writer"), app("Ace More Writer")],
            &["Acme Writer 2.4".into()],
            "Acme",
        );
        assert_eq!(registry_only[0].kind, Kind::Application);
        assert_eq!(registry_only[1].target, "name:Acme Writer 2.4");
    }

    #[test]
    fn management_prefixes_keep_actions_separate_and_browse_removals() {
        let index = [app("Firefox"), app("Notepad++")];
        let registry = ["Acme Writer 2.4".into()];
        for query in ["-", "- firefox", "- npp", "- Acme"] {
            let result = managed_results(&index, &registry, query);
            assert!(!result.is_empty());
            assert!(result.iter().all(|item| item.kind == Kind::Uninstall));
        }
        assert!(managed_results(&index, &registry, "@ npp")
            .iter()
            .all(|item| item.kind == Kind::Application));
        assert!(managed_results(&index, &registry, "+")
            .iter()
            .all(|item| item.kind == Kind::Install));
        for query in [
            "", "f", "/Firefox", "=Firefox", "?Firefox", ">Firefox", ";", ":Firefox",
        ] {
            assert!(managed_results(&index, &registry, query)
                .iter()
                .all(|item| item.kind != Kind::Uninstall));
        }
    }

    #[test]
    fn removal_queries_cannot_replace_the_cached_execution_target() {
        let registry = ["Editor C++ \"Pro\" 2026".into()];
        for query in ["- Editor", "- \"Editor\"", "Editor"] {
            let result = managed_results(&[], &registry, query);
            assert!(!result.is_empty());
            assert_eq!(result[0].target, "name:Editor C++ \"Pro\" 2026");
            assert!(result[0].arguments.is_empty());
        }
        let invalid = [
            Item::new(
                "fake:id",
                "Firefox",
                "",
                "",
                Kind::Uninstall,
                "id:Mozilla.Firefox; calc.exe",
            ),
            Item::new(
                "fake:name",
                "Firefox",
                "",
                "",
                Kind::Uninstall,
                "name:Different Name",
            ),
            Item::new(
                "fake:control",
                "Firefox\ncalc.exe",
                "",
                "",
                Kind::Uninstall,
                "name:Firefox\ncalc.exe",
            ),
        ];
        assert!(search_with_removals(&[], &[], &invalid, "-", 9, &HashMap::new()).is_empty());
        assert!(removable_items(&[], &[String::new(), "Bad\nName".into()]).is_empty());
    }

    #[test]
    fn protected_registered_products_cannot_reappear_through_app_shortcuts() {
        let all = [
            "Mozilla Firefox 145.0 (x64 fr)".into(),
            "Acme Writer 2.4".into(),
        ];
        assert!(
            removable_items_with_inventory(&[app("Firefox"), app("Acme Writer")], &all, &[])
                .is_empty()
        );
        assert!(!removal_is_allowed("id:Mozilla.Firefox", &all, &[]));
        assert!(!removal_is_allowed("name:Acme Writer 2.4", &all, &[]));
        assert!(removal_is_allowed("name:Acme Writer 2.4", &all, &all));
        let two_versions = [
            "Mozilla Firefox 145.0".into(),
            "Mozilla Firefox 144.0".into(),
        ];
        let only_one_removable = ["Mozilla Firefox 145.0".into()];
        assert!(!removal_is_allowed(
            "id:Mozilla.Firefox",
            &two_versions,
            &only_one_removable
        ));
        assert!(removal_is_allowed("id:Microsoft.WindowsTerminal", &[], &[]));
        assert!(!removal_is_allowed("id:unknown.Package", &[], &[]));
        assert!(!removal_is_allowed("name:arbitrary shortcut", &[], &[]));
    }

    #[test]
    fn powershell_major_version_and_typing_aliases_are_not_ambiguous_identities() {
        let modern = removable_items(&[], &["PowerShell 7.4.5.0 (x64)".into()]);
        assert_eq!(modern[0].target, "id:Microsoft.PowerShell");
        assert!(application_matches_removal(
            &app("PowerShell 7"),
            "name:PowerShell 7.4.5.0 (x64)"
        ));
        let legacy = removable_items(&[], &["PowerShell 5.1".into()]);
        assert_eq!(legacy[0].target, "name:PowerShell 5.1");
        assert!(!application_matches_removal(
            &app("PowerShell 5.1"),
            "id:Microsoft.PowerShell"
        ));
        assert!(removable_items(&[app("FF"), app("terminal"), app("NPP")], &[]).is_empty());
        assert!(!application_matches_removal(&app("FF"), "name:Firefox"));
    }

    #[test]
    fn elevation_requires_positive_eligible_machine_evidence() {
        let machine = ["Google Chrome 145.0.1 (x64)".into()];
        assert!(removal_requires_elevation_in_inventory(
            "id:Google.Chrome",
            &machine,
            &machine
        ));
        // No machine entry means unknown or HKCU-only, even for a catalog ID.
        assert!(!removal_requires_elevation_in_inventory(
            "id:Google.Chrome",
            &[],
            &[]
        ));
        assert!(!removal_requires_elevation_in_inventory(
            "id:Microsoft.WindowsTerminal",
            &[],
            &[]
        ));
        assert!(!removal_requires_elevation_in_inventory(
            "id:Unknown.Package",
            &machine,
            &machine
        ));
        assert!(!removal_requires_elevation_in_inventory(
            "Google Chrome",
            &machine,
            &machine
        ));
        assert!(!removal_requires_elevation_in_inventory(
            "id:Google.Chrome",
            &machine,
            &[]
        ));
        let both_versions = [
            "Google Chrome 145.0.1 (x64)".into(),
            "Google Chrome 144.0.1 (x64)".into(),
        ];
        assert!(!removal_requires_elevation_in_inventory(
            "id:Google.Chrome",
            &both_versions,
            &machine
        ));
    }

    #[test]
    fn elevation_uses_exact_display_names_and_separate_channels() {
        let machine = [
            "Google Chrome Canary".into(),
            "Editor C++ \"Pro\" 2026".into(),
        ];
        assert!(!removal_requires_elevation_in_inventory(
            "id:Google.Chrome",
            &machine,
            &machine
        ));
        assert!(removal_requires_elevation_in_inventory(
            "name:Google Chrome Canary",
            &machine,
            &machine
        ));
        assert!(!removal_requires_elevation_in_inventory(
            "name:Google Chrome",
            &machine,
            &machine
        ));
        assert!(removal_requires_elevation_in_inventory(
            "name:Editor C++ \"Pro\" 2026",
            &machine,
            &machine
        ));
        assert!(!removal_requires_elevation_in_inventory(
            "name:Editor C",
            &machine,
            &machine
        ));
        assert!(!removal_requires_elevation_in_inventory(
            "name:Editor C++ \"Pro\" 2026",
            &machine,
            &[]
        ));
        assert!(!removal_requires_elevation_in_inventory(
            "name:Editor C++ \"Pro\" 2026",
            &[],
            &[]
        ));
        let studio = ["Microsoft Visual Studio Code Insiders".into()];
        assert!(!removal_requires_elevation_in_inventory(
            "id:Microsoft.VisualStudioCode",
            &studio,
            &studio
        ));
    }
}
