//! One optional JSON language pack, loaded once and looked up entirely in memory.
//! French source text is the built-in fallback. No HTTP client or async runtime is bundled.

use serde::Deserialize;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_PACK_BYTES: usize = 1_048_576;
const MAX_MESSAGES: usize = 4096;
const MAX_TEXT_BYTES: usize = 16_384;
const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug)]
pub struct Language {
    pub code: &'static str,
    pub name: &'static str,
}

/// Only complete translations are offered. Adding a language does not bundle its pack.
pub const LANGUAGES: &[Language] = &[
    Language {
        code: "fr",
        name: "Français",
    },
    Language {
        code: "en",
        name: "English",
    },
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LanguagePack {
    schema_version: u32,
    language: String,
    #[serde(deserialize_with = "unique_messages")]
    messages: HashMap<String, String>,
}

impl LanguagePack {
    fn french() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            language: "fr".into(),
            messages: HashMap::new(),
        }
    }

    fn text<'a>(&'a self, source: &'a str) -> &'a str {
        self.messages.get(source).map_or(source, String::as_str)
    }

    fn translate(&self, source: &str) -> String {
        self.text(source).to_string()
    }
}

static ACTIVE_PACK: OnceLock<RwLock<LanguagePack>> = OnceLock::new();
static DOWNLOAD_LOCK: Mutex<()> = Mutex::new(());
static REVISION: AtomicUsize = AtomicUsize::new(0);

pub fn revision() -> usize {
    REVISION.load(Ordering::Acquire)
}

fn active_pack() -> &'static RwLock<LanguagePack> {
    ACTIVE_PACK.get_or_init(|| RwLock::new(LanguagePack::french()))
}

pub fn is_supported(language: &str) -> bool {
    LANGUAGES.iter().any(|entry| entry.code == language)
}

pub fn current_language() -> String {
    active_pack()
        .read()
        .map(|pack| pack.language.clone())
        .unwrap_or_else(|_| "fr".into())
}

/// Missing keys and an unavailable/poisoned pack always fall back to French.
pub fn tr(source: &str) -> String {
    active_pack()
        .read()
        .map(|pack| pack.translate(source))
        .unwrap_or_else(|_| source.to_string())
}

/// Replace named placeholders once; a replacement is literal text, never another template.
pub fn fmt(source: &str, values: &[(&str, String)]) -> String {
    // Keep the pack borrowed during formatting: cloning the template would allocate
    // a second String for every translated status message and result subtitle.
    active_pack()
        .read()
        .map(|pack| substitute(pack.text(source), values))
        .unwrap_or_else(|_| substitute(source, values))
}

fn substitute(template: &str, values: &[(&str, String)]) -> String {
    let mut result = String::with_capacity(template.len());
    let mut cursor = 0;
    for (start, end, name) in tokens(template) {
        result.push_str(&template[cursor..start]);
        match values.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => result.push_str(value),
            None => result.push_str(&template[start..end]),
        }
        cursor = end;
    }
    result.push_str(&template[cursor..]);
    result
}

/// Load the selected cached pack without performing any network operation.
/// An empty language means the built-in fallback until the first-run choice is made.
/// A failed load keeps the previously active pack intact.
pub fn init_cached(language: &str, cache_root: &Path) -> Result<(), String> {
    load_cached(language, cache_root)
}

pub fn load_cached(language: &str, cache_root: &Path) -> Result<(), String> {
    let pack = read_cached(language, cache_root)?;
    activate(pack)
}

/// A valid cached file wins immediately. Download only when the chosen pack is absent/invalid.
/// Both the cache and active language are left intact if download or validation fails.
#[allow(dead_code)] // Convenience API; the cancellable chooser uses ensure_cached + load_cached.
pub fn download_and_load(language: &str, cache_root: &Path) -> Result<(), String> {
    cache_path(language, cache_root)?;
    if language == "fr" {
        return activate(LanguagePack::french());
    }
    let _guard = DOWNLOAD_LOCK
        .lock()
        .map_err(|_| "Le chargement de la traduction est indisponible.".to_string())?;
    let pack = fetch_pack(language, cache_root, download)?;
    activate(pack)
}

