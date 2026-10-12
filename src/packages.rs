//! Local software management suggestions. Searching never runs WinGet or reads the registry.
//! Installation catalogue generated from a pinned WinUtil snapshot; see catalog/ and notices.
//! Runtime uses only compact local names/IDs, never the upstream JSON or installer scripts.
use crate::core::{self, Item, Kind};
use crate::unicode;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Source {
    Winget,
    MicrosoftStore,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Winget => "winget",
            Self::MicrosoftStore => "msstore",
        }
    }
}

pub struct Package {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub aliases: &'static [&'static str],
    pub source: Source,
    pub install_eligible: bool,
    pub removal_eligible: bool,
}

macro_rules! package {
    ($id:literal, $name:literal, $category:literal, [$($alias:literal),* $(,)?], $source:ident, $install:literal, $removal:literal) => {
        Package { id: $id, name: $name, category: $category, aliases: &[$($alias),*], source: Source::$source, install_eligible: $install, removal_eligible: $removal }
    };
}

include!("package_catalog.rs");

/// Only canonical, curated IDs are accepted. Returns the canonical spelling for execution.
pub fn find(id: &str) -> Option<&'static Package> {
    PACKAGES
        .iter()
        .find(|package| package.id.eq_ignore_ascii_case(id))
}

/// Preserve the reviewed removal identities independently of installation suggestions.
pub fn find_removal(id: &str) -> Option<&'static Package> {
    find(id).filter(|package| package.removal_eligible)
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
    for character in unicode::decomposed_without_marks(value).chars() {
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
    package_identity_names(package).any(|alias| {
        let Some(suffix) = name.strip_prefix(alias) else {
            return false;
        };
        if suffix.is_empty() {
            return true;
        }
        // Accept version/architecture suffixes, but never concatenate arbitrary app names.
        if !suffix_boundary(alias, suffix) {
            return false;
        }
        suffix
            .split_whitespace()
            .all(|token| installed_suffix(package.package.id, token))
    })
}

fn package_identity_names(package: &PreparedPackage) -> impl Iterator<Item = &str> {
    package.names.iter().map(String::as_str).chain(
        // Keep the historical joined ESR DisplayName as presence/channel evidence,
        // without making an explicit ESR search refer to the stable install ID.
        (package.package.id == "Mozilla.Firefox").then_some("firefoxesr"),
    )
}

fn suffix_boundary(alias: &str, suffix: &str) -> bool {
    suffix.starts_with(' ')
        || (suffix.as_bytes().first().is_some_and(u8::is_ascii_digit)
            && !alias.as_bytes().last().is_some_and(u8::is_ascii_digit))
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
        "Amazon.Corretto.8.JDK" | "Amazon.Corretto.21.JDK" | "Amazon.Corretto.25.JDK" => {
            matches!(token, "jdk" | "lts")
        }
        "TeamSpeakSystems.TeamSpeakClient.Beta.6" => token == "beta",
        _ => false,
    }
}

