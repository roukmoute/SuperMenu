//! Conservative, worker-only cleanup after a verified successful uninstall.
//!
//! Ownership is captured before the uninstaller runs, then checked against a
//! complete fresh inventory before each mutation. Profiles and arbitrary product
//! registry keys are deliberately outside this module's scope. Files are recycled
//! with Windows' explicit RECYCLEONDELETE flag; there is no permanent-delete path.
use crate::com::{self, ComPtr, IFileOperation, IPersistFile, IShellItem, IShellLinkW};
use crate::uninstall_inventory::{
    self, Hive, InstalledEntry, Inventory, RegistryKey, RegistryView,
};
use serde::Serialize;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
};
use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW,
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE,
    KEY_WOW64_32KEY, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_OPTION_OPEN_LINK, REG_SZ,
};
use windows_sys::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, FOLDERID_Desktop, FOLDERID_Programs, FOLDERID_PublicDesktop,
    FileOperation, SHGetKnownFolderPath, ShellLink, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE,
    FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_NO_CONNECTED_ELEMENTS, FOF_SILENT,
    KF_FLAG_DEFAULT, SLGP_RAWPATH,
};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CompareStringOrdinal(
        left: *const u16,
        left_len: i32,
        right: *const u16,
        right_len: i32,
        ignore_case: i32,
    ) -> i32;
}

const REPARSE_POINT: u32 = 0x400;
const MAX_CANDIDATES: usize = 256;
const MAX_TREE_ENTRIES: usize = 50_000;
const MAX_TREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_TREE_DEPTH: usize = 64;
const MAX_REGISTRY_ENTRIES: usize = 4096;
const MAX_REGISTRY_BYTES: usize = 65_536;
const APP_PATHS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
const RUN: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Serialize)]
pub struct CleanupItem {
    pub kind: String,
    pub location: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Serialize)]
pub struct CleanupReport {
    pub target: String,
    pub inspection_only: bool,
    pub removed: Vec<CleanupItem>,
    pub retained: Vec<CleanupItem>,
    pub notes: Vec<String>,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone)]
enum FileKind {
    Program,
    Cache,
    Shortcut(PathBuf),
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone)]
struct FileCandidate {
    path: PathBuf,
    scope: PathBuf,
    kind: FileKind,
    fingerprint: Option<FileFingerprint>,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, PartialEq, Eq)]
struct FileFingerprint {
    length: u64,
    created: u64,
    written: u64,
}

fn file_fingerprint(path: &Path) -> Result<FileFingerprint, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        crate::i18n::fmt(
            "Fichier impossible à identifier : {error}",
            &[("error", error.to_string())],
        )
    })?;
    if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(crate::i18n::tr("Fichier non ordinaire ; conservé."));
    }
    Ok(FileFingerprint {
        length: metadata.len(),
        created: metadata.creation_time(),
        written: metadata.last_write_time(),
    })
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Serialize)]
struct RegistryCandidate {
    key: RegistryKey,
    value: String,
    value_type: u32,
    bytes: Vec<u8>,
    executable: PathBuf,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone)]
struct Roots {
    programs: Vec<PathBuf>,
    local: Option<PathBuf>,
    roaming: Option<PathBuf>,
}

impl Roots {
    fn current() -> Self {
        let local = environment_root("LOCALAPPDATA");
        let roaming = environment_root("APPDATA");
        let mut programs: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
            .iter()
            .filter_map(|name| environment_root(name))
            .collect();
        if let Some(local) = &local {
            programs.push(local.join("Programs"));
        }
        let mut seen = HashSet::new();
        programs.retain(|path| seen.insert(path_key(path)));
        Self {
            programs,
            local,
            roaming,
        }
    }
}

/// An immutable ownership snapshot. Capture performs only bounded read operations.
pub struct CleanupPlan {
    target: String,
    before: Inventory,
    selected: Option<InstalledEntry>,
    roots: Roots,
    program_roots: Vec<PathBuf>,
    files: Vec<FileCandidate>,
    registry: Vec<RegistryCandidate>,
    retained: Vec<CleanupItem>,
    notes: Vec<String>,
    veto: Option<String>,
}

impl CleanupPlan {
    pub fn capture(canonical_target: &str) -> Self {
        let mut plan = Self::from_inventory(
            canonical_target,
            uninstall_inventory::capture(),
            Roots::current(),
        );
        if plan.veto.is_none() && !plan.program_roots.is_empty() {
            plan.capture_registry();
            plan.capture_shortcuts();
        }
        plan
    }

    pub fn summary(&self) -> String {
        if let Some(reason) = &self.veto {
            return crate::i18n::fmt(
                "Nettoyage automatique désactivé : {reason}",
                &[("reason", reason.to_string())],
            );
        }
        crate::i18n::fmt("Nettoyage automatique : {files} emplacement(s) de programme/cache, {registry} trace(s) technique(s). Profils et données personnelles conservés.", &[("files", self.files.len().to_string()), ("registry", self.registry.len().to_string())])
    }

    /// Inspection never writes backups, registry values or files.
    pub fn preview_report(&self) -> CleanupReport {
        let mut report = self.base_report(true);
        for candidate in &self.files {
            report
                .retained
                .push(file_item(candidate, "À vérifier après la désinstallation."));
        }
        for candidate in &self.registry {
            report.retained.push(registry_item(
                candidate,
                "À vérifier et sauvegarder après la désinstallation.",
                None,
            ));
        }
        report
    }

    /// The caller must first verify both uninstall success and WinGet absence.
    /// This independent second gate also refuses an incomplete registry inventory.
    pub fn execute_after_success(&self) -> CleanupReport {
        self.execute_with(uninstall_inventory::capture, recycle)
    }

    fn from_inventory(target: &str, before: Inventory, roots: Roots) -> Self {
        let (selected, veto) = select_entry(&before, target);
        let mut plan = Self {
            target: target.into(),
            before,
            selected,
            roots,
            program_roots: Vec::new(),
            files: Vec::new(),
            registry: Vec::new(),
            retained: Vec::new(),
            notes: vec![
                crate::i18n::tr("Les profils, réglages, sessions, sauvegardes et autres données personnelles sont conservés."),
                crate::i18n::tr("Services, pilotes, tâches planifiées, COM, MSI, associations de fichiers et clés de paramètres partagées ne sont pas modifiés."),
                crate::i18n::tr("Seuls les raccourcis .lnk dont la cible exécutable exacte appartient au programme sont recyclés ; leur titre ne sert jamais à deviner le propriétaire."),
            ],
            veto,
        };
        plan.notes
            .extend(plan.before.notes.iter().take(64).cloned());
        if plan.veto.is_none() {
            plan.capture_files();
        }
        plan
    }

    fn base_report(&self, inspection_only: bool) -> CleanupReport {
        let mut notes = self.notes.clone();
        if let Some(veto) = &self.veto {
            notes.push(crate::i18n::fmt(
                "Aucun nettoyage : {veto}",
                &[("veto", veto.to_string())],
            ));
        }
        CleanupReport {
            target: self.target.clone(),
            inspection_only,
            removed: Vec::new(),
            retained: self.retained.clone(),
            notes,
        }
    }

    fn capture_files(&mut self) {
        let Some(entry) = self.selected.clone() else {
            return;
        };
        let mut proposed = Vec::new();
        if let Some(location) = &entry.install_location {
            proposed.push(location.clone());
        }
        if let Some(icon) = &entry.display_icon {
            if let Some(parent) = icon.parent() {
                proposed.push(parent.to_path_buf());
            }
        }
        let mut seen = HashSet::new();
        for path in proposed {
            if !seen.insert(path_key(&path)) {
                continue;
            }
            match approved_program_path(&path, &entry, &self.roots) {
                Ok(scope) => {
                    let candidate = FileCandidate {
                        path: path.clone(),
                        scope,
                        kind: FileKind::Program,
                        fingerprint: None,
                    };
                    if self.owned_file(&candidate, &self.before).is_ok() {
                        self.program_roots.push(path);
                    } else {
                        self.retained.push(file_item(
                            &candidate,
                            "Emplacement partagé avec un autre logiciel.",
                        ));
                    }
                }
                Err(reason) => self.retained.push(CleanupItem {
                    kind: "programme".into(),
                    location: path.display().to_string(),
                    reason,
                    backup: None,
                }),
            }
        }
        // Do not recycle a whole installation directory: applications can keep
        // personal settings or saved games beside their executables. Only known
        // binary file types are program remnants; everything else remains intact.
        for root in &self.program_roots {
            match program_binaries(root) {
                Ok(binaries) => {
                    for path in binaries {
                        if self.files.len() >= MAX_CANDIDATES {
                            break;
                        }
                        if !seen.insert(path_key(&path)) {
                            continue;
                        }
                        let Ok(fingerprint) = file_fingerprint(&path) else {
                            continue;
                        };
                        self.files.push(FileCandidate {
                            path,
                            scope: root.clone(),
                            kind: FileKind::Program,
                            fingerprint: Some(fingerprint),
                        });
                    }
                }
                Err(reason) => self.notes.push(reason),
            }
        }
        self.notes.push(crate::i18n::tr("Dans les anciens dossiers d’installation, seuls les binaires .exe/.dll/.pdb/.manifest sont recyclés ; dossiers et ressources non attribuables sont conservés afin de préserver d’éventuelles données personnelles."));
        let caches = known_caches(&self.target, &entry, &self.roots);
        if caches.is_empty() {
            self.notes.push(crate::i18n::tr("Aucune règle de cache spécifique applicable ; les dossiers AppData et ProgramData du produit sont conservés."));
        }
        for (path, scope) in caches.into_iter().take(MAX_CANDIDATES) {
            if !seen.insert(path_key(&path)) {
                continue;
            }
            let candidate = FileCandidate {
                path,
                scope,
                kind: FileKind::Cache,
                fingerprint: None,
            };
            if self.files.len() >= MAX_CANDIDATES {
                self.notes.push(crate::i18n::tr("Limite des emplacements de nettoyage atteinte ; les autres emplacements sont conservés."));
                break;
            }
            if let Err(reason) = self.owned_file(&candidate, &self.before) {
                self.retained.push(file_item(&candidate, &reason));
            } else if candidate.path.exists() {
                self.files.push(candidate);
            }
        }
    }

