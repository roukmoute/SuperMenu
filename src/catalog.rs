//! Local, bounded catalogs and optional private launch/clipboard history.
use crate::config::{self, Settings};
use crate::core::{Item, Kind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CACHE_VERSION: u32 = 1;
const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DIRECTORIES: usize = 4096;
const MAX_ENTRIES_EXAMINED: usize = 100_000;
const MAX_FILES: usize = 100_000;
const MAX_DEPTH: usize = 24;
const SYSTEM_ATTRIBUTE: u32 = 0x4;
const REPARSE_ATTRIBUTE: u32 = 0x400;
const MAX_CLIPBOARD_TEXT: usize = 32_768;
const MAX_USAGE_ENTRIES: usize = 10_000;

#[derive(Serialize, Deserialize)]
struct Cache {
    version: u32,
    #[serde(default = "default_cache_language")]
    language: String,
    items: Vec<Item>,
}

fn default_cache_language() -> String {
    "fr".into()
}

impl Cache {
    fn compatible_with(&self, language: &str) -> bool {
        self.version == CACHE_VERSION
            && self.language == language
            && self.items.len() <= MAX_FILES + 20_000
    }
}

/// Fast startup uses this snapshot; the background catalog refresh reconciles it.
/// Clipboard, shell queries, and dynamic results are never written to this cache.
pub fn load_cache() -> Vec<Item> {
    let path = config::data_dir().join("index.json");
    let Ok(metadata) = fs::metadata(&path) else {
        return Vec::new();
    };
    if metadata.len() > MAX_CACHE_BYTES {
        return Vec::new();
    }
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(cache) = serde_json::from_reader::<_, Cache>(BufReader::new(file)) else {
        return Vec::new();
    };
    if !cache.compatible_with(&crate::i18n::current_language()) {
        return Vec::new();
    }
    cache
        .items
        .into_iter()
        .filter(is_cacheable)
        .map(|mut item| {
            item.prepare();
            item
        })
        .collect()
}

/// Tag the snapshot with the language captured when its catalog worker started.
pub fn save_cache(items: &[Item], language: &str) -> io::Result<()> {
    if !crate::i18n::is_supported(language) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsupported index cache language",
        ));
    }
    let cache = Cache {
        version: CACHE_VERSION,
        language: language.to_string(),
        items: items
            .iter()
            .filter(|item| is_cacheable(item))
            .take(MAX_FILES + 20_000)
            .cloned()
            .collect(),
    };
    write_json_atomic(&config::data_dir().join("index.json"), &cache)
}

fn is_cacheable(item: &Item) -> bool {
    match item.kind {
        Kind::Application => !crate::native::is_uninstall_shortcut_title(&item.title),
        Kind::File | Kind::Folder => true,
        _ => false,
    }
}

/// Runs on the catalog worker. Explicit roots only, never drive-wide or network crawls.
/// Time/entry/directory/depth caps make a huge tree yield a useful partial index.
pub fn files(settings: &Settings) -> Vec<Item> {
    let maximum = settings.max_indexed_files.min(MAX_FILES);
    if maximum == 0 {
        return Vec::new();
    }
    let started = Instant::now();
    let mut result = Vec::with_capacity(maximum.min(4096));
    let mut pending = VecDeque::new();
    let mut seen = HashSet::new();
    for value in settings.file_roots.iter().take(16) {
        if let Some(root) = local_root(value) {
            if seen.insert(path_key(&root)) {
                pending.push_back((root, 0_usize));
            }
        }
    }
    let mut directory_count = 0;
    let mut entry_count = 0;
    while let Some((path, depth)) = pending.pop_front() {
        if directory_count >= MAX_DIRECTORIES || started.elapsed() >= Duration::from_secs(5) {
            break;
        }
        directory_count += 1;
        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            entry_count += 1;
            if entry_count > MAX_ENTRIES_EXAMINED
                || result.len() >= maximum
                || started.elapsed() >= Duration::from_secs(5)
            {
                return result;
            }
            let path = entry.path();
            // symlink_metadata avoids following directory junctions and symbolic links.
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_attributes() & (SYSTEM_ATTRIBUTE | REPARSE_ATTRIBUTE) != 0 {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_directory = metadata.is_dir();
            if is_directory && ignored_directory(&name) {
                continue;
            }
            if !seen.insert(path_key(&path)) {
                continue;
            }
            let target = path.to_string_lossy().into_owned();
            result.push(Item::new(
                format!("file:{}", path_key(&path)),
                name,
                &target,
                if is_directory { "▱" } else { "▤" },
                if is_directory {
                    Kind::Folder
                } else {
                    Kind::File
                },
                &target,
            ));
            if is_directory
                && depth < MAX_DEPTH
                && pending.len() + directory_count < MAX_DIRECTORIES
            {
                pending.push_back((path, depth + 1));
            }
        }
    }
    result
}