fn package_item(package: &Package) -> Item {
    Item::new(
        format!("install:{}", package.id),
        package.name,
        crate::i18n::fmt(
            match package.source {
                Source::Winget => "Installer · {category} · WinGet · Entrée pour installer",
                Source::MicrosoftStore => {
                    "Installer · {category} · Microsoft Store · Entrée pour installer"
                }
            },
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
    missing_items_with_absent(index, installed_names, completed, &HashSet::new())
}

/// Absence evidence comes from the catalog worker, never the search path. Keep
/// raw registry inventory available independently for protected removal rules.
pub fn missing_items_with_absent(
    index: &[Item],
    installed_names: &[String],
    completed: &HashSet<String>,
    confirmed_absent: &HashSet<String>,
) -> Vec<Item> {
    let names: Vec<String> = index
        .iter()
        .filter(|item| item.kind == Kind::Application)
        .map(|item| normalize(&item.title))
        .chain(installed_names.iter().map(|name| normalize(name)))
        .collect();
    prepared()
        .iter()
        .filter(|package| package.package.install_eligible)
        .filter(|package| {
            !completed
                .iter()
                .any(|id| id.eq_ignore_ascii_case(package.package.id))
                && (confirmed_absent.contains(package.package.id)
                    || !names.iter().any(|name| installed_match(name, package)))
        })
        .map(|package| package_item(package.package))
        .collect()
}

/// Only Chrome has a bounded executable-presence policy. An orphan installer
/// registration alone must not hide installation; uncertainty never proves
/// absence. This function is restricted to catalog/installer worker threads.
pub fn installed_executable_presence(id: &str) -> Option<bool> {
    if !id.eq_ignore_ascii_case("Google.Chrome") {
        return None;
    }
    let (registered, mut uncertain) = chrome_registered_paths();
    let mut candidates = Vec::with_capacity(21);
    for variable in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        match std::env::var_os(variable).map(PathBuf::from) {
            Some(root) if root.is_absolute() => {
                for channel in [
                    "Chrome",
                    "Chrome Beta",
                    "Chrome Dev",
                    "Chrome SxS",
                    "ChromeBeta",
                    "ChromeDev",
                    "ChromeSxS",
                ] {
                    candidates.push(
                        root.join("Google")
                            .join(channel)
                            .join("Application/chrome.exe"),
                    );
                }
            }
            _ => uncertain = true,
        }
    }
    executable_presence_from_paths(&registered, &candidates, uncertain, executable_file_present)
}

fn executable_file_present(path: &Path) -> std::io::Result<bool> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(crate) fn confirmed_absent_packages(installed_names: &[String]) -> HashSet<String> {
    let mut absent = HashSet::new();
    let chrome = prepared()
        .iter()
        .find(|package| package.package.id == "Google.Chrome")
        .unwrap();
    if installed_names
        .iter()
        .any(|name| installed_match(&normalize(name), chrome))
        && installed_executable_presence("Google.Chrome") == Some(false)
    {
        absent.insert("Google.Chrome".into());
    }
    absent
}

fn registered_executable_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    let value = if value.starts_with('"') {
        value.strip_prefix('"')?.strip_suffix('"')?
    } else {
        value
    };
    if value.is_empty() || value.contains('"') || value.chars().any(char::is_control) {
        return None;
    }
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

fn executable_presence_from_paths(
    registered: &[String],
    candidates: &[PathBuf],
    mut uncertain: bool,
    mut probe: impl FnMut(&Path) -> std::io::Result<bool>,
) -> Option<bool> {
    let mut registered_path_observed = false;
    for value in registered {
        let Some(path) = registered_executable_path(value) else {
            uncertain = true;
            continue;
        };
        registered_path_observed = true;
        match probe(&path) {
            Ok(true) => return Some(true),
            Ok(false) => {}
            Err(_) => uncertain = true,
        }
    }
    for path in candidates {
        match probe(path) {
            Ok(true) => return Some(true),
            Ok(false) => {}
            Err(_) => uncertain = true,
        }
    }
    (registered_path_observed && !uncertain).then_some(false)
}

fn chrome_registered_paths() -> (Vec<String>, bool) {
    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
    };
    use windows_sys::Win32::System::Registry::{
        RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
        KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_SZ,
    };
    let key_name = wide("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\chrome.exe");
    let mut paths = Vec::new();
    let mut uncertain = false;
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let mut key = std::ptr::null_mut();
            let status =
                unsafe { RegOpenKeyExW(root, key_name.as_ptr(), 0, KEY_READ | view, &mut key) };
            if matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
                continue;
            }
            if status != ERROR_SUCCESS {
                uncertain = true;
                continue;
            }
            let key = RegistryKey(key);
            let mut bytes = [0u16; 2048];
            let mut length = std::mem::size_of_val(&bytes) as u32;
            let mut kind = 0;
            let status = unsafe {
                RegQueryValueExW(
                    key.0,
                    std::ptr::null(),
                    std::ptr::null(),
                    &mut kind,
                    bytes.as_mut_ptr().cast(),
                    &mut length,
                )
            };
            if status != ERROR_SUCCESS
                || !matches!(kind, REG_SZ | REG_EXPAND_SZ)
                || length == 0
                || !length.is_multiple_of(2)
                || length as usize > std::mem::size_of_val(&bytes)
            {
                uncertain = true;
                continue;
            }
            let bytes = &bytes[..length as usize / 2];
            let end = bytes
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(bytes.len());
            let Ok(value) = String::from_utf16(&bytes[..end]) else {
                uncertain = true;
                continue;
            };
            let value = if kind == REG_EXPAND_SZ {
                let Some(value) = expand_registered_environment(&value) else {
                    uncertain = true;
                    continue;
                };
                value
            } else {
                value
            };
            paths.push(value);
        }
    }
    (paths, uncertain)
}