    fn owned_file(&self, candidate: &FileCandidate, inventory: &Inventory) -> Result<(), String> {
        if !is_within(&candidate.path, &candidate.scope)
            || same_path(&candidate.path, &candidate.scope)
        {
            return Err(crate::i18n::tr(
                "Emplacement trop large ou hors du périmètre autorisé.",
            ));
        }
        safe_ancestors(&candidate.path)?;
        if let Some(expected) = &candidate.fingerprint {
            match file_fingerprint(&candidate.path) {
                Ok(current) if &current != expected => {
                    return Err(crate::i18n::tr(
                        "Le fichier du programme a changé depuis l’inspection ; conservé.",
                    ))
                }
                Ok(_) => {}
                Err(_) if !candidate.path.exists() => {}
                Err(reason) => return Err(reason),
            }
        }
        if let FileKind::Shortcut(executable) = &candidate.kind {
            self.owns_executable(executable, inventory)?;
            if !same_path(&shortcut_target(&candidate.path)?, executable) {
                return Err(crate::i18n::tr(
                    "La cible du raccourci a changé ; raccourci conservé.",
                ));
            }
        }
        let Some(selected) = &self.selected else {
            return Err(crate::i18n::tr("Propriétaire non identifié."));
        };
        for other in &inventory.entries {
            if logical_entry_eq(other, selected) {
                continue;
            }
            if entry_overlaps(other, &candidate.path) {
                return Err(crate::i18n::tr(
                    "Emplacement partagé avec un autre logiciel installé.",
                ));
            }
            if matches!(candidate.kind, FileKind::Cache) && cache_family_shared(selected, other) {
                return Err(crate::i18n::tr(
                    "Un autre canal ou logiciel utilise les mêmes profils ; caches conservés.",
                ));
            }
        }
        Ok(())
    }

    fn absence_gate(&self, inventory: &Inventory) -> Result<(), String> {
        if let Some(veto) = &self.veto {
            return Err(veto.clone());
        }
        if !inventory.complete {
            return Err(crate::i18n::tr("L’inventaire après désinstallation est incomplet ; tous les résidus sont conservés."));
        }
        if !uninstall_inventory::matching_entries(inventory, &self.target).is_empty() {
            return Err(crate::i18n::tr(
                "Le logiciel apparaît encore dans l’inventaire ; tous les résidus sont conservés.",
            ));
        }
        Ok(())
    }

    fn execute_with<I, R>(&self, mut inventory: I, mut recycler: R) -> CleanupReport
    where
        I: FnMut() -> Inventory,
        R: FnMut(&Path) -> Result<(), String>,
    {
        let mut report = self.base_report(false);
        let initial = inventory();
        if let Err(reason) = self.absence_gate(&initial) {
            report.notes.push(reason.clone());
            for candidate in &self.files {
                report.retained.push(file_item(candidate, &reason));
            }
            for candidate in &self.registry {
                report
                    .retained
                    .push(registry_item(candidate, &reason, None));
            }
            return report;
        }
        for candidate in &self.files {
            // Re-read immediately before each mutation: software can be installed
            // concurrently while another product's uninstaller is running.
            let fresh = inventory();
            let validation = self
                .absence_gate(&fresh)
                .and_then(|()| self.owned_file(candidate, &fresh))
                .and_then(|()| preflight_tree(&candidate.path));
            if let Err(reason) = validation {
                report.retained.push(file_item(candidate, &reason));
                continue;
            }
            match fs::symlink_metadata(&candidate.path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    report.retained.push(file_item(
                        candidate,
                        &crate::i18n::fmt(
                            "Lecture impossible : {error}",
                            &[("error", error.to_string())],
                        ),
                    ));
                    continue;
                }
                Ok(_) => {}
            }
            let _parents = match pin_ancestors(&candidate.path) {
                Ok(parents) => parents,
                Err(reason) => {
                    report.retained.push(file_item(candidate, &reason));
                    continue;
                }
            };
            if let Err(reason) = self
                .owned_file(candidate, &fresh)
                .and_then(|()| preflight_tree(&candidate.path))
            {
                report.retained.push(file_item(candidate, &reason));
                continue;
            }
            match recycler(&candidate.path) {
                Ok(()) => report
                    .removed
                    .push(file_item(candidate, "Déplacé dans la Corbeille Windows.")),
                Err(reason) => report.retained.push(file_item(candidate, &reason)),
            }
        }
        for candidate in &self.registry {
            let fresh = inventory();
            let validation = self
                .absence_gate(&fresh)
                .and_then(|()| self.owns_executable(&candidate.executable, &fresh))
                .and_then(|()| registry_unchanged(candidate));
            if let Err(reason) = validation {
                report
                    .retained
                    .push(registry_item(candidate, &reason, None));
                continue;
            }
            let backup = match backup_registry(candidate) {
                Ok(path) => path,
                Err(reason) => {
                    report
                        .retained
                        .push(registry_item(candidate, &reason, None));
                    continue;
                }
            };
            let final_inventory = inventory();
            if let Err(reason) = self
                .absence_gate(&final_inventory)
                .and_then(|()| self.owns_executable(&candidate.executable, &final_inventory))
            {
                report
                    .retained
                    .push(registry_item(candidate, &reason, Some(&backup)));
                continue;
            }
            // The backup is durable before requesting write access. Compare again
            // through the write handle to protect updates made during the backup.
            match delete_registry_value(candidate) {
                Ok(()) => report.removed.push(registry_item(
                    candidate,
                    "Valeur technique supprimée après sauvegarde.",
                    Some(&backup),
                )),
                Err(reason) => {
                    report
                        .retained
                        .push(registry_item(candidate, &reason, Some(&backup)))
                }
            }
        }
        report
    }

    fn owns_executable(&self, executable: &Path, inventory: &Inventory) -> Result<(), String> {
        safe_ancestors(executable)?;
        if !executable
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            return Err(crate::i18n::tr(
                "La trace ne désigne pas un exécutable exact.",
            ));
        }
        let Some(root) = self
            .program_roots
            .iter()
            .find(|root| is_within(executable, root))
        else {
            return Err(crate::i18n::tr(
                "Exécutable hors des dossiers du programme identifiés avant désinstallation.",
            ));
        };
        if same_path(executable, root) {
            return Err(crate::i18n::tr("Exécutable non identifié."));
        }
        let Some(selected) = &self.selected else {
            return Err(crate::i18n::tr("Propriétaire non identifié."));
        };
        if inventory
            .entries
            .iter()
            .any(|other| !logical_entry_eq(other, selected) && entry_overlaps(other, root))
        {
            return Err(crate::i18n::tr(
                "Dossier exécutable partagé avec un autre logiciel installé.",
            ));
        }
        Ok(())
    }

    fn capture_registry(&mut self) {
        let mut visited = 0;
        for hive in [Hive::User, Hive::Machine] {
            for view in [RegistryView::Registry64, RegistryView::Registry32] {
                for path in [RUN, APP_PATHS] {
                    if visited >= MAX_REGISTRY_ENTRIES {
                        self.notes
                            .push("Limite d’inspection des traces techniques atteinte.".into());
                        return;
                    }
                    let key = RegistryKey {
                        hive,
                        view,
                        path: path.into(),
                    };
                    let result = if path == RUN {
                        scan_run(&key, &mut visited)
                    } else {
                        scan_app_paths(&key, &mut visited)
                    };
                    match result {
                        Ok(candidates) => {
                            for candidate in candidates {
                                if self.registry.len() < MAX_CANDIDATES
                                    && self
                                        .owns_executable(&candidate.executable, &self.before)
                                        .is_ok()
                                {
                                    self.registry.push(candidate);
                                }
                            }
                        }
                        Err(reason) => self.notes.push(reason),
                    }
                }
            }
        }
        // HKCU commonly exposes the same logical value in both views. Retain one
        // operation, but include the explicit view in its reversible backup.
        let mut seen = HashSet::new();
        self.registry.retain(|candidate| {
            seen.insert((
                candidate.key.hive,
                if candidate.key.hive == Hive::Machine {
                    Some(candidate.key.view)
                } else {
                    None
                },
                candidate.key.path.to_lowercase(),
                candidate.value.to_lowercase(),
                candidate.bytes.clone(),
            ))
        });
    }

    fn capture_shortcuts(&mut self) {
        let mut seen: HashSet<String> = self
            .files
            .iter()
            .map(|candidate| path_key(&candidate.path))
            .collect();
        for root in shortcut_roots() {
            match shortcut_files(&root) {
                Ok(paths) => {
                    for path in paths {
                        if self.files.len() >= MAX_CANDIDATES {
                            self.notes.push(crate::i18n::tr("Limite des résidus atteinte ; les autres raccourcis sont conservés."));
                            return;
                        }
                        if !seen.insert(path_key(&path)) {
                            continue;
                        }
                        let Ok(executable) = shortcut_target(&path) else {
                            continue;
                        };
                        if self.owns_executable(&executable, &self.before).is_ok() {
                            self.files.push(FileCandidate {
                                path,
                                scope: root.clone(),
                                kind: FileKind::Shortcut(executable),
                                fingerprint: None,
                            });
                        }
                    }
                }
                Err(reason) => self.notes.push(reason),
            }
        }
    }
}

fn select_entry(inventory: &Inventory, target: &str) -> (Option<InstalledEntry>, Option<String>) {
    if !inventory.complete {
        return (
            None,
            Some(crate::i18n::tr(
                "L’inventaire avant désinstallation est incomplet.",
            )),
        );
    }
    let matching = uninstall_inventory::matching_entries(inventory, target);
    if matching.is_empty() {
        return (
            None,
            Some(crate::i18n::tr(
                "Aucune entrée de registre exacte ne prouve le propriétaire des résidus.",
            )),
        );
    }
    if matching.iter().any(|entry| entry.protected) {
        return (
            None,
            Some(crate::i18n::tr("L’entrée du logiciel est protégée.")),
        );
    }
    let selected = matching[0].clone();
    if matching
        .iter()
        .any(|entry| !logical_entry_eq(&selected, entry))
    {
        return (
            None,
            Some(crate::i18n::tr(
                "Plusieurs produits ou versions correspondent à la cible ; attribution ambiguë.",
            )),
        );
    }
    (Some(selected), None)
}

fn logical_entry_eq(left: &InstalledEntry, right: &InstalledEntry) -> bool {
    left.uninstall_key.hive == right.uninstall_key.hive
        && left
            .uninstall_key
            .path
            .eq_ignore_ascii_case(&right.uninstall_key.path)
        && left.name.eq_ignore_ascii_case(&right.name)
        && left.publisher.eq_ignore_ascii_case(&right.publisher)
        && optional_same_path(&left.install_location, &right.install_location)
        && optional_same_path(&left.display_icon, &right.display_icon)
}