/// Prepare an offline pack without changing the active UI language.
/// This lets a cancellable chooser download on a worker before committing settings.
pub fn ensure_cached(language: &str, cache_root: &Path) -> Result<(), String> {
    cache_path(language, cache_root)?;
    if language == "fr" {
        return Ok(());
    }
    let _guard = DOWNLOAD_LOCK
        .lock()
        .map_err(|_| "Le chargement de la traduction est indisponible.".to_string())?;
    fetch_pack(language, cache_root, download).map(|_| ())
}

fn activate(pack: LanguagePack) -> Result<(), String> {
    let mut active = active_pack()
        .write()
        .map_err(|_| "Le chargement de la traduction est indisponible.".to_string())?;
    *active = pack;
    REVISION.fetch_add(1, Ordering::Release);
    Ok(())
}

fn cache_path(language: &str, cache_root: &Path) -> Result<PathBuf, String> {
    if !is_supported(language) {
        return Err("Cette langue n’est pas disponible.".into());
    }
    Ok(cache_root.join(format!("{language}.json")))
}

fn read_cached(language: &str, cache_root: &Path) -> Result<LanguagePack, String> {
    if language.is_empty() || language == "fr" {
        return Ok(LanguagePack::french());
    }
    let path = cache_path(language, cache_root)?;
    let file = fs::File::open(path)
        .map_err(|_| "La traduction n’est pas encore disponible hors ligne.".to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_PACK_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Impossible de lire la traduction enregistrée.".to_string())?;
    parse_pack(&bytes, language)
}

fn fetch_pack<F>(language: &str, cache_root: &Path, downloader: F) -> Result<LanguagePack, String>
where
    F: FnOnce(&str) -> Result<Vec<u8>, String>,
{
    // Validate before creating files, making a request, or accepting a cache path.
    let path = cache_path(language, cache_root)?;
    if let Ok(pack) = read_cached(language, cache_root) {
        return Ok(pack);
    }
    let bytes = downloader(language)?;
    let pack = parse_pack(&bytes, language)?;
    save_pack(&bytes, &path).map_err(|_| {
        "Impossible d’enregistrer la traduction pour une utilisation hors ligne.".to_string()
    })?;
    Ok(pack)
}

fn unique_messages<'de, D>(deserializer: D) -> Result<HashMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct MessagesVisitor;
    impl<'de> serde::de::Visitor<'de> for MessagesVisitor {
        type Value = HashMap<String, String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an object containing unique translation keys")
        }

        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: serde::de::MapAccess<'de>,
        {
            let mut messages = HashMap::new();
            while let Some((source, translation)) = map.next_entry::<String, String>()? {
                if messages.len() >= MAX_MESSAGES {
                    return Err(serde::de::Error::custom("too many translation keys"));
                }
                if messages.insert(source, translation).is_some() {
                    return Err(serde::de::Error::custom("duplicate translation key"));
                }
            }
            Ok(messages)
        }
    }
    deserializer.deserialize_map(MessagesVisitor)
}

fn parse_pack(bytes: &[u8], language: &str) -> Result<LanguagePack, String> {
    if !is_supported(language) || language == "fr" {
        return Err("Cette langue n’utilise pas de fichier de traduction.".into());
    }
    if bytes.is_empty() || bytes.len() > MAX_PACK_BYTES {
        return Err("Le fichier de traduction est vide ou trop volumineux.".into());
    }
    let pack: LanguagePack = serde_json::from_slice(bytes)
        .map_err(|_| "Le fichier de traduction est invalide.".to_string())?;
    if pack.schema_version != SCHEMA_VERSION || pack.language != language {
        return Err("Le fichier de traduction est incompatible avec la langue demandée.".into());
    }
    if pack.messages.is_empty() {
        return Err("Le fichier de traduction ne contient aucun texte.".into());
    }
    for (source, translation) in &pack.messages {
        let valid = |text: &str| {
            !text.is_empty()
                && text.len() <= MAX_TEXT_BYTES
                && !text.chars().any(|character| {
                    character.is_control() && !matches!(character, '\n' | '\r' | '\t')
                })
        };
        if !valid(source)
            || !valid(translation)
            || placeholders(source) != placeholders(translation)
        {
            return Err("Un texte ou un paramètre du fichier de traduction est invalide.".into());
        }
    }
    Ok(pack)
}