fn ignored_directory(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git"
            | ".svn"
            | ".hg"
            | "node_modules"
            | "bin"
            | "obj"
            | "$recycle.bin"
            | "system volume information"
    )
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_uppercase()
}

fn expand_environment(value: &str) -> String {
    let mut result = String::new();
    let mut remaining = value;
    while let Some(start) = remaining.find('%') {
        result.push_str(&remaining[..start]);
        let tail = &remaining[start + 1..];
        let Some(end) = tail.find('%') else {
            result.push_str(&remaining[start..]);
            return result;
        };
        let name = &tail[..end];
        match std::env::var(name) {
            Ok(replacement) => result.push_str(&replacement),
            Err(_) => result.push_str(&remaining[start..start + end + 2]),
        }
        remaining = &tail[end + 1..];
    }
    result.push_str(remaining);
    result
}

fn local_root(value: &str) -> Option<PathBuf> {
    if value.trim().is_empty() {
        return None;
    }
    let expanded = expand_environment(value);
    if expanded.starts_with(r"\\") {
        return None;
    }
    let original = Path::new(&expanded);
    let metadata = fs::symlink_metadata(original).ok()?;
    if !metadata.is_dir()
        || metadata.file_attributes() & (SYSTEM_ATTRIBUTE | REPARSE_ATTRIBUTE) != 0
    {
        return None;
    }
    let canonical = fs::canonicalize(original).ok()?;
    let text = canonical.to_string_lossy();
    if text.starts_with(r"\\?\UNC\") {
        return None;
    }
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let text = text.trim_end_matches(['\\', '/']);
    if text.len() <= 2 || text.ends_with(':') {
        return None;
    }
    let root = PathBuf::from(text);
    let wide: Vec<u16> = root.as_os_str().encode_wide().collect();
    if wide.len() < 3 || wide[1] != u16::from(b':') {
        return None;
    }
    let drive = [wide[0], u16::from(b':'), u16::from(b'\\'), 0];
    // SAFETY: drive is a valid, NUL-terminated drive root and remains live for the call.
    if unsafe { native::GetDriveTypeW(drive.as_ptr()) } == 4 {
        return None;
    }
    Some(root)
}

/// Persistent counts contain SHA-256 identifiers, never launch targets or query text.
pub struct Usage {
    pub counts: HashMap<String, u32>,
}

impl Default for Usage {
    fn default() -> Self {
        Self::new()
    }
}

impl Usage {
    pub fn new() -> Self {
        let path = config::data_dir().join("usage.json");
        let saved = fs::metadata(&path)
            .ok()
            .filter(|metadata| metadata.len() <= 2 * 1024 * 1024)
            .and_then(|_| File::open(path).ok())
            .and_then(|file| {
                serde_json::from_reader::<_, HashMap<String, u32>>(BufReader::new(file)).ok()
            })
            .unwrap_or_default();
        let counts = saved
            .into_iter()
            .filter(|(key, value)| {
                key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()) && *value > 0
            })
            .take(MAX_USAGE_ENTRIES)
            .map(|(key, value)| (key.to_ascii_lowercase(), value))
            .collect();
        Self { counts }
    }

    pub fn snapshot_for(&self, items: &[Item]) -> HashMap<String, u32> {
        items
            .iter()
            .filter_map(|item| {
                self.counts
                    .get(&usage_key(&item.id))
                    .map(|count| (item.id.clone(), *count))
            })
            .collect()
    }

    pub fn record(&mut self, id: &str) {
        if id.is_empty() {
            return;
        }
        let key = usage_key(id);
        let current = self.counts.entry(key).or_default();
        *current = current.saturating_add(1);
        if self.counts.len() > MAX_USAGE_ENTRIES {
            if let Some(key) = self
                .counts
                .iter()
                .min_by_key(|(_, count)| **count)
                .map(|(key, _)| key.clone())
            {
                self.counts.remove(&key);
            }
        }
        let _ = write_json_atomic(&config::data_dir().join("usage.json"), &self.counts);
    }

    pub fn reset(&mut self) {
        self.counts.clear();
        let _ = write_json_atomic(&config::data_dir().join("usage.json"), &self.counts);
    }
}

fn usage_key(id: &str) -> String {
    format!("{:x}", Sha256::digest(id.as_bytes()))
}