fn optional_same_path(left: &Option<PathBuf>, right: &Option<PathBuf>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => same_path(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn entry_overlaps(entry: &InstalledEntry, candidate: &Path) -> bool {
    entry
        .install_location
        .as_ref()
        .is_some_and(|path| paths_overlap(path, candidate))
        || entry
            .display_icon
            .as_ref()
            .and_then(|path| path.parent())
            .is_some_and(|path| paths_overlap(path, candidate))
}

fn approved_program_path(
    path: &Path,
    entry: &InstalledEntry,
    roots: &Roots,
) -> Result<PathBuf, String> {
    safe_ancestors(path)?;
    let Some(scope) = roots
        .programs
        .iter()
        .find(|root| is_within(path, root) && !same_path(path, root))
    else {
        return Err(crate::i18n::tr(
            "Hors de Program Files ou LocalAppData\\Programs ; dossier conservé.",
        ));
    };
    let relative = relative_components(path, scope).ok_or("Chemin du programme invalide.")?;
    if relative.iter().any(|part| {
        matches!(
            part.as_str(),
            "common files" | "shared" | "windows" | "installer" | "microsoft shared"
        )
    }) {
        return Err(crate::i18n::tr("Dossier partagé ou système ; conservé."));
    }
    let Some(leaf) = relative.last() else {
        return Err(crate::i18n::tr("Dossier trop large."));
    };
    let display_name = compact_name(&entry.name);
    let generic_name = compact_name(leaf);
    let publisher = compact_name(&entry.publisher);
    let dedicated_name = !display_name.is_empty()
        && generic_name == display_name
        && generic_name != publisher
        && !matches!(
            leaf.as_str(),
            "application"
                | "applications"
                | "programs"
                | "software"
                | "bin"
                | "data"
                | "cache"
                | "microsoft"
                | "google"
                | "mozilla"
                | "bravesoftware"
                | "videolan"
        );
    let known = known_program_relative(&relative.join("\\"), entry);
    // An exact product leaf alone is insufficient: the uninstall metadata must
    // also identify a concrete executable in this independently approved folder.
    let concrete_icon = entry.display_icon.as_ref().is_some_and(|icon| {
        is_within(icon, path)
            && !same_path(icon, path)
            && icon
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
            && safe_ancestors(icon).is_ok()
    });
    if !(dedicated_name || known) || !concrete_icon {
        return Err(crate::i18n::tr(
            "Dossier non dédié ou absence de preuve exécutable exacte ; conservé.",
        ));
    }
    preflight_tree(path)?;
    Ok(scope.clone())
}

fn known_program_relative(relative: &str, entry: &InstalledEntry) -> bool {
    let icon = entry
        .display_icon
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        (relative, icon.as_str()),
        ("google\\chrome\\application", "chrome.exe")
            | ("google\\chrome", "chrome.exe")
            | ("bravesoftware\\brave-browser\\application", "brave.exe")
            | ("bravesoftware\\brave-browser", "brave.exe")
            | ("vivaldi\\application", "vivaldi.exe")
            | ("vivaldi", "vivaldi.exe")
            | ("mozilla firefox", "firefox.exe")
            | ("microsoft vs code", "code.exe")
            | ("microsoft vs code\\bin", "code.exe")
            | ("videolan\\vlc", "vlc.exe")
            | ("obs-studio", "obs64.exe")
            | ("obs-studio\\bin\\64bit", "obs64.exe")
            | ("streamfab\\streamfab", "streamfab64.exe")
    )
}

fn compact_name(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn cache_family_shared(selected: &InstalledEntry, other: &InstalledEntry) -> bool {
    let file_name = |entry: &InstalledEntry| {
        entry
            .display_icon
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_lowercase())
    };
    let Some(selected_name) = file_name(selected) else {
        return true;
    };
    file_name(other).is_some_and(|other_name| other_name == selected_name)
}

fn known_caches(target: &str, entry: &InstalledEntry, roots: &Roots) -> Vec<(PathBuf, PathBuf)> {
    let mut result = Vec::new();
    let name = compact_name(&entry.name);
    let icon = entry
        .display_icon
        .as_ref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let family = match (target, name.as_str(), icon.as_str()) {
        ("id:Google.Chrome", _, "chrome.exe") | (_, "googlechrome", "chrome.exe") => "chrome",
        ("id:Brave.Brave", _, "brave.exe") | (_, "brave", "brave.exe") => "brave",
        ("id:Vivaldi.Vivaldi", _, "vivaldi.exe") | (_, "vivaldi", "vivaldi.exe") => "vivaldi",
        ("id:Mozilla.Firefox", _, "firefox.exe")
        | (_, "mozillafirefox" | "firefox", "firefox.exe") => "firefox",
        ("id:Microsoft.VisualStudioCode", _, "code.exe")
        | (_, "microsoftvisualstudiocode" | "visualstudiocode", "code.exe") => "code",
        ("id:Discord.Discord", _, "discord.exe") | (_, "discord", "discord.exe") => "discord",
        ("id:Spotify.Spotify", _, "spotify.exe") | (_, "spotify", "spotify.exe") => "spotify",
        _ => return result,
    };
    let add_children = |parent: &Path, children: &[&str], result: &mut Vec<(PathBuf, PathBuf)>| {
        for child in children {
            result.push((parent.join(child), parent.to_path_buf()));
        }
    };
    match family {
        "chrome" | "brave" | "vivaldi" => {
            let Some(local) = &roots.local else {
                return result;
            };
            let profile_root = local.join(match family {
                "chrome" => r"Google\Chrome\User Data",
                "brave" => r"BraveSoftware\Brave-Browser\User Data",
                _ => r"Vivaldi\User Data",
            });
            if safe_ancestors(&profile_root).is_err() {
                return result;
            }
            let Ok(profiles) = fs::read_dir(&profile_root) else {
                return result;
            };
            for profile in profiles.take(64).flatten() {
                let path = profile.path();
                let name = profile.file_name().to_string_lossy().to_string();
                if name == "Default"
                    || name.strip_prefix("Profile ").is_some_and(|suffix| {
                        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
                    })
                {
                    add_children(
                        &path,
                        &[
                            "Cache",
                            "Code Cache",
                            "GPUCache",
                            "DawnCache",
                            "GrShaderCache",
                            "ShaderCache",
                        ],
                        &mut result,
                    );
                }
            }
        }
        "firefox" => {
            let Some(local) = &roots.local else {
                return result;
            };
            let profile_root = local.join(r"Mozilla\Firefox\Profiles");
            if safe_ancestors(&profile_root).is_err() {
                return result;
            }
            let Ok(profiles) = fs::read_dir(profile_root) else {
                return result;
            };
            for profile in profiles.take(64).flatten() {
                add_children(
                    &profile.path(),
                    &["cache2", "startupCache", "shader-cache"],
                    &mut result,
                );
            }
        }
        "code" | "discord" => {
            let Some(roaming) = &roots.roaming else {
                return result;
            };
            let root = roaming.join(if family == "code" { "Code" } else { "discord" });
            add_children(&root, &["Cache", "Code Cache", "GPUCache"], &mut result);
        }
        "spotify" => {
            let Some(local) = &roots.local else {
                return result;
            };
            add_children(
                &local.join(r"Spotify\Browser"),
                &["Cache", "Code Cache", "GPUCache"],
                &mut result,
            );
        }
        _ => {}
    }
    result
}

fn environment_root(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os(name)?);
    if safe_ancestors(&path).is_err() {
        return None;
    }
    let canonical = fs::canonicalize(&path).ok()?;
    let canonical = canonical.to_str()?;
    let canonical = canonical.strip_prefix(r"\\?\").unwrap_or(canonical);
    let path = PathBuf::from(canonical);
    (safe_ancestors(&path).is_ok() && path_components(&path).is_some()).then_some(path)
}

/// Lexical Windows paths are checked before Rust components can normalize `.`.
/// Device paths, UNC, ADS, wildcards and trailing-dot/space aliases are rejected.
fn path_components(path: &Path) -> Option<Vec<String>> {
    let text = path.to_str()?;
    let bytes = text.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
    {
        return None;
    }
    let mut result = vec![text[..2].to_string()];
    for component in text[3..].split(['\\', '/']) {
        if component.is_empty() {
            continue;
        }
        if component == "."
            || component == ".."
            || component.contains('~')
            || component.ends_with(['.', ' '])
            || component.chars().any(|character| {
                character < ' ' || matches!(character, ':' | '*' | '?' | '"' | '<' | '>' | '|')
            })
        {
            return None;
        }
        let stem = component.split('.').next()?.to_ascii_lowercase();
        if matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
            || (stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return None;
        }
        result.push(component.to_string());
    }
    Some(result)
}

fn path_key(path: &Path) -> String {
    path_components(path)
        .map(|parts| parts.join("\\").to_lowercase())
        .unwrap_or_else(|| path.to_string_lossy().to_lowercase())
}

fn component_equal(left: &str, right: &str) -> bool {
    let left: Vec<u16> = left.encode_utf16().collect();
    let right: Vec<u16> = right.encode_utf16().collect();
    // CSTR_EQUAL = 2. Windows ordinal case rules differ from Unicode lowercase;
    // use the actual Windows comparison for ownership and containment decisions.
    unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
            1,
        ) == 2
    }
}

fn component_prefix(path: &[String], root: &[String]) -> bool {
    path.len() >= root.len()
        && path
            .iter()
            .zip(root)
            .all(|(left, right)| component_equal(left, right))
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (path_components(left), path_components(right)) {
        (Some(left), Some(right)) => left.len() == right.len() && component_prefix(&left, &right),
        _ => false,
    }
}

fn is_within(path: &Path, root: &Path) -> bool {
    match (path_components(path), path_components(root)) {
        (Some(path), Some(root)) => component_prefix(&path, &root),
        _ => false,
    }
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    is_within(left, right) || is_within(right, left)
}

fn relative_components(path: &Path, root: &Path) -> Option<Vec<String>> {
    let path = path_components(path)?;
    let root = path_components(root)?;
    component_prefix(&path, &root).then(|| {
        path[root.len()..]
            .iter()
            .map(|part| part.to_ascii_lowercase())
            .collect()
    })
}

fn safe_ancestors(path: &Path) -> Result<(), String> {
    if path_components(path).is_none() {
        return Err(crate::i18n::tr(
            "Chemin non local, ambigu ou contenant une traversée ; conservé.",
        ));
    }
    let root: Vec<u16> = path.to_string_lossy()[..3]
        .encode_utf16()
        .chain(Some(0))
        .collect();
    if unsafe { GetDriveTypeW(root.as_ptr()) } != 3 {
        return Err(crate::i18n::tr(
            "Lecteur non fixe ou réseau ; emplacement conservé.",
        ));
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_attributes() & REPARSE_POINT != 0 => {
                return Err(crate::i18n::tr(
                    "Lien symbolique ou jonction détecté ; emplacement conservé.",
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(crate::i18n::fmt(
                    "Vérification des parents impossible : {error}",
                    &[("error", error.to_string())],
                ))
            }
        }
    }
    Ok(())
}

fn pin_ancestors(path: &Path) -> Result<Vec<fs::File>, String> {
    safe_ancestors(path)?;
    let mut handles = Vec::new();
    for parent in path.parent().into_iter().flat_map(Path::ancestors) {
        let metadata = fs::symlink_metadata(parent).map_err(|error| {
            crate::i18n::fmt(
                "Parent impossible à protéger : {error}",
                &[("error", error.to_string())],
            )
        })?;
        if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err(crate::i18n::tr(
                "Parent devenu un lien ; emplacement conservé.",
            ));
        }
        // BACKUP_SEMANTICS allows opening directories. Excluding SHARE_DELETE
        // prevents an ancestor from being renamed/replaced during shell recycling.
        let handle = OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .open(parent)
            .map_err(|error| {
                crate::i18n::fmt(
                    "Parent impossible à protéger ; emplacement conservé : {error}",
                    &[("error", error.to_string())],
                )
            })?;
        // OPEN_REPARSE_POINT opens a junction itself if it appeared after the
        // lexical checks. Verify the opened object, rather than following it.
        let opened = handle.metadata().map_err(|error| {
            crate::i18n::fmt(
                "Parent ouvert impossible à vérifier ; emplacement conservé : {error}",
                &[("error", error.to_string())],
            )
        })?;
        if !opened.is_dir() || opened.file_attributes() & REPARSE_POINT != 0 {
            return Err(crate::i18n::tr(
                "Parent ouvert devenu un lien ou non-dossier ; emplacement conservé.",
            ));
        }
        handles.push(handle);
    }
    safe_ancestors(path)?;
    Ok(handles)
}