fn tokens(template: &str) -> impl Iterator<Item = (usize, usize, &str)> {
    let mut cursor = 0;
    std::iter::from_fn(move || {
        while let Some(open) = template[cursor..].find('{') {
            let start = cursor + open;
            let Some(close) = template[start + 1..].find('}') else {
                cursor = template.len();
                return None;
            };
            let end = start + 1 + close;
            let name = &template[start + 1..end];
            cursor = end + 1;
            let mut bytes = name.bytes();
            if bytes
                .next()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
                && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Some((start, end + 1, name));
            }
        }
        None
    })
}

fn placeholders(template: &str) -> Vec<&str> {
    // Sorted names retain duplicate occurrences, so equality checks both names
    // and counts without allocating a tree node for each distinct placeholder.
    let mut placeholders: Vec<_> = tokens(template).map(|(_, _, name)| name).collect();
    placeholders.sort_unstable();
    placeholders
}

fn save_pack(bytes: &[u8], destination: &Path) -> io::Result<()> {
    let directory = destination
        .parent()
        .ok_or_else(|| io::Error::other("language cache has no parent"))?;
    fs::create_dir_all(directory)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = directory.join(format!(".language.{}.{nonce}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temporary, destination)
    })();
    let _ = fs::remove_file(temporary);
    result
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn download(language: &str) -> Result<Vec<u8>, String> {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Networking::WinHttp::*;

    struct InternetHandle(*mut c_void);
    impl InternetHandle {
        fn new(pointer: *mut c_void) -> Result<Self, String> {
            if pointer.is_null() {
                Err(network_error())
            } else {
                Ok(Self(pointer))
            }
        }
    }
    impl Drop for InternetHandle {
        fn drop(&mut self) {
            unsafe {
                WinHttpCloseHandle(self.0);
            }
        }
    }
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn check(result: i32) -> Result<(), String> {
        if result == 0 {
            Err(network_error())
        } else {
            Ok(())
        }
    }
    fn network_error() -> String {
        format!(
            "Impossible de télécharger la traduction : {}",
            io::Error::last_os_error()
        )
    }

    if !is_supported(language) || language == "fr" {
        return Err("Cette langue n’utilise pas de fichier de traduction.".into());
    }
    let started = Instant::now();
    let user_agent = wide("SuperMenu language pack");
    let host = wide("raw.githubusercontent.com");
    let path = wide(&format!(
        "/roukmoute/SuperMenu/main/locales/{language}.json"
    ));
    let verb = wide("GET");
    unsafe {
        let session = InternetHandle::new(WinHttpOpen(
            user_agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        ))?;
        check(WinHttpSetTimeouts(session.0, 3000, 5000, 5000, 5000))?;
        let protocols = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2;
        check(WinHttpSetOption(
            session.0,
            WINHTTP_OPTION_SECURE_PROTOCOLS,
            (&protocols as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        ))?;
        let connection = InternetHandle::new(WinHttpConnect(session.0, host.as_ptr(), 443, 0))?;
        let request = InternetHandle::new(WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        // Keep the download on the fixed HTTPS host; an unexpected redirect is an error.
        let redirects = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
        check(WinHttpSetOption(
            request.0,
            WINHTTP_OPTION_REDIRECT_POLICY,
            (&redirects as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        ))?;
        check(WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0))?;
        check(WinHttpReceiveResponse(request.0, null_mut()))?;
        let mut status = 0_u32;
        let mut status_bytes = std::mem::size_of::<u32>() as u32;
        check(WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut status_bytes,
            null_mut(),
        ))?;
        if status != 200 {
            return Err(format!(
                "Le téléchargement de la traduction a échoué (HTTP {status})."
            ));
        }
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            if started.elapsed() > Duration::from_secs(20) {
                return Err(
                    "Le téléchargement de la traduction a dépassé le délai autorisé.".into(),
                );
            }
            let mut received = 0_u32;
            check(WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut received,
            ))?;
            if received == 0 {
                return Ok(bytes);
            }
            if bytes.len() + received as usize > MAX_PACK_BYTES {
                return Err("Le fichier de traduction est trop volumineux.".into());
            }
            bytes.extend_from_slice(&buffer[..received as usize]);
        }
    }
}