fn expand_registered_environment(value: &str) -> Option<String> {
    let mut result = String::new();
    let mut remaining = value;
    while let Some(start) = remaining.find('%') {
        result.push_str(&remaining[..start]);
        let after = &remaining[start + 1..];
        let end = after.find('%')?;
        result.push_str(&std::env::var(&after[..end]).ok()?);
        remaining = &after[end + 1..];
    }
    result.push_str(remaining);
    Some(result)
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
    package.package.removal_eligible
        && package_channel(name, package) == "stable"
        && family_identity_match(name, package)
}

fn family_identity_match(name: &str, package: &PreparedPackage) -> bool {
    if !package.package.removal_eligible {
        return false;
    }
    package_identity_names(package).any(|alias| {
        // Typing aliases are useful for search, but names such as FF/terminal are
        // insufficient evidence for a destructive action on a specific package.
        if alias != package.primary_name
            && (alias.chars().count() <= 3 || matches!(alias, "terminal" | "pwsh" | "gh desktop"))
        {
            return false;
        }
        let Some(suffix) = name.strip_prefix(alias) else {
            return false;
        };
        if suffix.is_empty() {
            return true;
        }
        if !suffix_boundary(alias, suffix) {
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
    for package in prepared()
        .iter()
        .filter(|package| package.package.removal_eligible)
    {
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
                .find(|package| installed_match(&normalized, package))
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
    let Some(package) = prepared().iter().find(|package| {
        package.package.removal_eligible && package.package.id.eq_ignore_ascii_case(id)
    }) else {
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
    if !suffix_boundary(shorter, suffix) {
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
        return find_removal(id).is_some();
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
            .filter(|package| package.install_eligible)
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
                    .find(|(package, _)| {
                        package.package.install_eligible
                            && package.package.id.eq_ignore_ascii_case(&item.target)
                    })
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
    direct_apps.extend(
        ranked_removals(removals, query, limit, weights, false)
            .into_iter()
            .filter(|item| {
                // A confirmed-absent product can retain an orphan removal entry.
                // Its cached install action is the useful default; explicit '-'
                // queries above still expose the raw entry for residue cleanup.
                !item.target.strip_prefix("id:").is_some_and(|id| {
                    missing.iter().any(|candidate| {
                        candidate.kind == Kind::Install && candidate.target.eq_ignore_ascii_case(id)
                    })
                })
            }),
    );
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
            .is_some_and(|id| find_removal(id).is_some()))
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
    let Some(package) = prepared().iter().find(|package| {
        package.package.removal_eligible && package.package.id.eq_ignore_ascii_case(id)
    }) else {
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
    use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

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
    fn executable_absence_requires_registered_paths_and_complete_evidence() {
        let valid = r#""C:\Custom Chrome\Application\chrome.exe""#.to_string();
        let candidates = [PathBuf::from(r"C:\Other Chrome\chrome.exe")];
        assert_eq!(
            executable_presence_from_paths(
                std::slice::from_ref(&valid),
                &candidates,
                false,
                |_| Ok(false)
            ),
            Some(false)
        );
        assert_eq!(
            executable_presence_from_paths(&[], &candidates, false, |_| Ok(false)),
            None
        );
        assert_eq!(
            executable_presence_from_paths(
                std::slice::from_ref(&valid),
                &candidates,
                true,
                |_| Ok(false)
            ),
            None
        );
        for malformed in [
            "relative\\chrome.exe",
            r#""C:\Chrome\chrome.exe" --flag"#,
            "",
            "C:\\bad\nchrome.exe",
        ] {
            let paths = [valid.clone(), malformed.into()];
            assert_eq!(
                executable_presence_from_paths(&paths, &candidates, false, |_| Ok(false)),
                None,
                "{malformed:?}"
            );
        }
        assert_eq!(
            executable_presence_from_paths(&[valid], &candidates, false, |_| Err(
                std::io::ErrorKind::PermissionDenied.into()
            )),
            None
        );
        assert_eq!(installed_executable_presence("Notepad++.Notepad++"), None);
        assert_eq!(installed_executable_presence("Unknown.Package"), None);
    }

    #[test]
    fn registered_custom_executable_and_variants_prove_presence() {
        use std::io::Write;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let fixture = Fixture(std::env::temp_dir().join(format!(
            "Super Menu custom Chrome-{}-{nonce}.exe",
            std::process::id()
        )));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&fixture.0)
            .unwrap();
        file.write_all(b"Disposable presence fixture; never executed.")
            .unwrap();
        drop(file);
        let registered = [
            "relative\\chrome.exe".into(),
            format!("\"{}\"", fixture.0.display()),
        ];
        assert_eq!(
            executable_presence_from_paths(&registered, &[], true, executable_file_present),
            Some(true)
        );
        // An installed standard/variant executable also wins over incomplete
        // registry evidence; an App Paths entry is required only for absence.
        assert_eq!(
            executable_presence_from_paths(
                &[],
                std::slice::from_ref(&fixture.0),
                true,
                executable_file_present
            ),
            Some(true)
        );
    }

    #[test]
    fn orphan_chrome_registration_allows_install_without_weakening_removal() {
        let index = [app("Google Chrome"), app("Notepad++")];
        let registry = ["Google Chrome".into(), "Notepad++".into()];
        let absent = HashSet::from(["Google.Chrome".to_string()]);
        let ordinary = missing_items(&index, &registry, &HashSet::new());
        assert!(!ordinary.iter().any(|item| item.target == "Google.Chrome"));
        let missing = missing_items_with_absent(&index, &registry, &HashSet::new(), &absent);
        assert!(missing.iter().any(|item| item.target == "Google.Chrome"));
        assert!(!missing
            .iter()
            .any(|item| item.target == "Notepad++.Notepad++"));
        for query in ["chrome", "+ chrome"] {
            let results = search(&[], &missing, query, 9, &HashMap::new());
            assert!(
                results
                    .iter()
                    .any(|item| item.kind == Kind::Install && item.target == "Google.Chrome"),
                "{query}"
            );
        }
        let removals = removable_items(&index, &registry);
        for query in ["chrome", "+ chrome"] {
            let results = search_with_removals(&[], &missing, &removals, query, 9, &HashMap::new());
            assert_eq!(
                results
                    .iter()
                    .map(|item| item.id.as_str())
                    .collect::<Vec<_>>(),
                ["install:Google.Chrome"],
                "{query}"
            );
        }
        let explicit =
            search_with_removals(&[], &missing, &removals, "- chrome", 9, &HashMap::new());
        assert_eq!(
            explicit
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["uninstall:id:Google.Chrome"]
        );
        assert!(!removal_is_allowed("id:Google.Chrome", &registry, &[]));
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
        assert_eq!(PACKAGES.len(), 246);
        assert_eq!(
            PACKAGES
                .iter()
                .filter(|package| package.install_eligible)
                .count(),
            245
        );
        assert_eq!(
            PACKAGES
                .iter()
                .filter(|package| package.removal_eligible)
                .count(),
            52
        );
        let ids: HashSet<_> = PACKAGES
            .iter()
            .map(|package| package.id.to_ascii_lowercase())
            .collect();
        assert_eq!(ids.len(), PACKAGES.len());
    }

    #[test]
    fn pinned_winutil_snapshot_is_complete_and_sources_are_unambiguous() {
        use sha2::{Digest, Sha256};
        // Provenance and upstream descriptions are test inputs only. Neither JSON
        // is compiled into the release binary or downloaded during these tests.
        let snapshot = include_bytes!("../catalog/winutil-applications.json");
        let metadata: serde_json::Value =
            serde_json::from_str(include_str!("../catalog/winutil-source.json")).unwrap();
        let applications: serde_json::Value = serde_json::from_slice(snapshot).unwrap();
        let applications = applications.as_object().unwrap();
        assert_eq!(applications.len(), 236);
        assert_eq!(metadata["application_count"].as_u64(), Some(236));
        let revision = metadata["commit"].as_str().unwrap();
        assert_eq!(revision.len(), 40);
        assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            metadata["source_url"].as_str().unwrap(),
            format!("https://github.com/ChrisTitusTech/winutil/blob/{revision}/config/applications.json")
        );
        assert!(format!("{:x}", Sha256::digest(snapshot))
            .eq_ignore_ascii_case(metadata["source_sha256"].as_str().unwrap()));
        let mut covered = HashSet::new();
        for (key, entry) in applications {
            let upstream_id = entry["winget"].as_str().unwrap();
            let (id, source) = upstream_id
                .strip_prefix("msstore:")
                .map_or((upstream_id, Source::Winget), |id| {
                    (id, Source::MicrosoftStore)
                });
            assert!(
                covered.insert(id.to_ascii_lowercase()),
                "duplicate upstream ID for {key}"
            );
            let package =
                find(id).unwrap_or_else(|| panic!("missing WinUtil application {key}: {id}"));
            assert_eq!(package.source, source, "{key}");
            assert!(package.install_eligible, "{key}");
            assert!(
                !package.id.contains([';', ':', ' ', '\"', '\n', '\r']),
                "{key}"
            );
            assert_eq!(
                PACKAGES
                    .iter()
                    .filter(|candidate| candidate.id.eq_ignore_ascii_case(id))
                    .count(),
                1,
                "ambiguous canonical ID for {key}"
            );
        }
        assert_eq!(covered.len(), 236);
    }

    #[test]
    fn expanded_catalog_searches_cli_aliases_and_explicit_variants() {
        for (query, id) in [
            ("uv", "astral-sh.uv"),
            ("gh", "GitHub.cli"),
            ("syncthing", "Syncthing.Syncthing"),
            ("teamspeak6", "TeamSpeakSystems.TeamSpeakClient.Beta.6"),
            ("firefoxesr", "Mozilla.Firefox.ESR"),
            ("firefox esr", "Mozilla.Firefox.ESR"),
            ("nodejslts", "OpenJS.NodeJS.LTS"),
        ] {
            for query in [query.to_string(), format!("+ {query}")] {
                let hits = results(&[], &[], &query);
                assert_eq!(hits[0].target, id, "{query}");
                assert_eq!(hits[0].kind, Kind::Install, "{query}");
                assert_eq!(
                    hits.iter().filter(|item| item.target == id).count(),
                    1,
                    "{query}"
                );
                if id == "Mozilla.Firefox.ESR" {
                    assert!(
                        !hits.iter().any(|item| item.target == "Mozilla.Firefox"),
                        "{query}"
                    );
                }
            }
        }
        assert!(results(&[], &[], "@ uv").is_empty());
        assert!(results(&[], &["uv 0.9.0 (x64)".into()], "+ uv").is_empty());
    }

    #[test]
    fn installation_eligibility_and_store_labels_do_not_change_legacy_removal() {
        let missing = missing_items(&[], &[], &HashSet::new());
        assert!(!missing.iter().any(|item| item.target == "GIMP.GIMP"));
        assert!(missing.iter().any(|item| item.target == "GIMP.GIMP.3"));
        assert!(find_removal("GIMP.GIMP").is_some());
        assert!(find_removal("GIMP.GIMP.3").is_none());
        for query in ["gimp", "+ gimp"] {
            let hits = results(&[], &[], query);
            assert_eq!(hits[0].target, "GIMP.GIMP.3");
            assert!(!hits.iter().any(|item| item.target == "GIMP.GIMP"));
        }
        let legacy = package_item(find("GIMP.GIMP").unwrap());
        for query in ["+", "+ gimp"] {
            assert!(search(
                &[],
                std::slice::from_ref(&legacy),
                query,
                9,
                &HashMap::new()
            )
            .is_empty());
        }
        for (query, id) in [("chatgpt", "9NT1R1C2HH7J"), ("whatsapp", "9NKSQGP7F2NH")] {
            let hits = results(&[], &[], query);
            assert_eq!(hits[0].target, id);
            assert_eq!(find(id).unwrap().source.as_str(), "msstore");
            assert!(hits[0].subtitle.contains("Microsoft Store"));
            assert!(!hits[0].subtitle.contains("WinGet"));
        }
        let chrome = package_item(find("Google.Chrome").unwrap());
        assert!(chrome
            .subtitle
            .starts_with("Installer · Navigateur · WinGet"));
    }

    #[test]
    fn installed_major_versions_and_architectures_do_not_hide_other_packages() {
        for (display, present_id, absent_ids) in [
            (
                "Microsoft Windows Desktop Runtime 8.0.10 (x64)",
                "Microsoft.DotNet.DesktopRuntime.8",
                vec![
                    "Microsoft.DotNet.DesktopRuntime.6",
                    "Microsoft.DotNet.DesktopRuntime.9",
                    "Microsoft.DotNet.DesktopRuntime.10",
                ],
            ),
            (
                "Amazon Corretto JDK 21.0.5 (LTS) (x64)",
                "Amazon.Corretto.21.JDK",
                vec!["Amazon.Corretto.8.JDK", "Amazon.Corretto.25.JDK"],
            ),
            (
                "TeamSpeak 3 Client 3.6.2 (x64)",
                "TeamSpeakSystems.TeamSpeakClient",
                vec!["TeamSpeakSystems.TeamSpeakClient.Beta.6"],
            ),
            (
                "TeamSpeak 6 Client Beta 6.0.0 (x64)",
                "TeamSpeakSystems.TeamSpeakClient.Beta.6",
                vec!["TeamSpeakSystems.TeamSpeakClient"],
            ),
            (
                "Microsoft Visual C++ 2015-2022 Redistributable (x64) 14.44",
                "Microsoft.VCRedist.2015+.x64",
                vec!["Microsoft.VCRedist.2015+.x86"],
            ),
            (
                "Microsoft Visual C++ 2015-2022 Redistributable (x86) 14.44",
                "Microsoft.VCRedist.2015+.x86",
                vec!["Microsoft.VCRedist.2015+.x64"],
            ),
            (
                "Node.js LTS 22.0.0 (x64)",
                "OpenJS.NodeJS.LTS",
                vec!["OpenJS.NodeJS"],
            ),
            (
                "Node.js 23.0.0 (x64)",
                "OpenJS.NodeJS",
                vec!["OpenJS.NodeJS.LTS"],
            ),
        ] {
            let missing = missing_items(&[], &[display.into()], &HashSet::new());
            assert!(
                !missing.iter().any(|item| item.target == present_id),
                "{display}"
            );
            for id in absent_ids {
                assert!(
                    missing.iter().any(|item| item.target == id),
                    "{display} hides {id}"
                );
            }
        }
        let stable = missing_items(
            &[],
            &["Mozilla Firefox 145.0 (x64 fr)".into()],
            &HashSet::new(),
        );
        assert!(stable
            .iter()
            .any(|item| item.target == "Mozilla.Firefox.ESR"));
        let esr = missing_items(
            &[],
            &["Mozilla Firefox ESR 140.0 (x64 fr)".into()],
            &HashSet::new(),
        );
        assert!(!esr.iter().any(|item| matches!(
            item.target.as_str(),
            "Mozilla.Firefox" | "Mozilla.Firefox.ESR"
        )));
    }

    #[test]
    fn numeric_alias_boundaries_reject_other_major_versions() {
        for (display, id) in [
            (
                "Microsoft Windows Desktop Runtime 80.0 (x64)",
                "Microsoft.DotNet.DesktopRuntime.8",
            ),
            ("Amazon Corretto JDK 210.0 (x64)", "Amazon.Corretto.21.JDK"),
            ("TeamSpeak 30 Client", "TeamSpeakSystems.TeamSpeakClient"),
            ("TeamSpeak60", "TeamSpeakSystems.TeamSpeakClient.Beta.6"),
        ] {
            let missing = missing_items(&[], &[display.into()], &HashSet::new());
            assert!(missing.iter().any(|item| item.target == id), "{display}");
        }
        assert!(suffix_boundary("runtime 8", " 0 1"));
        assert!(!suffix_boundary("runtime 8", "0"));
        assert!(version_name_match("acme", "acme2 4"));
        assert!(!version_name_match("runtime 8", "runtime 80"));
    }

    #[test]
    fn imported_install_ids_cannot_expand_destructive_identity_matching() {
        for package in PACKAGES.iter().filter(|package| !package.removal_eligible) {
            let id_target = format!("id:{}", package.id);
            let registry = [package.name.to_string()];
            let application = app(package.name);
            assert!(find_removal(package.id).is_none(), "{}", package.id);
            assert!(
                !removal_is_allowed(&id_target, &registry, &registry),
                "{}",
                package.id
            );
            assert!(
                !registry_name_matches_target(&id_target, package.name),
                "{}",
                package.id
            );
            assert!(
                !application_matches_removal(&application, &id_target),
                "{}",
                package.id
            );
            assert!(
                !removal_requires_elevation_in_inventory(&id_target, &registry, &registry),
                "{}",
                package.id
            );
            let forged = Item::new("forged", package.name, "", "", Kind::Uninstall, &id_target);
            assert!(
                search_with_removals(&[], &[], &[forged], "-", 9, &HashMap::new()).is_empty(),
                "{}",
                package.id
            );
            assert!(
                !removable_items(std::slice::from_ref(&application), &registry)
                    .iter()
                    .any(|item| item.target == id_target),
                "{}",
                package.id
            );
        }
        for name in [
            "uv 0.9.0 (x64)",
            "GitHub CLI 2.81.0",
            "Mozilla Firefox ESR 140.0 (x64 fr)",
            ".NET Desktop Runtime 8.0.10 (x64)",
        ] {
            let registry = [name.to_string()];
            let removals = removable_items(&[], &registry);
            assert_eq!(removals.len(), 1, "{name}");
            assert_eq!(removals[0].target, format!("name:{name}"), "{name}");
            assert!(removal_is_allowed(
                &removals[0].target,
                &registry,
                &registry
            ));
            assert!(!removal_is_allowed(&removals[0].target, &registry, &[]));
            assert!(registry_name_matches_target(&removals[0].target, name));
        }
        assert!(!application_matches_removal(
            &app("NodeJS"),
            "name:NodeJS LTS 22.0"
        ));
        assert!(!application_matches_removal(
            &app("TeamSpeak 3"),
            "name:TeamSpeak 6"
        ));
        assert!(!application_matches_removal(
            &app(".NET Desktop Runtime 8"),
            "name:.NET Desktop Runtime 80"
        ));
        assert!(application_matches_removal(
            &app("Syncthing"),
            "name:Syncthing 2.0"
        ));
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
        assert_eq!(results[2].target, "Mozilla.Firefox.ESR");
        assert_eq!(results[2].kind, Kind::Install);
        assert_eq!(results[3].title, "Fire Fox Toolkit");
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