fn preflight_tree(path: &Path) -> Result<(), String> {
    safe_ancestors(path)?;
    let mut pending = vec![(path.to_path_buf(), 0)];
    let mut visited = 0;
    let mut bytes = 0u64;
    while let Some((path, depth)) = pending.pop() {
        visited += 1;
        if visited > MAX_TREE_ENTRIES || depth > MAX_TREE_DEPTH {
            return Err(crate::i18n::tr(
                "Arborescence trop importante pour une vérification complète ; conservée.",
            ));
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && depth == 0 => {
                return Ok(())
            }
            Err(error) => {
                return Err(crate::i18n::fmt(
                    "Arborescence impossible à vérifier : {error}",
                    &[("error", error.to_string())],
                ))
            }
        };
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err(crate::i18n::tr(
                "L’arborescence contient un lien ou une jonction ; conservée.",
            ));
        }
        if metadata.is_dir() {
            let children = fs::read_dir(&path).map_err(|error| {
                crate::i18n::fmt(
                    "Dossier illisible : {error}",
                    &[("error", error.to_string())],
                )
            })?;
            for child in children {
                if pending.len() + visited >= MAX_TREE_ENTRIES {
                    return Err(crate::i18n::tr(
                        "Limite d’inspection de l’arborescence atteinte ; conservée.",
                    ));
                }
                pending.push((
                    child
                        .map_err(|error| {
                            crate::i18n::fmt(
                                "Entrée illisible : {error}",
                                &[("error", error.to_string())],
                            )
                        })?
                        .path(),
                    depth + 1,
                ));
            }
        } else if metadata.is_file() {
            bytes = bytes.saturating_add(metadata.len());
            if bytes > MAX_TREE_BYTES {
                return Err(crate::i18n::tr("Plus de 2 Gio de résidus ; conservés pour éviter une destruction non récupérable."));
            }
        } else {
            return Err(crate::i18n::tr(
                "Type de fichier non pris en charge ; conservé.",
            ));
        }
    }
    Ok(())
}

fn program_binaries(root: &Path) -> Result<Vec<PathBuf>, String> {
    preflight_tree(root)?;
    let mut pending = vec![root.to_path_buf()];
    let mut result = Vec::new();
    let mut visited = 0;
    while let Some(path) = pending.pop() {
        visited += 1;
        if visited > MAX_TREE_ENTRIES {
            return Err(crate::i18n::tr(
                "Limite d’inspection du programme atteinte ; binaires conservés.",
            ));
        }
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            crate::i18n::fmt(
                "Inspection du programme impossible : {error}",
                &[("error", error.to_string())],
            )
        })?;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err(crate::i18n::tr(
                "Lien détecté dans le programme ; binaires conservés.",
            ));
        }
        if metadata.is_dir() {
            for child in fs::read_dir(&path).map_err(|error| {
                crate::i18n::fmt(
                    "Inspection du programme impossible : {error}",
                    &[("error", error.to_string())],
                )
            })? {
                pending.push(
                    child
                        .map_err(|error| {
                            crate::i18n::fmt(
                                "Entrée du programme illisible : {error}",
                                &[("error", error.to_string())],
                            )
                        })?
                        .path(),
                );
                if pending.len() + visited > MAX_TREE_ENTRIES {
                    return Err(crate::i18n::tr(
                        "Limite d’inspection du programme atteinte ; binaires conservés.",
                    ));
                }
            }
        } else if metadata.is_file()
            && path.extension().is_some_and(|extension| {
                ["exe", "dll", "pdb", "manifest"]
                    .iter()
                    .any(|known| extension.eq_ignore_ascii_case(known))
            })
        {
            result.push(path);
            if result.len() >= MAX_CANDIDATES {
                break;
            }
        }
    }
    Ok(result)
}

fn file_item(candidate: &FileCandidate, reason: &str) -> CleanupItem {
    CleanupItem {
        kind: match &candidate.kind {
            FileKind::Program => "programme",
            FileKind::Cache => "cache",
            FileKind::Shortcut(_) => "raccourci",
        }
        .into(),
        location: candidate.path.display().to_string(),
        reason: crate::i18n::tr(reason),
        backup: None,
    }
}

fn registry_item(
    candidate: &RegistryCandidate,
    reason: &str,
    backup: Option<&Path>,
) -> CleanupItem {
    CleanupItem {
        kind: "registre".into(),
        location: format!(
            "{:?}/{:?}\\{} [{}]",
            candidate.key.hive, candidate.key.view, candidate.key.path, candidate.value
        ),
        reason: crate::i18n::tr(reason),
        backup: backup.map(|path| path.display().to_string()),
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

struct RegistryChain(Vec<HKEY>);
impl Drop for RegistryChain {
    fn drop(&mut self) {
        for handle in self.0.iter().rev() {
            unsafe {
                RegCloseKey(*handle);
            }
        }
    }
}

struct OpenKey {
    value: HKEY,
    _ancestors: RegistryChain,
}
impl Drop for OpenKey {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.value);
        }
    }
}

fn open_key(key: &RegistryKey, write: bool) -> Result<Option<OpenKey>, String> {
    let access = if write {
        KEY_QUERY_VALUE | KEY_SET_VALUE
    } else {
        KEY_READ
    };
    let view = match key.view {
        RegistryView::Registry32 => KEY_WOW64_32KEY,
        RegistryView::Registry64 => KEY_WOW64_64KEY,
    };
    let hive = match key.hive {
        Hive::User => HKEY_CURRENT_USER,
        Hive::Machine => HKEY_LOCAL_MACHINE,
    };
    let components: Vec<&str> = key.path.split('\\').collect();
    if components.is_empty()
        || components
            .iter()
            .any(|component| component.is_empty() || component.contains('\0'))
    {
        return Err(crate::i18n::tr(
            "Chemin de registre invalide ; trace conservée.",
        ));
    }
    let mut chain = RegistryChain(Vec::new());
    for (index, component) in components.iter().enumerate() {
        let parent = chain.0.last().copied().unwrap_or(hive);
        let mut handle = null_mut();
        let desired = if index + 1 == components.len() {
            access
        } else {
            KEY_READ
        };
        let result = unsafe {
            RegOpenKeyExW(
                parent,
                wide(component).as_ptr(),
                REG_OPTION_OPEN_LINK,
                desired | view,
                &mut handle,
            )
        };
        match result {
            ERROR_SUCCESS => chain.0.push(handle),
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => return Ok(None),
            _ => {
                return Err(crate::i18n::fmt(
                    "Trace de registre conservée : accès refusé ou lecture impossible ({result}).",
                    &[("result", result.to_string())],
                ))
            }
        }
        // OPEN_LINK applies to the leaf only. Open one component at a time and
        // retain all handles so a linked ancestor never silently redirects a write.
        if read_value(handle, "SymbolicLinkValue")?.is_some() {
            return Err(crate::i18n::tr(
                "Lien de registre détecté dans la chaîne ; trace conservée.",
            ));
        }
    }
    let handle = chain.0.pop().ok_or("Chemin de registre vide.")?;
    Ok(Some(OpenKey {
        value: handle,
        _ancestors: chain,
    }))
}

