//! Curated WinGet suggestions. Searching is entirely local; installation is handled by the UI.
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
    for character in value.nfd().filter(|c| !is_combining_mark(*c)) {
        // '+' carries identity in Notepad++; stripping it would confuse the built-in Notepad.
        if character.is_alphanumeric() || character == '+' {
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
        format!(
            "Installer · {} · WinGet · Entrée pour installer",
            package.category
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
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let name_match = package.names.iter().any(|name| {
        tokens.iter().all(|query_token| {
            name.split_whitespace()
                .any(|token| token.starts_with(query_token))
        })
    });
    if name_match {
        return Some(750);
    }
    // Category words may accompany a product name, or be searched on their own.
    let all_match = tokens.iter().all(|query_token| {
        package.names.iter().any(|name| {
            name.split_whitespace()
                .any(|token| token.starts_with(query_token))
        }) || package
            .category
            .split_whitespace()
            .any(|token| token.starts_with(query_token))
    });
    all_match.then_some(500)
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
    if !install_only {
        // Exact/strong aliases also find installed apps such as 'npp' => Notepad++.
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
        .is_none_or(|character| !"/=?:.~!;>".contains(character));
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

/// Read only uninstall DisplayNames from both machine/user and 32/64-bit registry
/// views. Called on the catalog worker, never in the typing path.
pub fn installed_names() -> Vec<String> {
    use windows_sys::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_SZ,
    };
    let path = wide("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall");
    let display_name = wide("DisplayName");
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
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
                    return names;
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
                if !text.is_empty() && seen.insert(normalize(&text)) {
                    names.push(text);
                }
            }
        }
    }
    names
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
}