/// In-memory clipboard history. Caller must check the opt-in setting before capture.
pub struct ClipHistory {
    entries: VecDeque<Item>,
    sequence: u64,
}

impl Default for ClipHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipHistory {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            sequence: 0,
        }
    }

    pub fn capture(&mut self, maximum: usize) -> bool {
        if maximum == 0 {
            self.clear();
            return false;
        }
        let Some(text) = clipboard_text() else {
            return false;
        };
        self.entries.retain(|item| item.target != text);
        self.sequence = self.sequence.wrapping_add(1);
        let mut title: String = text
            .chars()
            .take(96)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        if text.chars().count() > 96 {
            title.push('…')
        }
        self.entries.push_front(Item::new(
            format!("clipboard:{}", self.sequence),
            title,
            crate::i18n::fmt(
                "Presse-papiers · {count} caractères",
                &[("count", text.chars().count().to_string())],
            ),
            "▤",
            Kind::Clipboard,
            text,
        ));
        self.entries.truncate(maximum.min(100));
        true
    }

    pub fn items(&self) -> Vec<Item> {
        self.entries.iter().cloned().collect()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

fn clipboard_text() -> Option<String> {
    // SAFETY: all memory is read while the clipboard is owned, globally locked, and bounded.
    // RAII guards release the clipboard and every locked handle on all return paths.
    unsafe {
        if native::OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let _clipboard = ClipboardGuard;
        let exclude = wide_null("ExcludeClipboardContentFromMonitorProcessing");
        let history = wide_null("CanIncludeInClipboardHistory");
        let exclude_format = native::RegisterClipboardFormatW(exclude.as_ptr());
        let history_format = native::RegisterClipboardFormatW(history.as_ptr());
        if exclude_format == 0
            || history_format == 0
            || native::IsClipboardFormatAvailable(exclude_format) != 0
        {
            return None;
        }
        if native::IsClipboardFormatAvailable(history_format) != 0 {
            let handle = native::GetClipboardData(history_format);
            if handle.is_null() || native::GlobalSize(handle) < 4 {
                return None;
            }
            let policy = GlobalGuard::new(handle)?;
            if std::ptr::read_unaligned(policy.pointer.cast::<u32>()) != 1 {
                return None;
            }
        }
        if native::IsClipboardFormatAvailable(13) == 0 {
            return None;
        }
        let handle = native::GetClipboardData(13);
        if handle.is_null() {
            return None;
        }
        let bytes = native::GlobalSize(handle);
        if !(2..=(MAX_CLIPBOARD_TEXT + 1) * 2).contains(&bytes) {
            return None;
        }
        let locked = GlobalGuard::new(handle)?;
        let buffer = std::slice::from_raw_parts(locked.pointer.cast::<u16>(), bytes / 2);
        let length = buffer.iter().position(|value| *value == 0)?;
        if length == 0 || length > MAX_CLIPBOARD_TEXT {
            return None;
        }
        let text = String::from_utf16_lossy(&buffer[..length]);
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

struct ClipboardGuard;
impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: this guard is only constructed following a successful OpenClipboard call.
        unsafe {
            native::CloseClipboard();
        }
    }
}

struct GlobalGuard {
    handle: *mut c_void,
    pointer: *mut c_void,
}
impl GlobalGuard {
    unsafe fn new(handle: *mut c_void) -> Option<Self> {
        // SAFETY: the caller verified a non-null global memory handle from the open clipboard.
        let pointer = unsafe { native::GlobalLock(handle) };
        if pointer.is_null() {
            None
        } else {
            Some(Self { handle, pointer })
        }
    }
}
impl Drop for GlobalGuard {
    fn drop(&mut self) {
        // SAFETY: the handle was successfully locked when this guard was constructed.
        unsafe {
            native::GlobalUnlock(self.handle);
        }
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing storage directory"))?;
    fs::create_dir_all(parent)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".{}.{}.tmp", std::process::id(), now));
    let result = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, value).map_err(io::Error::other)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[allow(non_snake_case)]
