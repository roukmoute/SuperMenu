use serde::{Deserialize, Serialize};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static SAVE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hotkey: String,
    pub theme: String,
    pub accent_color: String,
    pub max_results: usize,
    pub window_width: i32,
    pub clipboard_enabled: bool,
    pub max_clipboard_entries: usize,
    pub max_indexed_files: usize,
    pub file_roots: Vec<String>,
    pub commands: Vec<CustomCommand>,
    pub web_search_template: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomCommand {
    pub name: String,
    pub file_name: String,
    pub arguments: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: "Win+Space".into(),
            theme: "dark".into(),
            accent_color: "#9A8CFF".into(),
            max_results: 9,
            window_width: 720,
            clipboard_enabled: false,
            max_clipboard_entries: 30,
            max_indexed_files: 20_000,
            file_roots: default_file_roots(),
            commands: Vec::new(),
            web_search_template: "https://www.google.com/search?q={query}".into(),
        }
    }
}

impl Settings {
    pub fn normalize(&mut self) {
        self.hotkey = normalize_hotkey(&self.hotkey);
        self.theme = if self.theme.trim().eq_ignore_ascii_case("light") {
            "light"
        } else {
            "dark"
        }
        .into();
        let color = self.accent_color.trim();
        self.accent_color = if matches!(color.len(), 7 | 9)
            && color.starts_with('#')
            && color[1..].bytes().all(|c| c.is_ascii_hexdigit())
        {
            color.to_ascii_uppercase()
        } else {
            "#9A8CFF".into()
        };
        self.max_results = self.max_results.clamp(3, 30);
        self.window_width = self.window_width.clamp(420, 1200);
        self.max_clipboard_entries = self.max_clipboard_entries.clamp(1, 100);
        self.max_indexed_files = self.max_indexed_files.clamp(100, 100_000);
        let mut roots = Vec::new();
        for root in self
            .file_roots
            .iter()
            .map(|root| root.trim())
            .filter(|root| !root.is_empty())
        {
            if roots.len() >= 16 {
                break;
            }
            let root = expand_environment(root);
            if Path::new(&root).is_absolute()
                && !root.contains('\0')
                && !roots
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&root))
            {
                roots.push(root);
            }
        }
        self.file_roots = roots;
        self.commands.retain(|command| {
            !command.name.trim().is_empty() && !command.file_name.trim().is_empty()
        });
        self.commands.truncate(100);
        for command in &mut self.commands {
            command.name = command.name.trim().to_string();
            command.file_name = command.file_name.trim().to_string();
        }
        let template = self.web_search_template.trim();
        if !(template.starts_with("https://") || template.starts_with("http://"))
            || !template.contains("{query}")
            || template
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
            || template
                .split_once("://")
                .is_none_or(|(_, host)| host.starts_with('/') || host.len() < 2)
        {
            self.web_search_template = "https://www.google.com/search?q={query}".into();
        } else {
            self.web_search_template = template.to_string();
        }
    }
}

pub fn data_dir() -> PathBuf {
    if let Some(directory) = env::var_os("LOCALAPPDATA").filter(|directory| !directory.is_empty()) {
        PathBuf::from(directory).join("SuperMenu")
    } else if let Some(home) = env::var_os("USERPROFILE").filter(|directory| !directory.is_empty())
    {
        PathBuf::from(home)
            .join("AppData")
            .join("Local")
            .join("SuperMenu")
    } else {
        env::temp_dir().join("SuperMenu")
    }
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}
pub fn load() -> Settings {
    load_from(&settings_path())
}
pub fn save(settings: &Settings) -> io::Result<()> {
    save_to(settings, &settings_path())
}

pub fn load_from(path: &Path) -> Settings {
    if fs::metadata(path).map_or(true, |metadata| metadata.len() > 1_048_576) {
        return Settings::default();
    }
    let mut settings: Settings = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    settings.normalize();
    settings
}

