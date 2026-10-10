//! Read-only, bounded inventory for post-uninstall ownership checks.
//! An incomplete snapshot must never be used as proof that software is absent.
use serde::{Deserialize, Serialize};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS, FILETIME,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryInfoKeyW, RegQueryValueExW, HKEY,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_DWORD,
    REG_EXPAND_SZ, REG_OPTION_OPEN_LINK, REG_SZ,
};

const UNINSTALL_ROOT: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const MAX_ENTRIES: usize = 20_000;
const MAX_VALUE_BYTES: usize = 65_536;
const MAX_NOTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum Hive {
    User,
    Machine,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum RegistryView {
    Registry32,
    Registry64,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct RegistryKey {
    pub hive: Hive,
    pub view: RegistryView,
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstalledEntry {
    pub name: String,
    pub publisher: String,
    pub install_location: Option<PathBuf>,
    pub display_icon: Option<PathBuf>,
    pub uninstall_key: RegistryKey,
    pub protected: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Inventory {
    pub entries: Vec<InstalledEntry>,
    pub complete: bool,
    pub notes: Vec<String>,
}

impl Inventory {
    fn note(&mut self, note: String) {
        if self.notes.len() < MAX_NOTES {
            self.notes.push(note);
        }
    }

    fn issue(&mut self, note: String) {
        self.complete = false;
        self.note(note);
    }
}

/// Read HKCU/HKLM in both Windows registry views without executing any uninstaller.
/// Duplicate display names are retained: their distinct keys can own shared paths.
pub fn capture() -> Inventory {
    let mut inventory = Inventory {
        entries: Vec::new(),
        complete: true,
        notes: Vec::new(),
    };
    let mut visited = 0_usize;
    for hive in [Hive::User, Hive::Machine] {
        for view in [RegistryView::Registry64, RegistryView::Registry32] {
            if visited >= MAX_ENTRIES {
                inventory.issue(crate::i18n::tr(
                    "Limite de l’inventaire du registre atteinte.",
                ));
                return inventory;
            }
            scan_view(hive, view, &mut visited, &mut inventory);
        }
    }
    inventory
}

/// Match an exact `name:` target or a known stable-channel catalog `id:` target.
/// Protected entries remain in the result so a caller cannot ignore an ownership veto.
pub fn matching_entries(inventory: &Inventory, canonical_target: &str) -> Vec<InstalledEntry> {
    inventory
        .entries
        .iter()
        .filter(|entry| {
            crate::packages::registry_name_matches_target(canonical_target, &entry.name)
        })
        .cloned()
        .collect()
}

fn scan_view(hive: Hive, view: RegistryView, visited: &mut usize, inventory: &mut Inventory) {
    let root = match open_chain(hive, view, UNINSTALL_ROOT) {
        Ok(Some(key)) => key,
        Ok(None) => return,
        Err(error) => {
            inventory.issue(format!("{hive:?}/{view:?} : {error}"));
            return;
        }
    };
    let before = key_stamp(root.0);
    for index in 0..=MAX_ENTRIES as u32 {
        let mut name = [0_u16; 256];
        let mut length = name.len() as u32;
        // SAFETY: the read-only handle and writable UTF-16 buffer live through the call.
        let result = unsafe {
            RegEnumKeyExW(
                root.0,
                index,
                name.as_mut_ptr(),
                &mut length,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if result == ERROR_NO_MORE_ITEMS {
            verify_stamp(
                before,
                key_stamp(root.0),
                &format!("{hive:?}/{view:?}"),
                inventory,
            );
            return;
        }
        if *visited >= MAX_ENTRIES {
            inventory.issue(crate::i18n::tr(
                "Limite de l’inventaire du registre atteinte.",
            ));
            return;
        }
        *visited += 1;
        if result != ERROR_SUCCESS || length as usize >= name.len() {
            inventory.issue(crate::i18n::fmt(
                "{hive}/{view} : énumération impossible ({result}).",
                &[
                    ("hive", format!("{hive:?}")),
                    ("view", format!("{view:?}")),
                    ("result", result.to_string()),
                ],
            ));
            // Do not keep advancing through a changing/inaccessible enumeration.
            return;
        }
        let Ok(child_name) = String::from_utf16(&name[..length as usize]) else {
            inventory.issue(crate::i18n::fmt(
                "{hive}/{view} : nom de clé UTF-16 incorrect.",
                &[("hive", format!("{hive:?}")), ("view", format!("{view:?}"))],
            ));
            continue;
        };
        if !valid_component(&child_name) {
            inventory.issue(crate::i18n::fmt(
                "{hive}/{view} : nom de clé non sûr.",
                &[("hive", format!("{hive:?}")), ("view", format!("{view:?}"))],
            ));
            continue;
        }
        let child = match open_component(root.0, view, &child_name) {
            Ok(Some(key)) => key,
            Ok(None) => {
                // A key vanished after enumeration; this is not a complete snapshot.
                inventory.issue(crate::i18n::fmt(
                    "{hive}/{view} : clé disparue pendant la lecture.",
                    &[("hive", format!("{hive:?}")), ("view", format!("{view:?}"))],
                ));
                continue;
            }
            Err(error) => {
                inventory.issue(format!("{hive:?}/{view:?} : {error}"));
                continue;
            }
        };
        let key = RegistryKey {
            hive,
            view,
            path: format!(r"{UNINSTALL_ROOT}\{child_name}"),
        };
        if let Some(entry) = read_entry(child.0, key, inventory) {
            inventory.entries.push(entry);
        }
    }
    inventory.issue(crate::i18n::tr(
        "Limite d’énumération du registre atteinte.",
    ));
}

fn read_entry(handle: HKEY, key: RegistryKey, inventory: &mut Inventory) -> Option<InstalledEntry> {
    let before = key_stamp(handle);
    let description = format!("{:?}/{:?}/{}", key.hive, key.view, key.path);
    let entry = read_entry_fields(handle, key, inventory);
    verify_stamp(before, key_stamp(handle), &description, inventory);
    entry
}

fn read_entry_fields(
    handle: HKEY,
    key: RegistryKey,
    inventory: &mut Inventory,
) -> Option<InstalledEntry> {
    let mut read = |name| match read_string(handle, name) {
        Ok(value) => value,
        Err(error) => {
            inventory.issue(format!(
                "{:?}/{:?}/{} : {error}",
                key.hive, key.view, key.path
            ));
            None
        }
    };
    let name = read("DisplayName").map(|text| text.value);
    let publisher = read("Publisher").map(|text| text.value).unwrap_or_default();
    let location = read("InstallLocation");
    let icon = read("DisplayIcon");
    let install_location = metadata_path(location.as_ref(), "InstallLocation", &key, inventory);
    let display_icon = metadata_path(icon.as_ref(), "DisplayIcon", &key, inventory);
    let mut name = name.unwrap_or_default();
    if name.trim().is_empty() {
        name.clear();
    }
    if (!name.is_empty() && !valid_name(&name)) || publisher.chars().count() > 2048 {
        inventory.issue(crate::i18n::fmt(
            "{path} : nom ou éditeur incorrect.",
            &[("path", key.path.clone())],
        ));
        return None;
    }
    if name.is_empty() && install_location.is_none() && display_icon.is_none() {
        return None;
    }
    // An unnamed entry cannot be selected, but its known paths still veto cleanup
    // of another product that happens to share the same directory.
    let mut protected = name.is_empty();
    for flag in ["NoRemove", "SystemComponent"] {
        match read_flag(handle, flag) {
            Ok(value) => protected |= value,
            Err(error) => {
                protected = true;
                inventory.issue(format!("{} : {error}", key.path));
            }
        }
    }
    Some(InstalledEntry {
        name,
        publisher,
        install_location,
        display_icon,
        uninstall_key: key,
        protected,
    })
}

struct OpenKey(HKEY);
impl Drop for OpenKey {
    fn drop(&mut self) {
        // SAFETY: only successful opens of non-predefined keys construct this owner.
        unsafe { RegCloseKey(self.0) };
    }
}

/// RegEnumKeyExW has no snapshot semantics: a concurrent key deletion can shift
/// later indexes without returning an error. Verify counts and last-write stamps.
fn key_stamp(handle: HKEY) -> Result<(u32, u32, u64), u32> {
    let mut children = 0;
    let mut values = 0;
    let mut changed = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    // SAFETY: only the three valid writable metadata pointers are non-null.
    let result = unsafe {
        RegQueryInfoKeyW(
            handle,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut children,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut values,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut changed,
        )
    };
    if result != ERROR_SUCCESS {
        return Err(result);
    }
    Ok((
        children,
        values,
        (u64::from(changed.dwHighDateTime) << 32) | u64::from(changed.dwLowDateTime),
    ))
}

fn verify_stamp(
    before: Result<(u32, u32, u64), u32>,
    after: Result<(u32, u32, u64), u32>,
    description: &str,
    inventory: &mut Inventory,
) {
    match (before, after) {
        (Ok(before), Ok(after)) if before == after => {}
        (Ok(_), Ok(_)) => inventory.issue(crate::i18n::fmt(
            "{description} : registre modifié pendant la lecture.",
            &[("description", description.to_string())],
        )),
        (Err(error), _) | (_, Err(error)) => inventory.issue(crate::i18n::fmt(
            "{description} : vérification de stabilité impossible ({error}).",
            &[
                ("description", description.to_string()),
                ("error", error.to_string()),
            ],
        )),
    }
}

fn open_chain(hive: Hive, view: RegistryView, path: &str) -> Result<Option<OpenKey>, String> {
    let mut parent = match hive {
        Hive::User => HKEY_CURRENT_USER,
        Hive::Machine => HKEY_LOCAL_MACHINE,
    };
    let mut owned = None;
    for component in path.split('\\') {
        if !valid_component(component) {
            return Err(crate::i18n::tr("Chemin du registre non sûr."));
        }
        let Some(next) = open_component(parent, view, component)? else {
            return Ok(None);
        };
        parent = next.0;
        owned = Some(next);
    }
    Ok(owned)
}

fn open_component(
    parent: HKEY,
    view: RegistryView,
    component: &str,
) -> Result<Option<OpenKey>, String> {
    let name = wide(component);
    let mut key = std::ptr::null_mut();
    let view = match view {
        RegistryView::Registry32 => KEY_WOW64_32KEY,
        RegistryView::Registry64 => KEY_WOW64_64KEY,
    };
    // SAFETY: names are validated single components, and only read access is requested.
    // Opening one component at a time prevents traversing symbolic-link ancestors.
    let result = unsafe {
        RegOpenKeyExW(
            parent,
            name.as_ptr(),
            REG_OPTION_OPEN_LINK,
            KEY_READ | view,
            &mut key,
        )
    };
    if matches!(result, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
        return Ok(None);
    }
    if result != ERROR_SUCCESS {
        return Err(crate::i18n::fmt(
            "Ouverture du registre impossible ({result}).",
            &[("result", result.to_string())],
        ));
    }
    let key = OpenKey(key);
    let link_name = wide("SymbolicLinkValue");
    let mut kind = 0;
    let mut size = 0;
    // SAFETY: querying metadata only; no link target is followed or expanded.
    let result = unsafe {
        RegQueryValueExW(
            key.0,
            link_name.as_ptr(),
            std::ptr::null(),
            &mut kind,
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if result != ERROR_FILE_NOT_FOUND {
        return Err(if result == ERROR_SUCCESS {
            crate::i18n::tr("Lien de registre refusé.")
        } else {
            crate::i18n::fmt(
                "Vérification du lien de registre impossible ({result}).",
                &[("result", result.to_string())],
            )
        });
    }
    Ok(Some(key))
}

struct RegistryText {
    value: String,
    expand: bool,
}

fn read_string(key: HKEY, name: &str) -> Result<Option<RegistryText>, String> {
    let encoded = wide(name);
    let mut kind = 0;
    let mut size = 0;
    // SAFETY: metadata query writes only to these live type/size pointers.
    let result = unsafe {
        RegQueryValueExW(
            key,
            encoded.as_ptr(),
            std::ptr::null(),
            &mut kind,
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if result != ERROR_SUCCESS {
        return Err(crate::i18n::fmt(
            "Lecture de {name} impossible ({result}).",
            &[("name", name.to_string()), ("result", result.to_string())],
        ));
    }
    if !matches!(kind, REG_SZ | REG_EXPAND_SZ)
        || !size.is_multiple_of(2)
        || size as usize > MAX_VALUE_BYTES
    {
        return Err(crate::i18n::fmt(
            "Type ou taille de {name} incorrect.",
            &[("name", name.to_string())],
        ));
    }
    let mut buffer = vec![0_u16; size as usize / 2 + 1];
    let capacity = buffer.len() * 2;
    size = capacity as u32;
    // SAFETY: buffer has at least the queried size plus one UTF-16 terminator.
    let result = unsafe {
        RegQueryValueExW(
            key,
            encoded.as_ptr(),
            std::ptr::null(),
            &mut kind,
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if result != ERROR_SUCCESS
        || !matches!(kind, REG_SZ | REG_EXPAND_SZ)
        || !size.is_multiple_of(2)
        || size as usize > capacity
        || size as usize > MAX_VALUE_BYTES
    {
        return Err(crate::i18n::fmt(
            "Lecture incohérente de {name} ({result}).",
            &[("name", name.to_string()), ("result", result.to_string())],
        ));
    }
    decode_string(&buffer[..size as usize / 2])
        .map(|value| {
            Some(RegistryText {
                value,
                expand: kind == REG_EXPAND_SZ,
            })
        })
        .ok_or_else(|| {
            crate::i18n::fmt("Texte de {name} incorrect.", &[("name", name.to_string())])
        })
}

/// A nonempty but unknown ownership path cannot simply be ignored: it might be
/// shared with the product being removed, so it vetoes all automatic cleanup.
fn metadata_path(
    text: Option<&RegistryText>,
    field: &str,
    key: &RegistryKey,
    inventory: &mut Inventory,
) -> Option<PathBuf> {
    let text = text.filter(|text| !text.value.trim().is_empty())?;
    let expanded = if text.expand {
        expand_environment(&text.value)
    } else {
        Some(text.value.clone())
    };
    let parsed = expanded.as_deref().and_then(|value| {
        if field == "DisplayIcon" {
            parse_display_icon(value)
        } else {
            parse_location(value)
        }
    });
    if let Some(path) = parsed.and_then(|path| normalize_ownership_path(&path)) {
        return Some(path);
    }
    inventory.issue(crate::i18n::fmt("{path} : propriétaire de {field} inconnu ; nettoyage automatique désactivé pour protéger les chemins partagés.", &[("path", key.path.clone()), ("field", field.to_string())]));
    None
}

/// Windows environment lookup is case-insensitive. This mirrors one-pass
/// ExpandEnvironmentStrings expansion without a shell, command interpretation,
/// recursion or modifying the environment. Unknown variables remain a veto.
fn expand_environment(value: &str) -> Option<String> {
    expand_environment_with(value, |name| std::env::var(name).ok())
}

fn expand_environment_with(
    value: &str,
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> Option<String> {
    if value.matches('%').count() > 128 {
        return None;
    }
    let mut remaining = value;
    let mut expanded = String::with_capacity(value.len());
    while let Some(start) = remaining.find('%') {
        expanded.push_str(&remaining[..start]);
        let after = &remaining[start + 1..];
        let end = after.find('%')?;
        let variable = &after[..end];
        if variable.is_empty() {
            return None;
        }
        expanded.push_str(&lookup(variable)?);
        if expanded.encode_utf16().count() > MAX_VALUE_BYTES / 2 {
            return None;
        }
        remaining = &after[end + 1..];
    }
    expanded.push_str(remaining);
    (expanded.encode_utf16().count() <= MAX_VALUE_BYTES / 2).then_some(expanded)
}

/// Normalize live 8.3 aliases and junction ownership before the uninstaller can
/// remove the directory. For already missing leaves, resolve the first existing
/// ancestor and append only the validated suffix. Unresolved aliases veto cleanup.
fn normalize_ownership_path(path: &Path) -> Option<PathBuf> {
    // Check the original spelling before canonicalize can erase junction/symlink
    // evidence. Inspect from the drive down, never through an unchecked ancestor.
    let ancestors: Vec<_> = path.ancestors().collect();
    for ancestor in ancestors.into_iter().rev() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if metadata.file_attributes()
                    & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                    != 0
                {
                    return None;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return None,
        }
    }
    let mut ancestor = path.to_path_buf();
    let mut suffix = Vec::new();
    let canonical = loop {
        match std::fs::canonicalize(&ancestor) {
            Ok(canonical) => break canonical,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                suffix.push(ancestor.file_name()?.to_os_string());
                if !ancestor.pop() {
                    return None;
                }
            }
            Err(_) => return None,
        }
    };
    if !suffix.is_empty() && !ancestor.is_dir() {
        return None;
    }
    let canonical = canonical.to_str()?;
    let canonical = canonical.strip_prefix(r"\\?\").unwrap_or(canonical);
    let mut normalized = parse_location(canonical)?;
    for component in suffix.into_iter().rev() {
        normalized.push(component);
    }
    let text = normalized.to_str()?;
    // Tildes in missing leaves can be unresolved DOS aliases. Conservatively
    // refuse those rather than claim that their spelling cannot overlap.
    if text.contains('~') {
        return None;
    }
    parse_location(text)
}

fn read_flag(key: HKEY, name: &str) -> Result<bool, String> {
    let name_encoded = wide(name);
    let mut kind = 0;
    let mut value = 0_u32;
    let mut size = std::mem::size_of_val(&value) as u32;
    // SAFETY: the live four-byte value buffer is exactly the advertised size.
    let result = unsafe {
        RegQueryValueExW(
            key,
            name_encoded.as_ptr(),
            std::ptr::null(),
            &mut kind,
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(false);
    }
    if result != ERROR_SUCCESS || kind != REG_DWORD || size != 4 {
        return Err(crate::i18n::fmt(
            "Indicateur {name} incorrect ({result}).",
            &[("name", name.to_string()), ("result", result.to_string())],
        ));
    }
    Ok(value != 0)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn decode_string(buffer: &[u16]) -> Option<String> {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    if buffer[end..].iter().any(|&c| c != 0) {
        return None;
    }
    let value = String::from_utf16(&buffer[..end]).ok()?;
    (!value.chars().any(char::is_control)).then_some(value)
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 2048 && !name.chars().any(char::is_control)
}

fn valid_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with([' ', '.'])
        && name.encode_utf16().count() <= 255
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | '/'))
}

fn parse_location(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    let value = if let Some(quoted) = value.strip_prefix('"') {
        quoted.strip_suffix('"')?
    } else {
        value
    };
    if value.is_empty() || value.chars().any(|c| c.is_control() || c == '"') {
        return None;
    }
    let value = value.replace('/', "\\");
    let bytes = value.as_bytes();
    // Reject UNC/device namespaces, drive-relative paths, URLs and ADS syntax.
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1..3] != *b":\\" {
        return None;
    }
    let tail = value[3..].trim_end_matches('\\');
    if !tail.is_empty()
        && tail.split('\\').any(|component| {
            component.is_empty()
                || matches!(component, "." | "..")
                || component.ends_with([' ', '.'])
                || component
                    .chars()
                    .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '%'))
                || is_dos_device(component)
        })
    {
        return None;
    }
    Some(PathBuf::from(value))
}

fn is_dos_device(component: &str) -> bool {
    let name = component
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            name.strip_prefix(prefix)
                .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
        })
}

/// An icon index belongs to registry metadata, never to the filesystem path.
/// Environment expansion is performed separately, only for REG_EXPAND_SZ.
fn parse_display_icon(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if let Some(quoted) = value.strip_prefix('"') {
        let end = quoted.find('"')?;
        let path = &quoted[..end];
        let suffix = quoted[end + 1..].trim();
        if !suffix.is_empty() {
            suffix.strip_prefix(',')?.trim().parse::<i32>().ok()?;
        }
        return parse_location(path);
    }
    let path = match value.rsplit_once(',') {
        Some((path, index)) if index.trim().parse::<i32>().is_ok() => path.trim_end(),
        _ => value,
    };
    parse_location(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_indexes_and_quotes_are_not_part_of_paths() {
        for (input, expected) in [
            (
                r"C:\Program Files\Acme\app.exe,0",
                r"C:\Program Files\Acme\app.exe",
            ),
            (
                r#""C:\Program Files\Acme\app.exe", -12"#,
                r"C:\Program Files\Acme\app.exe",
            ),
            (
                r#""C:\Apps\Acme, Inc\app.exe""#,
                r"C:\Apps\Acme, Inc\app.exe",
            ),
            (r"C:\Apps\Acme, Inc\app.exe", r"C:\Apps\Acme, Inc\app.exe"),
        ] {
            assert_eq!(
                parse_display_icon(input),
                Some(PathBuf::from(expected)),
                "{input}"
            );
        }
        for input in [
            "",
            r#""C:\app.exe" --remove"#,
            r#""C:\app.exe",bogus"#,
            "C:\\app\n.exe",
            r#"C:\"app.exe"#,
            r"%ProgramFiles%\Acme\app.exe,1",
            r"relative\app.exe",
            r"C:relative.exe",
        ] {
            assert!(parse_display_icon(input).is_none(), "{input}");
        }
    }

    #[test]
    fn expanded_program_files_metadata_preserves_ownership() {
        let expanded = expand_environment_with(r"%pRoGrAmFiLeS%\Acme", |name| {
            name.eq_ignore_ascii_case("ProgramFiles")
                .then(|| r"C:\Program Files".into())
        })
        .unwrap();
        assert_eq!(
            parse_location(&expanded),
            Some(PathBuf::from(r"C:\Program Files\Acme"))
        );
        assert!(expand_environment_with(r"%Unknown%\Acme", |_| None).is_none());
        assert!(expand_environment_with(r"%Unclosed\Acme", |_| Some("C:\\".into())).is_none());
        assert_eq!(
            expand_environment_with("%Outer%", |_| Some("%Inner%".into())),
            Some("%Inner%".into())
        );

        let mut inventory = Inventory {
            entries: Vec::new(),
            complete: true,
            notes: Vec::new(),
        };
        let key = entry("Other product", false).uninstall_key;
        let text = RegistryText {
            value: r"%ProgramFiles%\SuperMenuInventoryMissingFixture\Acme".into(),
            expand: true,
        };
        let path = metadata_path(Some(&text), "InstallLocation", &key, &mut inventory).unwrap();
        let expected = PathBuf::from(std::env::var("ProgramFiles").unwrap())
            .join("SuperMenuInventoryMissingFixture")
            .join("Acme");
        assert_eq!(path, normalize_ownership_path(&expected).unwrap());
        assert!(inventory.complete);
    }

    #[test]
    fn unknown_other_product_paths_veto_the_entire_cleanup_snapshot() {
        for (field, value, expand) in [
            (
                "InstallLocation",
                r"%SuperMenuMissingOwnershipVariable%\Acme",
                true,
            ),
            ("InstallLocation", r"C:\Program Files\%Unknown%\Acme", false),
            ("InstallLocation", r"relative\Acme", false),
            (
                "DisplayIcon",
                r#""C:\Program Files\Shared\app.exe" --arguments"#,
                false,
            ),
            ("DisplayIcon", r"\\server\shared\app.exe,0", false),
        ] {
            let mut inventory = Inventory {
                entries: vec![entry("Target product", false)],
                complete: true,
                notes: Vec::new(),
            };
            let key = entry("Other product", false).uninstall_key;
            let text = RegistryText {
                value: value.into(),
                expand,
            };
            assert!(metadata_path(Some(&text), field, &key, &mut inventory).is_none());
            assert!(!inventory.complete, "{value}");
            assert!(inventory
                .notes
                .iter()
                .any(|note| note.contains("chemins partagés")));
        }
        for value in [
            r"C:relative",
            r"\\?\C:\Acme",
            r"C:\Acme\..\Other",
            r"C:\Acme\file:stream",
            r"C:\Acme\CON.txt",
            r"C:\Acme\folder.",
            r"C:\Acme\\Other",
        ] {
            assert!(parse_location(value).is_none(), "{value}");
        }
    }

    #[test]
    fn short_names_and_missing_leaves_use_existing_ancestor_identity() {
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
        let programs = std::env::var("ProgramFiles").unwrap();
        let encoded = wide(&programs);
        let mut short = vec![0_u16; 32768];
        // SAFETY: only the existing system path is read; the output buffer is live.
        let length =
            unsafe { GetShortPathNameW(encoded.as_ptr(), short.as_mut_ptr(), short.len() as u32) }
                as usize;
        assert!(length > 0 && length < short.len());
        let short = PathBuf::from(String::from_utf16(&short[..length]).unwrap());
        let missing = "SuperMenuInventoryMissingFixture";
        assert_eq!(
            normalize_ownership_path(&short.join(missing).join("Acme")),
            normalize_ownership_path(&PathBuf::from(programs).join(missing).join("Acme")),
        );
        assert!(normalize_ownership_path(&short.join("SuperMenuMissing~1")).is_none());
    }

    #[test]
    fn original_reparse_ancestors_cannot_be_hidden_by_canonicalization() {
        let temp = std::env::temp_dir();
        let root = temp.join(format!(
            "supermenu-inventory-fixture-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("Actual");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("app.exe"), b"fixture").unwrap();
        let link = root.join("SelectedApp");
        assert!(normalize_ownership_path(&target).is_some());
        // Windows may require Developer Mode/privilege to create the fixture link.
        if std::os::windows::fs::symlink_dir(&target, &link).is_ok() {
            assert!(normalize_ownership_path(&link).is_none());
            assert!(normalize_ownership_path(&link.join("app.exe")).is_none());
            assert!(normalize_ownership_path(&link.join("missing.exe")).is_none());
            std::fs::remove_dir(&link).unwrap();
        }
        assert_eq!(std::fs::read(target.join("app.exe")).unwrap(), b"fixture");
        // Delete only this freshly created fixture, after confirming its parent.
        assert_eq!(root.parent(), Some(temp.as_path()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("supermenu-inventory-fixture-"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn text_and_keys_reject_embedded_nuls_and_invalid_utf16() {
        assert_eq!(decode_string(&[65, 66]), Some("AB".into()));
        assert_eq!(decode_string(&[65, 0, 0]), Some("A".into()));
        assert_eq!(decode_string(&[65, 0, 66]), None);
        assert_eq!(decode_string(&[0xD800]), None);
        assert_eq!(decode_string(&[65, 10]), None);
        for value in [
            "",
            ".",
            "..",
            "bad\\child",
            "bad/child",
            "bad\0name",
            "bad.",
            "bad ",
        ] {
            assert!(!valid_component(value));
        }
        assert!(valid_component("{ABC-123} Acme 2.0"));
        // Preserve actual registry spelling, including PostgreSQL's trailing space.
        assert!(valid_name("PostgreSQL 17 "));
        assert!(!valid_name(" "));
    }

    fn entry(name: &str, protected: bool) -> InstalledEntry {
        InstalledEntry {
            name: name.into(),
            publisher: "Acme".into(),
            install_location: None,
            display_icon: None,
            uninstall_key: RegistryKey {
                hive: Hive::User,
                view: RegistryView::Registry64,
                path: format!(r"{UNINSTALL_ROOT}\{name}"),
            },
            protected,
        }
    }

    #[test]
    fn matching_is_exact_and_retains_protected_and_duplicate_owners() {
        let inventory = Inventory {
            entries: vec![
                entry("Acme Writer 2.4", false),
                entry("Acme Writer 2.4", true),
                entry("Acme Writer 2.4 Beta", false),
            ],
            complete: false,
            notes: vec!["Access denied".into()],
        };
        let found = matching_entries(&inventory, "name:Acme Writer 2.4");
        assert_eq!(found.len(), 2);
        assert!(found[1].protected);
        assert!(matching_entries(&inventory, "name:Acme Writer").is_empty());
        assert!(matching_entries(&inventory, "id:Acme.Writer").is_empty());
        assert!(matching_entries(&inventory, "Acme Writer 2.4").is_empty());
    }

    #[test]
    fn catalog_matching_does_not_confuse_stable_and_variant_channels() {
        let inventory = Inventory {
            entries: vec![
                entry("Google Chrome", false),
                entry("Google Chrome Beta", false),
                entry("Mozilla Firefox ESR (x64 fr)", false),
            ],
            complete: true,
            notes: Vec::new(),
        };
        assert_eq!(matching_entries(&inventory, "id:Google.Chrome").len(), 1);
        assert!(matching_entries(&inventory, "id:Mozilla.Firefox").is_empty());
    }
}