#[cfg(not(windows))]
fn download(_language: &str) -> Result<Vec<u8>, String> {
    Err("Le téléchargement de traductions nécessite Windows.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temporary(PathBuf);
    impl Temporary {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "SuperMenu.Language.Tests.{}.{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn english() -> Vec<u8> {
        br#"{"schema_version":1,"language":"en","messages":{"Applications":"Applications","Lancement de {name}":"Launching {name}"}}"#.to_vec()
    }

    #[test]
    fn validated_pack_translates_and_missing_keys_fall_back_to_source() {
        let pack = parse_pack(&english(), "en").unwrap();
        assert_eq!(pack.translate("Lancement de {name}"), "Launching {name}");
        assert_eq!(
            pack.translate("Texte ajouté plus tard"),
            "Texte ajouté plus tard"
        );
        assert_eq!(LanguagePack::french().translate("À propos"), "À propos");
    }

    #[test]
    fn shipped_english_pack_has_valid_ui_about_and_named_parameters() {
        // Test-only inclusion: the downloadable pack is never bundled in the release binary.
        let pack = parse_pack(include_bytes!("../locales/en.json"), "en").unwrap();
        assert_eq!(pack.translate("EXPLORER"), "EXPLORE");
        assert_eq!(pack.translate("À propos"), "About");
        assert_eq!(pack.translate("Matériel"), "Hardware");
        assert_eq!(pack.translate("Mémoire"), "Memory");
        assert_eq!(
            substitute(
                &pack.translate("Presse-papiers · {count} caractères"),
                &[("count", "12".into())]
            ),
            "Clipboard · 12 characters"
        );
        let query_help = "L'adresse de recherche doit commencer par https:// et contenir {query}.";
        assert_eq!(
            placeholders(&pack.translate(query_help)),
            placeholders(query_help)
        );
    }

    #[test]
    fn pack_rejects_wrong_schema_language_fields_and_duplicate_keys() {
        for bytes in [
            br#"{"schema_version":2,"language":"en","messages":{"A":"B"}}"#.as_slice(),
            br#"{"schema_version":1,"language":"fr","messages":{"A":"B"}}"#.as_slice(),
            br#"{"schema_version":1,"language":"en","messages":{}}"#.as_slice(),
            br#"{"schema_version":1,"language":"en","messages":{"A":"B"},"code":"run"}"#.as_slice(),
            br#"{"schema_version":1,"language":"en","messages":{"A":"B","A":"C"}}"#.as_slice(),
            br#"{"schema_version":1,"language":"en","messages":{"A":3}}"#.as_slice(),
            b"not JSON".as_slice(),
        ] {
            assert!(parse_pack(bytes, "en").is_err(), "{:?}", bytes);
        }
    }

    #[test]
    fn pack_size_count_text_and_control_characters_are_bounded() {
        assert!(parse_pack(&vec![b' '; MAX_PACK_BYTES + 1], "en").is_err());
        assert!(parse_pack(b"", "en").is_err());
        for translation in [
            String::new(),
            "X".repeat(MAX_TEXT_BYTES + 1),
            "nul\0".into(),
        ] {
            let value = serde_json::json!({
                "schema_version": 1, "language": "en", "messages": {"A": translation}
            });
            assert!(parse_pack(&serde_json::to_vec(&value).unwrap(), "en").is_err());
        }
        let messages: HashMap<_, _> = (0..=MAX_MESSAGES)
            .map(|index| (format!("A{index}"), "B"))
            .collect();
        let value = serde_json::json!({
            "schema_version": 1, "language": "en", "messages": messages
        });
        assert!(parse_pack(&serde_json::to_vec(&value).unwrap(), "en").is_err());
    }

    #[test]
    fn placeholders_must_keep_names_and_occurrence_counts() {
        for translation in ["Launching", "Launching {other}", "Launching {name} {name}"] {
            let value = serde_json::json!({
                "schema_version": 1, "language": "en",
                "messages": {"Lancement de {name}": translation}
            });
            assert!(parse_pack(&serde_json::to_vec(&value).unwrap(), "en").is_err());
        }
        assert_eq!(placeholders("{a}: {b} {a}"), placeholders("{b}: {a}/{a}"));
        assert_eq!(
            placeholders("Literal { } or JSON {\"a\":1}"),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn formatting_is_literal_unicode_safe_and_preserves_unknown_placeholders() {
        assert_eq!(
            substitute(
                "Été {name}: {count}, {unknown}",
                &[("name", "应用 {count}".into()), ("count", "4".into())]
            ),
            "Été 应用 {count}: 4, {unknown}"
        );
        assert_eq!(
            substitute("{query} / {query}", &[("query", "$()".into())]),
            "$() / $()"
        );
        assert_eq!(substitute("{{ braces }", &[]), "{{ braces }");
    }

    #[test]
    fn french_needs_no_cache_directory_or_download() {
        let temporary = Temporary::new();
        let absent = temporary.0.join("absent");
        let pack = fetch_pack("fr", &absent, |_| panic!("French must never download")).unwrap();
        assert_eq!(pack.language, "fr");
        assert!(!absent.exists());
        assert_eq!(read_cached("", &absent).unwrap().language, "fr");
    }

    #[test]
    fn cached_selection_does_not_use_network() {
        let temporary = Temporary::new();
        fs::write(temporary.0.join("en.json"), english()).unwrap();
        let pack = fetch_pack("en", &temporary.0, |_| {
            panic!("A cache hit must not download")
        })
        .unwrap();
        assert_eq!(pack.language, "en");
        assert_eq!(fs::read_dir(&temporary.0).unwrap().count(), 1);
    }

    #[test]
    fn first_download_is_saved_atomically_then_available_offline() {
        let temporary = Temporary::new();
        let directory = temporary.0.join("languages");
        let bytes = english();
        let pack = fetch_pack("en", &directory, |language| {
            assert_eq!(language, "en");
            Ok(bytes.clone())
        })
        .unwrap();
        assert_eq!(pack.language, "en");
        assert_eq!(fs::read(directory.join("en.json")).unwrap(), bytes);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        assert_eq!(read_cached("en", &directory).unwrap().language, "en");
    }

    #[test]
    fn failed_or_invalid_download_leaves_existing_cache_unchanged() {
        let temporary = Temporary::new();
        let path = temporary.0.join("en.json");
        fs::write(&path, b"old invalid cache").unwrap();
        assert!(fetch_pack("en", &temporary.0, |_| Err("offline".into())).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old invalid cache");
        assert!(fetch_pack("en", &temporary.0, |_| Ok(b"invalid response".to_vec())).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old invalid cache");
        assert_eq!(fs::read_dir(&temporary.0).unwrap().count(), 1);
    }

    #[test]
    fn cache_write_failure_removes_temporary_file_and_preserves_destination() {
        let temporary = Temporary::new();
        let destination = temporary.0.join("en.json");
        fs::create_dir(&destination).unwrap();
        assert!(save_pack(&english(), &destination).is_err());
        assert!(destination.is_dir());
        assert_eq!(fs::read_dir(&temporary.0).unwrap().count(), 1);
    }

    #[test]
    fn unsupported_codes_cannot_create_paths_or_reach_downloader() {
        let temporary = Temporary::new();
        for code in ["", "../en", "en.json", "EN", "de", "en\0"] {
            assert!(fetch_pack(code, &temporary.0, |_| panic!(
                "Invalid code reached network"
            ))
            .is_err());
        }
        assert_eq!(fs::read_dir(&temporary.0).unwrap().count(), 0);
    }
}