/// Write beside the destination, flush, then replace it in one filesystem operation.
pub fn save_to(settings: &Settings, path: &Path) -> io::Result<()> {
    let _guard = SAVE_LOCK
        .lock()
        .map_err(|_| io::Error::other("settings lock poisoned"))?;
    let directory = path
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(directory)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = directory.join(format!(".settings.{}.{nonce}.tmp", std::process::id()));
    let mut normalized = settings.clone();
    normalized.normalize();
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let bytes = serde_json::to_vec_pretty(&normalized).map_err(io::Error::other)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        replace_file(&temporary, path)?;
        Ok(())
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(source: *const u16, target: *const u16, flags: u32) -> i32;
    }
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH; source and target share a directory.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0x1 | 0x8) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

fn default_file_roots() -> Vec<String> {
    let Some(home) = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .filter(|home| !home.is_empty())
    else {
        return Vec::new();
    };
    let home = PathBuf::from(home);
    ["Desktop", "Documents", "Downloads"]
        .into_iter()
        .map(|folder| home.join(folder).to_string_lossy().into_owned())
        .collect()
}

fn expand_environment(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    let mut remaining = path;
    while let Some(start) = remaining.find('%') {
        result.push_str(&remaining[..start]);
        let after = &remaining[start + 1..];
        let Some(end) = after.find('%') else {
            result.push_str(&remaining[start..]);
            return result;
        };
        match env::var(&after[..end]) {
            Ok(value) => result.push_str(&value),
            Err(_) => result.push_str(&remaining[start..start + end + 2]),
        }
        remaining = &after[end + 1..];
    }
    result.push_str(remaining);
    result
}