mod native {
    use std::ffi::c_void;
    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn OpenClipboard(window: *mut c_void) -> i32;
        pub fn CloseClipboard() -> i32;
        pub fn RegisterClipboardFormatW(name: *const u16) -> u32;
        pub fn IsClipboardFormatAvailable(format: u32) -> i32;
        pub fn GetClipboardData(format: u32) -> *mut c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GlobalSize(handle: *mut c_void) -> usize;
        pub fn GlobalLock(handle: *mut c_void) -> *mut c_void;
        pub fn GlobalUnlock(handle: *mut c_void) -> i32;
        pub fn GetDriveTypeW(root: *const u16) -> u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_scan_is_bounded_and_skips_generated_directories() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temporary_root = std::env::temp_dir();
        let root = temporary_root.join(format!(
            "SuperMenu-catalog-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("keep")).unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("keep/document.txt"), "visible").unwrap();
        fs::write(root.join("node_modules/ignored.txt"), "ignored").unwrap();
        fs::write(root.join(".git/config"), "ignored").unwrap();
        let settings = Settings {
            file_roots: vec![root.to_string_lossy().into_owned()],
            max_indexed_files: 100,
            ..Settings::default()
        };
        let items = files(&settings);
        let mut bounded = settings.clone();
        bounded.max_indexed_files = 1;
        let small = files(&bounded);
        // Resolve and verify the disposable fixture before recursively removing it.
        let verified_root = fs::canonicalize(&root).unwrap();
        let verified_temporary = fs::canonicalize(&temporary_root).unwrap();
        assert!(verified_root.starts_with(verified_temporary));
        fs::remove_dir_all(&verified_root).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|item| item.title == "document.txt"));
        assert!(!items
            .iter()
            .any(|item| item.target.contains("node_modules") || item.target.contains(".git")));
        assert_eq!(small.len(), 1);
    }

    #[test]
    fn whole_drive_root_is_rejected() {
        let drive = std::env::current_dir().unwrap();
        let mut components = drive.components();
        let root: PathBuf = components.by_ref().take(2).collect();
        assert!(local_root(&root.to_string_lossy()).is_none());
    }

    #[test]
    fn cache_never_includes_clipboard_or_commands() {
        let private = Item::new("secret", "secret", "", "", Kind::Clipboard, "secret text");
        let command = Item::new("command", "command", "", "", Kind::Command, "cmd.exe");
        assert!(!is_cacheable(&private));
        assert!(!is_cacheable(&command));
    }

    #[test]
    fn old_cached_uninstall_shortcuts_are_not_launchable_applications() {
        let entries = [
            ("Steam", Kind::Application),
            ("Désinstaller Telegram", Kind::Application),
            ("Telegram Uninstall", Kind::Application),
            ("Bulk Crap Uninstaller", Kind::Application),
            ("Uninstall notes.txt", Kind::File),
        ];
        let cache = Cache {
            version: CACHE_VERSION,
            language: "fr".into(),
            items: entries
                .into_iter()
                .map(|(title, kind)| Item::new(title, title, "", "", kind, title))
                .collect(),
        };
        let restored: Cache =
            serde_json::from_str(&serde_json::to_string(&cache).unwrap()).unwrap();
        let titles: Vec<_> = restored
            .items
            .iter()
            .filter(|item| is_cacheable(item))
            .map(|item| item.title.as_str())
            .collect();
        assert_eq!(
            titles,
            ["Steam", "Bulk Crap Uninstaller", "Uninstall notes.txt"]
        );
    }

    #[test]
    fn usage_identifier_does_not_store_path_text() {
        assert_eq!(
            usage_key("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let item = Item::new(
            "C:\\private\\app.exe",
            "app",
            "",
            "",
            Kind::Application,
            "C:\\private\\app.exe",
        );
        let usage = Usage {
            counts: [(usage_key(&item.id), 7)].into_iter().collect(),
        };
        assert_eq!(
            usage
                .snapshot_for(std::slice::from_ref(&item))
                .get(&item.id),
            Some(&7)
        );
        assert!(!usage.counts.keys().any(|key| key.contains("private")));
    }

    #[test]
    fn old_index_cache_is_french_and_cannot_seed_an_english_session() {
        let cache: Cache = serde_json::from_str(r#"{"version":1,"items":[]}"#).unwrap();
        assert_eq!(cache.language, "fr");
        assert!(cache.compatible_with("fr"));
        assert!(!cache.compatible_with("en"));
    }

    #[test]
    fn index_cache_only_accepts_matching_language_and_schema() {
        let mut cache = Cache {
            version: CACHE_VERSION,
            language: "en".into(),
            items: Vec::new(),
        };
        assert!(cache.compatible_with("en"));
        assert!(!cache.compatible_with("fr"));
        cache.version += 1;
        assert!(!cache.compatible_with("en"));
        cache.version = CACHE_VERSION;
        cache.language = "unknown".into();
        assert!(!cache.compatible_with("en"));
        let restored: Cache = serde_json::from_slice(&serde_json::to_vec(&cache).unwrap()).unwrap();
        assert_eq!(restored.language, "unknown");
    }
}