fn read_value(handle: HKEY, value: &str) -> Result<Option<(u32, Vec<u8>)>, String> {
    let name = wide(value);
    let mut value_type = 0;
    let mut length = 0;
    let status = unsafe {
        RegQueryValueExW(
            handle,
            name.as_ptr(),
            null(),
            &mut value_type,
            null_mut(),
            &mut length,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS || length as usize > MAX_REGISTRY_BYTES {
        return Err(crate::i18n::tr(
            "Valeur de registre illisible ou trop volumineuse ; conservée.",
        ));
    }
    let mut bytes = vec![0; length as usize];
    let status = unsafe {
        RegQueryValueExW(
            handle,
            name.as_ptr(),
            null(),
            &mut value_type,
            bytes.as_mut_ptr(),
            &mut length,
        )
    };
    if status != ERROR_SUCCESS || length as usize > bytes.len() {
        return Err(crate::i18n::tr(
            "Valeur de registre modifiée pendant la lecture ; conservée.",
        ));
    }
    bytes.truncate(length as usize);
    Ok(Some((value_type, bytes)))
}

fn value_text(value_type: u32, bytes: &[u8]) -> Option<String> {
    if !matches!(value_type, REG_SZ | REG_EXPAND_SZ) || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let utf16: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let end = utf16
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(utf16.len());
    if utf16[end..].iter().any(|unit| *unit != 0) {
        return None;
    }
    let text = String::from_utf16(&utf16[..end]).ok()?;
    if value_type == REG_EXPAND_SZ {
        expand_environment(&text)
    } else {
        Some(text)
    }
}

fn expand_environment(text: &str) -> Option<String> {
    let mut result = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find('%') {
        result.push_str(&remaining[..start]);
        let tail = &remaining[start + 1..];
        let end = tail.find('%')?;
        result.push_str(&std::env::var(&tail[..end]).ok()?);
        remaining = &tail[end + 1..];
        if result.len() > MAX_REGISTRY_BYTES {
            return None;
        }
    }
    result.push_str(remaining);
    Some(result)
}

fn executable_path(text: &str, command_line: bool) -> Option<PathBuf> {
    let text = text.trim();
    let path = if let Some(tail) = text.strip_prefix('"') {
        let end = tail.find('"')?;
        if !command_line && !tail[end + 1..].trim().is_empty() {
            return None;
        }
        if command_line
            && !tail[end + 1..].is_empty()
            && !tail[end + 1..].starts_with(char::is_whitespace)
        {
            return None;
        }
        &tail[..end]
    } else {
        // Even an existing unquoted path containing spaces is ambiguous:
        // CreateProcess can choose C:\Program.exe before Program Files\....
        // Never guess which prefix Windows would execute.
        if command_line && text.chars().any(char::is_whitespace) {
            return None;
        }
        text
    };
    let path = PathBuf::from(path);
    (path_components(&path).is_some()
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe")))
    .then_some(path)
}

fn scan_run(key: &RegistryKey, visited: &mut usize) -> Result<Vec<RegistryCandidate>, String> {
    let Some(handle) = open_key(key, false)? else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for index in 0..MAX_REGISTRY_ENTRIES as u32 {
        if *visited >= MAX_REGISTRY_ENTRIES {
            return Err(crate::i18n::tr(
                "Limite d’inspection des démarrages automatiques atteinte.",
            ));
        }
        let mut name = vec![0u16; 16_384];
        let mut length = name.len() as u32;
        let status = unsafe {
            RegEnumValueW(
                handle.value,
                index,
                name.as_mut_ptr(),
                &mut length,
                null(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != ERROR_SUCCESS || length as usize >= name.len() {
            return Err(crate::i18n::tr(
                "Lecture d’un démarrage automatique impossible ; traces conservées.",
            ));
        }
        *visited += 1;
        let value =
            String::from_utf16(&name[..length as usize]).map_err(|_| "Nom de valeur illisible.")?;
        let Some((value_type, bytes)) = read_value(handle.value, &value)? else {
            continue;
        };
        let Some(text) = value_text(value_type, &bytes) else {
            continue;
        };
        let Some(executable) = executable_path(&text, true) else {
            continue;
        };
        result.push(RegistryCandidate {
            key: key.clone(),
            value,
            value_type,
            bytes,
            executable,
        });
    }
    Ok(result)
}

fn scan_app_paths(
    key: &RegistryKey,
    visited: &mut usize,
) -> Result<Vec<RegistryCandidate>, String> {
    let Some(handle) = open_key(key, false)? else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for index in 0..MAX_REGISTRY_ENTRIES as u32 {
        if *visited >= MAX_REGISTRY_ENTRIES {
            return Err(crate::i18n::tr("Limite d’inspection App Paths atteinte."));
        }
        let mut name = [0u16; 1024];
        let mut length = name.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                handle.value,
                index,
                name.as_mut_ptr(),
                &mut length,
                null(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        if status != ERROR_SUCCESS || length as usize >= name.len() {
            return Err(crate::i18n::tr(
                "Lecture d’une trace App Paths impossible ; traces conservées.",
            ));
        }
        *visited += 1;
        let child = String::from_utf16(&name[..length as usize])
            .map_err(|_| crate::i18n::tr("Nom de clé illisible."))?;
        if !child.to_ascii_lowercase().ends_with(".exe") || child.contains(['\\', '/']) {
            continue;
        }
        let key = RegistryKey {
            path: format!("{}\\{child}", key.path),
            ..key.clone()
        };
        let Some(child_handle) = open_key(&key, false)? else {
            continue;
        };
        let Some((value_type, bytes)) = read_value(child_handle.value, "")? else {
            continue;
        };
        let Some(text) = value_text(value_type, &bytes) else {
            continue;
        };
        let Some(executable) = executable_path(&text, false) else {
            continue;
        };
        result.push(RegistryCandidate {
            key,
            value: String::new(),
            value_type,
            bytes,
            executable,
        });
    }
    Ok(result)
}

fn registry_unchanged(candidate: &RegistryCandidate) -> Result<(), String> {
    let Some(handle) = open_key(&candidate.key, false)? else {
        return Err(crate::i18n::tr("Trace déjà absente ; aucune modification."));
    };
    compare_registry_value(handle.value, candidate)
}

fn compare_registry_value(handle: HKEY, candidate: &RegistryCandidate) -> Result<(), String> {
    if read_value(handle, &candidate.value)?
        != Some((candidate.value_type, candidate.bytes.clone()))
    {
        return Err(crate::i18n::tr(
            "La trace a changé depuis l’inspection ; conservée.",
        ));
    }
    Ok(())
}

fn backup_registry(candidate: &RegistryCandidate) -> Result<PathBuf, String> {
    let directory = crate::config::data_dir().join("uninstall-backups");
    safe_ancestors(&directory)?;
    fs::create_dir_all(&directory).map_err(|error| {
        crate::i18n::fmt(
            "Sauvegarde impossible ; trace conservée : {error}",
            &[("error", error.to_string())],
        )
    })?;
    safe_ancestors(&directory)?;
    let _parents = pin_ancestors(&directory.join("backup-placeholder"))?;
    let bytes = serde_json::to_vec_pretty(candidate).map_err(|error| {
        crate::i18n::fmt(
            "Sauvegarde impossible : {error}",
            &[("error", error.to_string())],
        )
    })?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for suffix in 0..16 {
        let path = directory.join(format!("{now}-{}-{suffix}.json", std::process::id()));
        let file = OpenOptions::new().write(true).create_new(true).open(&path);
        match file {
            Ok(mut file) => {
                file.write_all(&bytes)
                    .and_then(|()| file.sync_all())
                    .map_err(|error| {
                        crate::i18n::fmt(
                            "Sauvegarde incomplète ; trace conservée : {error}",
                            &[("error", error.to_string())],
                        )
                    })?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(crate::i18n::fmt(
                    "Sauvegarde impossible ; trace conservée : {error}",
                    &[("error", error.to_string())],
                ))
            }
        }
    }
    Err(crate::i18n::tr(
        "Impossible de créer une sauvegarde unique ; trace conservée.",
    ))
}

fn delete_registry_value(candidate: &RegistryCandidate) -> Result<(), String> {
    let Some(handle) = open_key(&candidate.key, true)? else {
        return Err(crate::i18n::tr("Trace déjà absente ; aucune modification."));
    };
    compare_registry_value(handle.value, candidate)?;
    let status = unsafe { RegDeleteValueW(handle.value, wide(&candidate.value).as_ptr()) };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(crate::i18n::fmt(
            "Trace conservée : Windows a refusé la modification ({status}).",
            &[("status", status.to_string())],
        ))
    }
}

struct ComApartment(bool);
impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

fn shortcut_roots() -> Vec<PathBuf> {
    let mut result: Vec<PathBuf> = Vec::new();
    for folder in [
        FOLDERID_Programs,
        FOLDERID_CommonPrograms,
        FOLDERID_Desktop,
        FOLDERID_PublicDesktop,
    ] {
        let mut raw = null_mut();
        if unsafe { SHGetKnownFolderPath(&folder, KF_FLAG_DEFAULT as u32, null_mut(), &mut raw) }
            < 0
        {
            continue;
        }
        let Some(text) = (unsafe { com::take_co_task_string(raw) }) else {
            continue;
        };
        let path = PathBuf::from(text);
        if safe_ancestors(&path).is_ok() && !result.iter().any(|other| same_path(other, &path)) {
            result.push(path);
        }
    }
    result
}

fn shortcut_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    safe_ancestors(root)?;
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut visited = 0;
    let mut result = Vec::new();
    while let Some((path, depth)) = pending.pop() {
        visited += 1;
        if visited > MAX_TREE_ENTRIES || depth > 16 {
            return Err(crate::i18n::tr(
                "Limite d’inspection des raccourcis atteinte ; autres raccourcis conservés.",
            ));
        }
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            crate::i18n::fmt(
                "Raccourcis non inspectables : {error}",
                &[("error", error.to_string())],
            )
        })?;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            continue;
        }
        if metadata.is_dir() {
            for child in fs::read_dir(&path).map_err(|error| {
                crate::i18n::fmt(
                    "Raccourcis non inspectables : {error}",
                    &[("error", error.to_string())],
                )
            })? {
                if pending.len() + visited >= MAX_TREE_ENTRIES {
                    return Err(crate::i18n::tr(
                        "Limite d’inspection des raccourcis atteinte.",
                    ));
                }
                pending.push((
                    child
                        .map_err(|error| {
                            crate::i18n::fmt(
                                "Raccourci illisible : {error}",
                                &[("error", error.to_string())],
                            )
                        })?
                        .path(),
                    depth + 1,
                ));
            }
        } else if metadata.is_file()
            && metadata.len() <= 1_048_576
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        {
            result.push(path);
        }
    }
    Ok(result)
}

fn shortcut_target(path: &Path) -> Result<PathBuf, String> {
    safe_ancestors(path)?;
    let status = unsafe { CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) };
    if status < 0 {
        return Err(crate::i18n::tr(
            "Lecture COM du raccourci indisponible ; conservé.",
        ));
    }
    let _apartment = ComApartment(true);
    let link: ComPtr<IShellLinkW> =
        unsafe { com::create_instance(&ShellLink, CLSCTX_INPROC_SERVER) }.map_err(|error| {
            crate::i18n::fmt(
                "Raccourci non inspectable : {error}",
                &[("error", error.to_string())],
            )
        })?;
    let persist: ComPtr<IPersistFile> = link.cast().map_err(|error| {
        crate::i18n::fmt(
            "Raccourci non inspectable : {error}",
            &[("error", error.to_string())],
        )
    })?;
    let encoded: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe { persist.Load(encoded.as_ptr(), STGM_READ) }.map_err(|error| {
        crate::i18n::fmt(
            "Raccourci non inspectable : {error}",
            &[("error", error.to_string())],
        )
    })?;
    let mut destination = vec![0u16; 32_768];
    // RAWPATH reads the saved target. Never call Resolve: it can search the
    // machine/network, update the shortcut or display user-facing dialogs.
    unsafe { link.GetPath(&mut destination, null_mut(), SLGP_RAWPATH as u32) }.map_err(
        |error| {
            crate::i18n::fmt(
                "Cible du raccourci non inspectable : {error}",
                &[("error", error.to_string())],
            )
        },
    )?;
    let Some(end) = destination.iter().position(|unit| *unit == 0) else {
        return Err(crate::i18n::tr(
            "Cible du raccourci trop longue ; conservé.",
        ));
    };
    let text =
        String::from_utf16(&destination[..end]).map_err(|_| "Cible du raccourci illisible.")?;
    executable_path(&text, false)
        .ok_or_else(|| crate::i18n::tr("Cible du raccourci non locale ou ambiguë ; conservé."))
}

fn recycle(path: &Path) -> Result<(), String> {
    // Windows 8+ can require recycling explicitly. Unlike a permanent delete
    // fallback, RECYCLEONDELETE returns a failure when Windows cannot recycle.
    let status = unsafe { CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) };
    if status < 0 {
        return Err(crate::i18n::fmt(
            "Corbeille indisponible ; emplacement conservé ({status}).",
            &[("status", format!("{status:#010X}"))],
        ));
    }
    let _apartment = ComApartment(true);
    let operation: ComPtr<IFileOperation> =
        unsafe { com::create_instance(&FileOperation, CLSCTX_INPROC_SERVER) }.map_err(|error| {
            crate::i18n::fmt(
                "Corbeille indisponible ; emplacement conservé : {error}",
                &[("error", error.to_string())],
            )
        })?;
    let flags = FOFX_RECYCLEONDELETE
        | FOFX_EARLYFAILURE
        | FOF_ALLOWUNDO
        | FOF_NOCONFIRMATION
        | FOF_NOERRORUI
        | FOF_SILENT
        | FOF_NO_CONNECTED_ELEMENTS;
    unsafe { operation.SetOperationFlags(flags) }.map_err(|error| {
        crate::i18n::fmt(
            "Corbeille indisponible : {error}",
            &[("error", error.to_string())],
        )
    })?;
    let encoded: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let item: ComPtr<IShellItem> = unsafe { com::item_from_parsing_name(encoded.as_ptr()) }
        .map_err(|error| {
            crate::i18n::fmt(
                "Emplacement conservé : {error}",
                &[("error", error.to_string())],
            )
        })?;
    unsafe { operation.DeleteItem(&item) }
        .and_then(|()| unsafe { operation.PerformOperations() })
        .map_err(|error| {
            crate::i18n::fmt(
                "Déplacement dans la Corbeille refusé : {error}",
                &[("error", error.to_string())],
            )
        })?;
    let aborted = unsafe { operation.GetAnyOperationsAborted() }.map_err(|error| {
        crate::i18n::fmt(
            "État de la Corbeille inconnu : {error}",
            &[("error", error.to_string())],
        )
    })?;
    if aborted != 0 || path.exists() {
        return Err(crate::i18n::tr(
            "Windows a interrompu le déplacement dans la Corbeille ; emplacement conservé.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

    struct Fixture {
        root: PathBuf,
        temp_root: PathBuf,
        root_name: String,
        roots: Roots,
    }

    fn fixture_temp_root(directory: &Path) -> PathBuf {
        // GitHub's Windows TMP can contain RUNNER~1. Normalize that existing
        // alias before exercising the production policy, which rejects unknown
        // short names. A verbatim DOS prefix is unnecessary for these fixtures.
        let canonical = fs::canonicalize(directory).expect("fixture temporary directory exists");
        let text = canonical.to_str().expect("fixture temporary path is UTF-8");
        let path = PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(text));
        assert!(
            path_components(&path).is_some(),
            "fixture temporary directory must have an unambiguous local long path"
        );
        path
    }

    impl Fixture {
        fn new() -> Self {
            Self::in_temp_directory(&std::env::temp_dir())
        }

        fn in_temp_directory(directory: &Path) -> Self {
            let temp_root = fixture_temp_root(directory);
            let root_name = format!(
                "supermenu-cleanup-fixture-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            );
            let root = temp_root.join(&root_name);
            let programs = root.join("Programs");
            let local = root.join("Local");
            let roaming = root.join("Roaming");
            for directory in [&programs, &local, &roaming] {
                fs::create_dir_all(directory).unwrap();
            }
            Self {
                root,
                temp_root,
                root_name,
                roots: Roots {
                    programs: vec![programs],
                    local: Some(local),
                    roaming: Some(roaming),
                },
            }
        }
        fn entry(&self, name: &str, leaf: &str, executable: &str) -> InstalledEntry {
            let directory = self.roots.programs[0].join(leaf);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join(executable), b"fixture executable").unwrap();
            InstalledEntry {
                name: name.into(),
                publisher: "Fixture Publisher".into(),
                install_location: Some(directory.clone()),
                display_icon: Some(directory.join(executable)),
                uninstall_key: RegistryKey {
                    hive: Hive::User,
                    view: RegistryView::Registry64,
                    path: format!("Fixture\\{name}"),
                },
                protected: false,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only this test's freshly created and verified absolute directory is
            // removed. No production cleanup API is invoked by fixture tests.
            // Never panic while unwinding another test failure: it would abort
            // the whole test process before libtest can report the first panic.
            if std::thread::panicking() {
                return;
            }
            // Compare against the exact frozen parent and nonce captured during
            // creation; an environment change cannot widen the deletion scope.
            if self.root.parent() != Some(self.temp_root.as_path())
                || self.root.file_name() != Some(std::ffi::OsStr::new(&self.root_name))
                || !self.root_name.starts_with("supermenu-cleanup-fixture-")
            {
                eprintln!("Invalid fixture cleanup scope; fixture retained.");
                return;
            }
            if let Err(error) = fs::remove_dir_all(&self.root) {
                eprintln!("Own fixture cleanup failed; fixture retained: {error}");
            }
        }
    }

    #[test]
    fn temporary_short_alias_is_normalized_before_fixture_ownership() {
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
        let original = std::env::temp_dir();
        let encoded: Vec<u16> = original.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut output = vec![0u16; 32_768];
        let length = unsafe {
            GetShortPathNameW(encoded.as_ptr(), output.as_mut_ptr(), output.len() as u32)
        };
        assert!(length > 0 && (length as usize) < output.len());
        let alias = PathBuf::from(String::from_utf16(&output[..length as usize]).unwrap());
        let fixture = Fixture::in_temp_directory(&alias);
        assert!(same_path(&fixture.temp_root, &fixture_temp_root(&original)));
        assert!(!fixture.root.to_string_lossy().contains('~'));
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        let plan = fixture_plan(&fixture, entry);
        assert!(
            !plan.files.is_empty(),
            "canonical fixture must exercise positive program ownership"
        );
    }
    fn inventory(entries: Vec<InstalledEntry>) -> Inventory {
        Inventory {
            entries,
            complete: true,
            notes: Vec::new(),
        }
    }
    fn fixture_plan(fixture: &Fixture, entry: InstalledEntry) -> CleanupPlan {
        CleanupPlan::from_inventory(
            &format!("name:{}", entry.name),
            inventory(vec![entry]),
            fixture.roots.clone(),
        )
    }

    #[test]
    fn local_components_reject_traversal_devices_and_aliases() {
        for path in [
            r"C:\Programs\..\Documents",
            r"C:\Programs\.\App",
            r"\\server\share\App",
            r"\\?\C:\App",
            r"C:App",
            r"C:\Programs\App:stream",
            r"C:\Programs\App.",
            r"C:\Programs\NUL",
            r"C:\Programs\*.exe",
        ] {
            assert!(path_components(Path::new(path)).is_none(), "{path}");
        }
        assert!(is_within(
            Path::new(r"C:\Program Files\App\bin"),
            Path::new(r"c:\program files\APP")
        ));
        assert!(!is_within(
            Path::new(r"C:\Program Files\AppOther"),
            Path::new(r"C:\Program Files\App")
        ));
    }

    #[test]
    fn preview_does_not_remove_or_write_any_fixture() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        let directory = entry.install_location.clone().unwrap();
        let plan = fixture_plan(&fixture, entry);
        assert!(!plan.files.is_empty());
        let report = plan.preview_report();
        assert!(report.inspection_only);
        assert!(report.removed.is_empty());
        assert!(directory.join("fixture.exe").exists());
    }

    #[test]
    fn dated_streamfab_entry_identifies_only_exact_program_binaries() {
        let fixture = Fixture::new();
        let entry = fixture.entry(
            "StreamFab (x64) (23/07/2026)",
            r"StreamFab\StreamFab",
            "StreamFab64.exe",
        );
        let directory = entry.install_location.clone().unwrap();
        let binaries = [
            directory.join("StreamFab64.exe"),
            directory.join("Qt6Core.dll"),
            directory.join("StreamFab64.pdb"),
            directory.join("StreamFab64.exe.manifest"),
        ];
        for binary in &binaries[1..] {
            fs::write(binary, b"fixture program binary").unwrap();
        }
        let personal = [
            directory.join("settings.ini"),
            directory.join(r"Profiles\Default\profile.sqlite"),
            directory.join(r"Downloads\video.mp4"),
            fixture
                .roots
                .local
                .as_ref()
                .unwrap()
                .join(r"StreamFab\StreamFab\settings.json"),
            fixture
                .roots
                .roaming
                .as_ref()
                .unwrap()
                .join(r"StreamFab\StreamFab\profile.sqlite"),
            fixture.root.join(r"Downloads\video.mp4"),
        ];
        for path in &personal {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"personal fixture data").unwrap();
        }
        let plan = fixture_plan(&fixture, entry);
        assert_eq!(plan.program_roots.len(), 1);
        assert!(same_path(&plan.program_roots[0], &directory));
        assert_eq!(plan.files.len(), binaries.len());
        for binary in &binaries {
            assert!(plan.files.iter().any(|candidate| {
                matches!(candidate.kind, FileKind::Program) && same_path(&candidate.path, binary)
            }));
        }
        for path in &personal {
            assert!(!plan
                .files
                .iter()
                .any(|candidate| same_path(&candidate.path, path)));
            assert_eq!(fs::read(path).unwrap(), b"personal fixture data");
        }
        let report = plan.preview_report();
        assert!(report.inspection_only);
        assert!(report.removed.is_empty());
        assert!(binaries.iter().all(|path| path.exists()));
    }

    #[test]
    fn streamfab_path_exception_keeps_metadata_and_shared_owner_guards() {
        let fixture = Fixture::new();
        let entry = fixture.entry(
            "StreamFab (x64) (23/07/2026)",
            r"StreamFab\StreamFab",
            "StreamFab64.exe",
        );
        let directory = entry.install_location.clone().unwrap();
        let other = fixture.entry("Other App", "Other App", "other.exe");
        let mut wrong_executable = entry.clone();
        let wrong_icon = directory.join("other.exe");
        fs::write(&wrong_icon, b"different fixture executable").unwrap();
        wrong_executable.display_icon = Some(wrong_icon);
        let parent = fixture.entry(&entry.name, "StreamFab", "StreamFab64.exe");
        let mut missing_icon = entry.clone();
        missing_icon.display_icon = None;
        let mut external_icon = entry.clone();
        external_icon.display_icon = other.display_icon.clone();
        let mut protected = entry.clone();
        protected.protected = true;
        for (case, candidate) in [
            ("different executable", wrong_executable),
            ("publisher parent folder", parent),
            ("absent executable metadata", missing_icon),
            ("external executable metadata", external_icon),
            ("protected target", protected),
        ] {
            let plan = fixture_plan(&fixture, candidate);
            assert!(plan.files.is_empty(), "{case}");
            assert!(plan.program_roots.is_empty(), "{case}");
        }
        for protected_other in [false, true] {
            let mut shared = other.clone();
            shared.install_location = entry.install_location.clone();
            shared.protected = protected_other;
            let plan = CleanupPlan::from_inventory(
                &format!("name:{}", entry.name),
                inventory(vec![entry.clone(), shared]),
                fixture.roots.clone(),
            );
            assert!(plan.files.is_empty());
            assert!(plan.program_roots.is_empty());
            assert!(plan
                .retained
                .iter()
                .any(|item| item.reason.contains("partagé")));
        }
        assert!(directory.join("StreamFab64.exe").exists());
        assert!(other.display_icon.unwrap().exists());
    }

    #[test]
    fn incomplete_or_still_installed_inventory_prevents_every_mutation() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        let plan = fixture_plan(&fixture, entry.clone());
        for current in [
            inventory(vec![entry]),
            Inventory {
                entries: Vec::new(),
                complete: false,
                notes: Vec::new(),
            },
        ] {
            let report = plan.execute_with(|| current.clone(), |_| panic!("must never mutate"));
            assert!(report.removed.is_empty());
            assert!(!report.retained.is_empty());
        }
    }

    #[test]
    fn registry_view_duplicates_are_one_product_but_distinct_keys_are_ambiguous() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        let mut duplicate = entry.clone();
        duplicate.uninstall_key.view = RegistryView::Registry32;
        assert!(select_entry(
            &inventory(vec![entry.clone(), duplicate.clone()]),
            "name:Fixture App"
        )
        .1
        .is_none());
        duplicate.uninstall_key.path.push_str("-other");
        assert!(
            select_entry(&inventory(vec![entry, duplicate]), "name:Fixture App")
                .1
                .is_some()
        );
    }

    #[test]
    fn protected_target_and_shared_program_directories_are_retained() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        let mut protected = entry.clone();
        protected.protected = true;
        assert!(fixture_plan(&fixture, protected).veto.is_some());
        let mut other = fixture.entry("Other App", "Other App", "other.exe");
        other.install_location = entry.install_location.clone();
        let plan = CleanupPlan::from_inventory(
            "name:Fixture App",
            inventory(vec![entry, other]),
            fixture.roots.clone(),
        );
        assert!(plan.files.is_empty());
        assert!(!plan.retained.is_empty());
    }

    #[test]
    fn newly_installed_owner_prevents_recycling_after_capture() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        let plan = fixture_plan(&fixture, entry.clone());
        let mut other = fixture.entry("Other App", "Other App", "other.exe");
        other.install_location = entry.install_location;
        let report = plan.execute_with(
            || inventory(vec![other.clone()]),
            |_| panic!("shared folder must be retained"),
        );
        assert!(report.removed.is_empty());
        assert!(report
            .retained
            .iter()
            .any(|item| item.reason.contains("partagé")));
    }

    #[test]
    fn only_named_caches_move_and_personal_browser_files_remain() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Google Chrome", r"Google\Chrome\Application", "chrome.exe");
        let profile = fixture
            .roots
            .local
            .as_ref()
            .unwrap()
            .join(r"Google\Chrome\User Data\Default");
        fs::create_dir_all(profile.join("Cache")).unwrap();
        fs::write(profile.join(r"Cache\temporary.bin"), b"cache").unwrap();
        for name in [
            "Bookmarks",
            "Preferences",
            "History",
            "Login Data",
            "Sessions",
        ] {
            fs::write(profile.join(name), b"personal").unwrap();
        }
        let plan = CleanupPlan::from_inventory(
            "id:Google.Chrome",
            inventory(vec![entry]),
            fixture.roots.clone(),
        );
        let bin = fixture.root.join("FixtureBin");
        fs::create_dir(&bin).unwrap();
        let mut number = 0;
        let report = plan.execute_with(
            || inventory(Vec::new()),
            |path| {
                number += 1;
                fs::rename(path, bin.join(number.to_string())).map_err(|error| error.to_string())
            },
        );
        assert!(report.removed.iter().any(|item| item.kind == "cache"));
        assert!(!profile.join("Cache").exists());
        for name in [
            "Bookmarks",
            "Preferences",
            "History",
            "Login Data",
            "Sessions",
        ] {
            assert_eq!(fs::read(profile.join(name)).unwrap(), b"personal");
        }
        assert!(profile.exists());
    }

    #[test]
    fn another_browser_channel_preserves_caches() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Google Chrome", r"Google\Chrome\Application", "chrome.exe");
        let other = fixture.entry(
            "Google Chrome Beta",
            r"Google\Chrome Beta\Application",
            "chrome.exe",
        );
        let profile = fixture
            .roots
            .local
            .as_ref()
            .unwrap()
            .join(r"Google\Chrome\User Data\Default\Cache");
        fs::create_dir_all(&profile).unwrap();
        let plan = CleanupPlan::from_inventory(
            "id:Google.Chrome",
            inventory(vec![entry, other]),
            fixture.roots.clone(),
        );
        assert!(!plan
            .files
            .iter()
            .any(|candidate| matches!(candidate.kind, FileKind::Cache)));
        assert!(profile.exists());
    }

    #[test]
    fn generic_appdata_and_publisher_roots_are_not_program_folders() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        for directory in [
            fixture.roots.programs[0].clone(),
            fixture.roots.local.as_ref().unwrap().join("Fixture App"),
            fixture.roots.programs[0].join("Fixture Publisher"),
        ] {
            fs::create_dir_all(&directory).unwrap();
            assert!(approved_program_path(&directory, &entry, &fixture.roots).is_err());
        }
    }

    #[test]
    fn locked_or_unrecyclable_files_have_no_permanent_delete_fallback() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        let path = entry.install_location.clone().unwrap();
        let plan = fixture_plan(&fixture, entry);
        let report = plan.execute_with(
            || inventory(Vec::new()),
            |_| Err("Corbeille refusée".into()),
        );
        assert!(report.removed.is_empty());
        assert!(path.join("fixture.exe").exists());
        assert!(report
            .retained
            .iter()
            .any(|item| item.reason == "Corbeille refusée"));
    }

    #[test]
    fn registry_command_parsing_never_guesses_unquoted_arguments() {
        assert_eq!(
            executable_path(r#""C:\Program Files\App\app.exe" --start"#, true).unwrap(),
            PathBuf::from(r"C:\Program Files\App\app.exe")
        );
        assert!(executable_path(r"C:\Program Files\App\app.exe --start", true).is_none());
        assert!(executable_path(r"C:\Program Files\App\app.exe --other.exe", true).is_none());
        assert!(executable_path(r"C:\Program Files\App\app.exe", true).is_none());
        assert!(executable_path(r#""C:\App\app.exe"argument"#, true).is_none());
        assert!(executable_path(r#""C:\App\app.exe" --start"#, false).is_none());
        assert!(executable_path(r"rundll32.exe C:\App\module.dll", true).is_none());
    }

    #[test]
    fn linked_descendant_is_rejected_without_following_it() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        let directory = entry.install_location.unwrap();
        let outside = fixture.root.join("PersonalOutside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("save.txt"), b"personal").unwrap();
        // Creating Windows symbolic links needs Developer Mode or a privilege.
        // If unavailable, the pure lexical guards remain covered independently.
        if std::os::windows::fs::symlink_dir(&outside, directory.join("linked")).is_ok() {
            assert!(preflight_tree(&directory).is_err());
            assert!(safe_ancestors(&directory.join(r"linked\save.txt")).is_err());
            fs::remove_dir(directory.join("linked")).unwrap();
        }
        assert_eq!(fs::read(outside.join("save.txt")).unwrap(), b"personal");
    }

    #[test]
    fn settings_and_saves_beside_program_binaries_are_preserved() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        let directory = entry.install_location.clone().unwrap();
        fs::write(directory.join("settings.json"), b"personal settings").unwrap();
        fs::create_dir(directory.join("Saves")).unwrap();
        fs::write(directory.join(r"Saves\game.dat"), b"personal saved game").unwrap();
        let plan = fixture_plan(&fixture, entry);
        let bin = fixture.root.join("FixtureBin");
        fs::create_dir(&bin).unwrap();
        let report = plan.execute_with(
            || inventory(Vec::new()),
            |path| {
                fs::rename(path, bin.join(path.file_name().unwrap()))
                    .map_err(|error| error.to_string())
            },
        );
        assert_eq!(report.removed.len(), 1);
        assert!(directory.exists());
        assert_eq!(
            fs::read(directory.join("settings.json")).unwrap(),
            b"personal settings"
        );
        assert_eq!(
            fs::read(directory.join(r"Saves\game.dat")).unwrap(),
            b"personal saved game"
        );
    }

    #[test]
    fn changed_program_binary_is_retained() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        let path = entry.display_icon.clone().unwrap();
        let plan = fixture_plan(&fixture, entry);
        fs::write(&path, b"changed application file or personal replacement").unwrap();
        let report = plan.execute_with(
            || inventory(Vec::new()),
            |_| panic!("changed binary must stay"),
        );
        assert!(report.removed.is_empty());
        assert!(report
            .retained
            .iter()
            .any(|item| item.reason.contains("changé")));
    }

    fn write_fixture_shortcut(path: &Path, target: &Path) {
        let status = unsafe { CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) };
        assert!(status >= 0);
        let _apartment = ComApartment(true);
        let link: ComPtr<IShellLinkW> =
            unsafe { com::create_instance(&ShellLink, CLSCTX_INPROC_SERVER) }.unwrap();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe { link.SetPath(target.as_ptr()) }.unwrap();
        let persist: ComPtr<IPersistFile> = link.cast().unwrap();
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe { persist.Save(path.as_ptr(), true) }.unwrap();
    }

    #[test]
    fn shortcut_target_is_read_without_execution_and_survives_missing_executable() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        let executable = entry.display_icon.clone().unwrap();
        let shortcut = fixture.root.join("Unrelated title.lnk");
        write_fixture_shortcut(&shortcut, &executable);
        assert!(same_path(&shortcut_target(&shortcut).unwrap(), &executable));
        fs::rename(&executable, executable.with_extension("fixture-moved")).unwrap();
        assert!(same_path(&shortcut_target(&shortcut).unwrap(), &executable));
        let plan = fixture_plan(&fixture, entry);
        let candidate = FileCandidate {
            path: shortcut,
            scope: fixture.root.clone(),
            kind: FileKind::Shortcut(executable),
            fingerprint: None,
        };
        assert!(plan.owned_file(&candidate, &inventory(Vec::new())).is_ok());
    }

    #[test]
    fn shortcut_pointing_to_other_program_or_updated_target_is_retained() {
        let fixture = Fixture::new();
        let entry = fixture.entry("Fixture App", "Fixture App", "fixture.exe");
        let executable = entry.display_icon.clone().unwrap();
        let other = fixture.entry("Other App", "Other App", "other.exe");
        let other_executable = other.display_icon.clone().unwrap();
        let plan = fixture_plan(&fixture, entry);
        let shortcut = fixture.root.join("Fixture App misleading title.lnk");
        write_fixture_shortcut(&shortcut, &other_executable);
        assert!(plan
            .owns_executable(&shortcut_target(&shortcut).unwrap(), &inventory(Vec::new()))
            .is_err());
        write_fixture_shortcut(&shortcut, &executable);
        let candidate = FileCandidate {
            path: shortcut.clone(),
            scope: fixture.root.clone(),
            kind: FileKind::Shortcut(executable),
            fingerprint: None,
        };
        assert!(plan.owned_file(&candidate, &inventory(Vec::new())).is_ok());
        write_fixture_shortcut(&shortcut, &other_executable);
        assert!(plan.owned_file(&candidate, &inventory(Vec::new())).is_err());
    }

    #[test]
    fn native_recycling_keeps_a_single_fixture_file_recoverable() {
        let fixture = Fixture::new();
        let path = fixture.root.join("recycle-proof-fixture.bin");
        let content = format!("Recoverable Super Menu fixture {}", fixture.root.display());
        fs::write(&path, content.as_bytes()).unwrap();
        assert!(is_within(&path, &fixture.root));
        assert!(safe_ancestors(&path).is_ok());
        let _parents = pin_ancestors(&path).unwrap();
        recycle(&path).unwrap();
        assert!(!path.exists());
        assert!(
            fixture_is_recoverable(&path, content.as_bytes()),
            "the exact fixture content must remain recoverable in the Windows Recycle Bin"
        );
        // Leave this one tiny test file in the Corbeille. Never empty or modify
        // anybody else's recycled files as part of a verification.
    }

    fn fixture_is_recoverable(path: &Path, content: &[u8]) -> bool {
        let original_path: Vec<u8> = path
            .as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect();
        let drive = path.to_string_lossy()[..3].to_string();
        let recycle_root = PathBuf::from(drive).join("$Recycle.Bin");
        let mut recoverable = false;
        for owner in fs::read_dir(&recycle_root).unwrap().flatten() {
            let Ok(items) = fs::read_dir(owner.path()) else {
                continue;
            };
            for item in items.flatten() {
                let name = item.file_name().to_string_lossy().to_string();
                if !name.starts_with("$I") {
                    continue;
                }
                let Ok(metadata) = item.metadata() else {
                    continue;
                };
                if metadata.len() > 131_072 {
                    continue;
                }
                let Ok(record) = fs::read(item.path()) else {
                    continue;
                };
                if !record
                    .windows(original_path.len())
                    .any(|window| window == original_path)
                {
                    continue;
                }
                let restored_data = owner.path().join(format!("$R{}", &name[2..]));
                if fs::read(restored_data).is_ok_and(|bytes| bytes == content) {
                    recoverable = true;
                }
            }
        }
        recoverable
    }

    #[test]
    fn pinned_parent_handles_are_directories_and_refuse_links() {
        let fixture = Fixture::new();
        let path = fixture.root.join("pinning-fixture.txt");
        fs::write(&path, b"fixture").unwrap();
        let parents = pin_ancestors(&path).unwrap();
        assert!(!parents.is_empty());
        for handle in parents {
            let metadata = handle.metadata().unwrap();
            assert!(metadata.is_dir());
            assert_eq!(metadata.file_attributes() & REPARSE_POINT, 0);
        }
        let outside = fixture.root.join("FixtureOutside");
        fs::create_dir(&outside).unwrap();
        let link = fixture.root.join("FixtureLink");
        if std::os::windows::fs::symlink_dir(&outside, &link).is_ok() {
            assert!(pin_ancestors(&link.join("fixture.txt")).is_err());
            fs::remove_dir(&link).unwrap();
        }
    }

    #[test]
    fn native_cleanup_after_own_fixture_uninstall_recycles_and_backs_up_its_trace() {
        use windows_sys::Win32::System::Registry::{
            RegCreateKeyExW, RegDeleteKeyExW, RegSetValueExW, REG_CREATED_NEW_KEY,
        };
        const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
        // Serialize with other desktop integration tests. Nothing below launches
        // an uninstaller: this fixture is the only registry product we remove.
        let _lock = crate::native::UI_TEST_LOCK.lock().unwrap();
        let fixture = Fixture::new();
        let nonce = format!(
            "SuperMenuCleanupFixture-{:x}-{:x}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let name = format!("SM Cleanup Fixture {nonce}");
        let entry = fixture.entry(&name, &name, "fixture.exe");
        let executable = entry.display_icon.unwrap();
        let directory = entry.install_location.unwrap();
        let settings = directory.join("settings.json");
        fs::write(&settings, b"personal fixture settings").unwrap();
        let binary = fs::read(&executable).unwrap();
        assert!(is_within(&executable, &fixture.root));
        let uninstall_path = format!(r"{UNINSTALL}\{nonce}");

        struct OwnRegistryFixture {
            path: String,
            value: String,
        }
        impl OwnRegistryFixture {
            fn remove_product(&self) {
                assert!(self
                    .path
                    .starts_with(&format!(r"{UNINSTALL}\SuperMenuCleanupFixture-")));
                let status = unsafe {
                    RegDeleteKeyExW(
                        HKEY_CURRENT_USER,
                        wide(&self.path).as_ptr(),
                        KEY_WOW64_64KEY,
                        0,
                    )
                };
                assert!(matches!(
                    status,
                    ERROR_SUCCESS | ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND
                ));
            }
        }
        impl Drop for OwnRegistryFixture {
            fn drop(&mut self) {
                assert!(self
                    .path
                    .starts_with(&format!(r"{UNINSTALL}\SuperMenuCleanupFixture-")));
                assert!(self.value.starts_with("SuperMenuCleanupFixture-"));
                unsafe {
                    RegDeleteKeyExW(
                        HKEY_CURRENT_USER,
                        wide(&self.path).as_ptr(),
                        KEY_WOW64_64KEY,
                        0,
                    );
                }
                let key = RegistryKey {
                    hive: Hive::User,
                    view: RegistryView::Registry64,
                    path: RUN.into(),
                };
                if let Ok(Some(handle)) = open_key(&key, true) {
                    unsafe {
                        RegDeleteValueW(handle.value, wide(&self.value).as_ptr());
                    }
                }
            }
        }
        fn create_own_key(path: &str) -> OpenKey {
            let mut handle = null_mut();
            let mut disposition = 0;
            let status = unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    wide(path).as_ptr(),
                    0,
                    null(),
                    0,
                    KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY,
                    null(),
                    &mut handle,
                    &mut disposition,
                )
            };
            assert_eq!(status, ERROR_SUCCESS);
            let handle = OpenKey {
                value: handle,
                _ancestors: RegistryChain(Vec::new()),
            };
            assert_eq!(
                disposition, REG_CREATED_NEW_KEY,
                "a fixture must never replace an existing product key"
            );
            handle
        }
        fn set_own_string(handle: HKEY, name: &str, value: &str) {
            let bytes: Vec<u8> = wide(value).into_iter().flat_map(u16::to_le_bytes).collect();
            let status = unsafe {
                RegSetValueExW(
                    handle,
                    wide(name).as_ptr(),
                    0,
                    REG_SZ,
                    bytes.as_ptr(),
                    bytes.len() as u32,
                )
            };
            assert_eq!(status, ERROR_SUCCESS);
        }
        fn ensure_fixture_run_key(parent: &RegistryKey) -> OpenKey {
            assert_eq!(parent.hive, Hive::User);
            assert_eq!(parent.view, RegistryView::Registry64);
            let child = RegistryKey {
                path: format!(r"{}\Run", parent.path),
                ..parent.clone()
            };
            if let Some(handle) = open_key(&child, true).unwrap() {
                return handle;
            }
            // A clean runner profile may not have a Run key. Keep the verified
            // non-link parent open and create only its literal Run child. Windows
            // checks parent key creation permission against its security descriptor.
            let parent = open_key(parent, false)
                .unwrap()
                .expect("verified fixture parent exists");
            let mut handle = null_mut();
            let status = unsafe {
                RegCreateKeyExW(
                    parent.value,
                    wide("Run").as_ptr(),
                    0,
                    null(),
                    0,
                    KEY_READ | KEY_SET_VALUE | KEY_WOW64_64KEY,
                    null(),
                    &mut handle,
                    null_mut(),
                )
            };
            assert_eq!(status, ERROR_SUCCESS);
            let created = OpenKey {
                value: handle,
                _ancestors: RegistryChain(Vec::new()),
            };
            drop(created);
            // Reopen through the same component-by-component link checks before
            // writing even the unique fixture value. Existing values are untouched.
            open_key(&child, true)
                .unwrap()
                .expect("verified Run key now exists")
        }
        let key = create_own_key(&uninstall_path);
        let guard = OwnRegistryFixture {
            path: uninstall_path,
            value: nonce.clone(),
        };
        set_own_string(key.value, "DisplayName", &name);
        set_own_string(key.value, "Publisher", "Super Menu Test Fixture");
        set_own_string(
            key.value,
            "InstallLocation",
            &directory.display().to_string(),
        );
        set_own_string(key.value, "DisplayIcon", &executable.display().to_string());
        drop(key);
        // Exercise the missing-child branch on our own disposable product key,
        // never by deleting or changing anybody's actual shared Run key.
        let own_parent = RegistryKey {
            hive: Hive::User,
            view: RegistryView::Registry64,
            path: guard.path.clone(),
        };
        let temporary_run = ensure_fixture_run_key(&own_parent);
        assert!(read_value(temporary_run.value, &nonce).unwrap().is_none());
        drop(temporary_run);
        let temporary_run_path = format!(r"{}\Run", guard.path);
        assert!(temporary_run_path.starts_with(&format!(r"{UNINSTALL}\SuperMenuCleanupFixture-")));
        assert_eq!(
            unsafe {
                RegDeleteKeyExW(
                    HKEY_CURRENT_USER,
                    wide(&temporary_run_path).as_ptr(),
                    KEY_WOW64_64KEY,
                    0,
                )
            },
            ERROR_SUCCESS
        );
        let run_key = RegistryKey {
            hive: Hive::User,
            view: RegistryView::Registry64,
            path: RUN.into(),
        };
        let run_parent = RegistryKey {
            path: RUN.rsplit_once('\\').unwrap().0.into(),
            ..run_key.clone()
        };
        let run = ensure_fixture_run_key(&run_parent);
        assert!(read_value(run.value, &nonce).unwrap().is_none());
        set_own_string(
            run.value,
            &nonce,
            &format!("\"{}\" --fixture-only", executable.display()),
        );
        drop(run);

        let before = uninstall_inventory::capture();
        let target = format!("name:{name}");
        let mut plan = CleanupPlan::from_inventory(&target, before.clone(), fixture.roots.clone());
        if !before.complete {
            // An unfamiliar CI runner may contain unsupported product metadata.
            // The production safety gate must win even for our test fixture.
            guard.remove_product();
            let report = plan.execute_after_success();
            assert!(report.removed.is_empty());
            assert!(executable.exists());
            assert!(settings.exists());
            return;
        }
        assert!(
            plan.veto.is_none(),
            "unique fixture product must be unambiguous"
        );
        plan.capture_registry();
        assert!(plan
            .registry
            .iter()
            .any(|candidate| candidate.value == nonce && candidate.key.hive == Hive::User));
        guard.remove_product();
        let report = plan.execute_after_success();
        assert!(
            !executable.exists(),
            "fixture binary should be recycled: {:?}",
            report.retained
        );
        assert!(fixture_is_recoverable(&executable, &binary));
        assert_eq!(fs::read(settings).unwrap(), b"personal fixture settings");
        let run = open_key(&run_key, false).unwrap().unwrap();
        assert!(read_value(run.value, &nonce).unwrap().is_none());
        let trace = report
            .removed
            .iter()
            .find(|item| item.kind == "registre" && item.location.contains(&nonce))
            .unwrap();
        let backup = trace.backup.as_ref().unwrap();
        let backup: serde_json::Value = serde_json::from_slice(&fs::read(backup).unwrap()).unwrap();
        assert_eq!(backup["value"], nonce);
        assert_eq!(backup["key"]["path"], RUN);
        assert_eq!(backup["value_type"], REG_SZ);
    }
}