fn normalize_hotkey(hotkey: &str) -> String {
    let parts: Vec<_> = hotkey
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if !(2..=5).contains(&parts.len()) {
        return "Win+Space".into();
    }
    let mut modifiers = Vec::new();
    for part in &parts[..parts.len() - 1] {
        let modifier = match part.to_ascii_lowercase().as_str() {
            "win" | "windows" => "Win",
            "ctrl" | "control" => "Ctrl",
            "alt" => "Alt",
            "shift" => "Shift",
            _ => return "Win+Space".into(),
        };
        if modifiers.contains(&modifier) {
            return "Win+Space".into();
        }
        modifiers.push(modifier);
    }
    let raw = parts[parts.len() - 1].to_ascii_lowercase();
    let key = match raw.as_str() {
        "space" => "Space".to_string(),
        "enter" => "Enter".to_string(),
        "tab" => "Tab".to_string(),
        "escape" => "Escape".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "insert" => "Insert".to_string(),
        "delete" => "Delete".to_string(),
        "up" => "Up".to_string(),
        "down" => "Down".to_string(),
        "left" => "Left".to_string(),
        "right" => "Right".to_string(),
        "pageup" => "PageUp".to_string(),
        "pagedown" => "PageDown".to_string(),
        _ if raw.len() == 1 && raw.as_bytes()[0].is_ascii_alphanumeric() => {
            raw.to_ascii_uppercase()
        }
        _ if raw
            .strip_prefix('f')
            .and_then(|number| number.parse::<u8>().ok())
            .is_some_and(|number| (1..=24).contains(&number)) =>
        {
            raw.to_ascii_uppercase()
        }
        _ => return "Win+Space".into(),
    };
    let mut result: Vec<String> = ["Ctrl", "Alt", "Shift", "Win"]
        .iter()
        .filter(|modifier| modifiers.contains(*modifier))
        .map(|modifier| modifier.to_string())
        .collect();
    result.push(key);
    result.join("+")
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temporary(PathBuf);
    impl Temporary {
        fn new() -> Self {
            let directory = env::temp_dir().join(format!(
                "SuperMenu.Tests.{}.{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn missing_and_malformed_settings_recover() {
        let temporary = Temporary::new();
        let path = temporary.0.join("settings.json");
        assert_eq!(load_from(&path).hotkey, "Win+Space");
        for malformed in [
            "{broken",
            "null",
            "[]",
            "{\"max_results\":\"oops\"}",
            "{\"commands\":null}",
        ] {
            fs::write(&path, malformed).unwrap();
            assert_eq!(load_from(&path).max_results, 9);
        }
        fs::write(&path, "{\"max_results\":12}").unwrap();
        assert_eq!(load_from(&path).max_results, 12);
        fs::write(&path, vec![b' '; 1_048_577]).unwrap();
        assert_eq!(load_from(&path).max_results, 9);
    }
    #[test]
    fn settings_validation_and_atomic_roundtrip() {
        let temporary = Temporary::new();
        let path = temporary.0.join("nested").join("settings.json");
        let root = temporary.0.to_string_lossy().into_owned();
        let mut settings = Settings {
            hotkey: "control + alt + k".into(),
            theme: "LIGHT".into(),
            accent_color: "#ab12CD".into(),
            max_results: usize::MAX,
            window_width: -4,
            max_clipboard_entries: 0,
            max_indexed_files: usize::MAX,
            clipboard_enabled: true,
            web_search_template: "javascript:{query}".into(),
            file_roots: vec![
                root.clone(),
                format!(" {root} "),
                "relative/path".into(),
                "".into(),
            ],
            commands: vec![
                CustomCommand {
                    name: " Terminal ".into(),
                    file_name: " wt.exe ".into(),
                    arguments: "-d .".into(),
                },
                CustomCommand {
                    name: "invalid".into(),
                    ..CustomCommand::default()
                },
            ],
        };
        save_to(&settings, &path).unwrap();
        let loaded = load_from(&path);
        assert_eq!(
            (
                loaded.hotkey.as_str(),
                loaded.theme.as_str(),
                loaded.accent_color.as_str()
            ),
            ("Ctrl+Alt+K", "light", "#AB12CD")
        );
        assert_eq!(
            (
                loaded.max_results,
                loaded.window_width,
                loaded.max_clipboard_entries,
                loaded.max_indexed_files
            ),
            (30, 420, 1, 100_000)
        );
        assert_eq!(loaded.file_roots, vec![root]);
        assert_eq!(loaded.commands.len(), 1);
        assert_eq!(loaded.commands[0].name, "Terminal");
        assert!(loaded.clipboard_enabled);
        assert!(loaded.web_search_template.starts_with("https://"));
        settings.hotkey = "Win+Space".into();
        settings.max_results = 11;
        settings.window_width = 800;
        settings.web_search_template = "https://duckduckgo.com/?q={query}".into();
        save_to(&settings, &path).unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.max_results, 11);
        assert_eq!(loaded.window_width, 800);
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["max_results"], 11);
        assert_eq!(
            fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1,
            "No temp files should remain."
        );
    }
    #[test]
    fn settings_hotkeys_roots_and_url_validation() {
        for (input, expected) in [
            ("Win+Space", "Win+Space"),
            ("shift+windows+k", "Shift+Win+K"),
            ("Alt+F12", "Alt+F12"),
            ("Win+Win+A", "Win+Space"),
            ("unknown", "Win+Space"),
            ("Win+F25", "Win+Space"),
        ] {
            assert_eq!(normalize_hotkey(input), expected);
        }
        let mut settings = Settings {
            max_results: 0,
            window_width: i32::MAX,
            accent_color: "#Z00000".into(),
            theme: "unknown".into(),
            ..Settings::default()
        };
        settings.normalize();
        assert_eq!(settings.max_results, 3);
        assert_eq!(settings.window_width, 1200);
        assert_eq!(settings.accent_color, "#9A8CFF");
        assert_eq!(settings.theme, "dark");
        for template in [
            "file:///tmp/{query}",
            "https:///{query}",
            "https://example.com/no-marker",
            "https://example.com/?q={query}\n",
        ] {
            settings.web_search_template = template.into();
            settings.normalize();
            if !template.ends_with('\n') {
                assert_eq!(
                    settings.web_search_template,
                    "https://www.google.com/search?q={query}"
                );
            }
        }
        if let Some(variable) = ["USERPROFILE", "HOME"]
            .iter()
            .find(|variable| env::var(variable).is_ok())
        {
            settings.file_roots = vec![format!("%{variable}%")];
            settings.normalize();
            assert_eq!(settings.file_roots, vec![env::var(variable).unwrap()]);
        }
    }
}
