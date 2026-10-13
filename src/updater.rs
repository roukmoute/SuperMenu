//! Daily background checks; the UI authorizes a restart only after a second idle check.
//! The copied helper waits for the complete old process exit before replacing anything.

use crate::{config, http, native::wide, process_tree};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::ptr::null_mut;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, HWND, WAIT_OBJECT_0},
    Storage::FileSystem::{
        MoveFileExW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_READ, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH,
    },
    System::{Registry::*, Threading::*},
    UI::WindowsAndMessaging::{GetClassNameW, GetWindowThreadProcessId, PostMessageW, WM_APP},
};

pub const UPDATE_READY: u32 = WM_APP + 13;
pub const UPDATE_COMMIT: u32 = WM_APP + 14;
pub const UPDATE_ABORTED: u32 = WM_APP + 15;
pub const CHECK_TIMER: usize = 101;
pub const APPLY_TIMER: usize = 102;
const OPTION: &str = "--apply-update";
const API: &str = "https://api.github.com/repos/roukmoute/SuperMenu/releases?per_page=20";
const INSTALLER: &str = "SuperMenu-Setup-x64.exe";
const PORTABLE: &str = "supermenu.exe";
const MAX_METADATA: usize = 1_048_576;
const MAX_PAYLOAD: usize = 8_388_608;
const CHECK_INTERVAL: u64 = 24 * 60 * 60;
const ARP: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\SuperMenu";
const DEFERRED: &str = "Mise à jour reportée : Super Menu est encore utilisé.";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(test, derive(Debug))]
struct Version([u32; 3]);

impl Version {
    fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let mut values = [0; 3];
        for value in &mut values {
            let part = parts.next()?;
            if part.is_empty()
                || (part.len() > 1 && part.starts_with('0'))
                || !part.bytes().all(|byte| byte.is_ascii_digit())
            {
                return None;
            }
            *value = part.parse().ok()?;
        }
        parts.next().is_none().then_some(Self(values))
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    digest: Option<String>,
    browser_download_url: String,
    state: String,
}

fn select_release(
    bytes: &[u8],
    current: Version,
    previews: bool,
) -> Result<Option<Release>, String> {
    if bytes.len() > MAX_METADATA {
        return Err("Réponse de mise à jour trop volumineuse.".into());
    }
    let releases: Vec<Release> =
        serde_json::from_slice(bytes).map_err(|_| "Réponse de mise à jour invalide.")?;
    if releases.len() > 100 {
        return Err("Réponse de mise à jour trop volumineuse.".into());
    }
    Ok(releases
        .into_iter()
        .filter_map(|release| {
            let version = Version::parse(release.tag_name.strip_prefix('v')?)?;
            (!release.draft && (previews || !release.prerelease) && version > current)
                .then_some((version, release))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, release)| release))
}

fn select_stable_release(bytes: &[u8], current: Version) -> Result<Option<Release>, String> {
    if bytes.len() > MAX_METADATA {
        return Err("Réponse de mise à jour trop volumineuse.".into());
    }
    let release: Release =
        serde_json::from_slice(bytes).map_err(|_| "Réponse de mise à jour invalide.")?;
    let newer = release
        .tag_name
        .strip_prefix('v')
        .and_then(Version::parse)
        .is_some_and(|version| version > current);
    Ok((!release.draft && !release.prerelease && newer).then_some(release))
}

fn digest(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut hash = [0; 32];
    for (value, pair) in hash.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
        let digit = |byte: u8| (byte as char).to_digit(16).map(|value| value as u8);
        *value = digit(pair[0])? * 16 + digit(pair[1])?;
    }
    Some(hash)
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 15) as usize] as char);
    }
    text
}

fn asset(release: &Release, installed: bool) -> Result<(&Asset, &str), String> {
    let name = if installed { INSTALLER } else { PORTABLE };
    let mut matching = release.assets.iter().filter(|asset| asset.name == name);
    let asset = matching
        .next()
        .ok_or("Cette version ne propose pas de mise à jour compatible.")?;
    let hash = asset
        .digest
        .as_deref()
        .and_then(|text| text.strip_prefix("sha256:"))
        .filter(|text| digest(text).is_some());
    let expected_url = format!(
        "https://github.com/roukmoute/SuperMenu/releases/download/{}/{name}",
        release.tag_name
    );
    if matching.next().is_some()
        || asset.state != "uploaded"
        || asset.size < 64
        || asset.size > MAX_PAYLOAD as u64
        || hash.is_none()
        || asset.browser_download_url != expected_url
    {
        return Err("Le téléchargement de mise à jour n'est pas vérifiable.".into());
    }
    Ok((asset, hash.unwrap()))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn due(last: u64, now: u64) -> bool {
    last == 0 || last > now || now - last >= CHECK_INTERVAL
}

fn file_hash(file: &mut File) -> io::Result<[u8; 32]> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().into())
}

fn locked_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
}

fn verified_payload(path: &Path, size: u64, hash: &str, installed: bool) -> Result<File, String> {
    let mut file = locked_file(path).map_err(error)?;
    if size < 64
        || size > MAX_PAYLOAD as u64
        || file.metadata().map_err(error)?.len() != size
        || digest(hash) != Some(file_hash(&mut file).map_err(error)?)
    {
        return Err("L'intégrité de la mise à jour n'est pas vérifiée.".into());
    }
    let mut header = [0u8; 64];
    file.seek(SeekFrom::Start(0)).map_err(error)?;
    file.read_exact(&mut header).map_err(error)?;
    let offset = u32::from_le_bytes(header[60..64].try_into().unwrap()) as u64;
    if &header[..2] != b"MZ" || offset < 64 || offset > size.saturating_sub(6) {
        return Err("Le fichier de mise à jour n'est pas un exécutable Windows x64.".into());
    }
    file.seek(SeekFrom::Start(offset)).map_err(error)?;
    let mut pe = [0; 6];
    file.read_exact(&mut pe).map_err(error)?;
    // NSIS runs as an x86 bootstrapper containing our x64 payload; the portable
    // executable itself must remain AMD64. Both require the official asset hash.
    if pe[..4] != [b'P', b'E', 0, 0]
        || !(pe[4..] == [0x64, 0x86] || (installed && pe[4..] == [0x4c, 0x01]))
    {
        return Err("Le fichier de mise à jour n'est pas un exécutable Windows x64.".into());
    }
    Ok(file)
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn same_path(first: &Path, second: &Path) -> bool {
    fs::canonicalize(first)
        .ok()
        .zip(fs::canonicalize(second).ok())
        .is_some_and(|(first, second)| first == second)
}

fn registry_text(name: &str) -> Option<String> {
    let mut text = vec![0u16; 32768];
    let mut size = (text.len() * 2) as u32;
    if unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(ARP).as_ptr(),
            wide(name).as_ptr(),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            null_mut(),
            text.as_mut_ptr().cast(),
            &mut size,
        )
    } != 0
        || size < 2
        || size as usize > text.len() * 2
        || !size.is_multiple_of(2)
    {
        return None;
    }
    text.truncate(size as usize / 2 - 1);
    String::from_utf16(&text).ok()
}

fn installed_at(target: &Path) -> bool {
    let Some(directory) = registry_text("InstallLocation").map(PathBuf::from) else {
        return false;
    };
    let uninstaller = directory.join("uninstall.exe");
    same_path(target, &directory.join(PORTABLE))
        && uninstaller.is_file()
        && registry_text("UninstallString")
            .is_some_and(|value| value == format!("\"{}\"", uninstaller.display()))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    version: String,
    target: PathBuf,
    original_sha256: String,
    payload_sha256: String,
    payload_bytes: u64,
    installed: bool,
    parent_pid: u32,
    parent_created: u64,
    parent_hwnd: usize,
    request: usize,
}

struct Stage {
    directory: PathBuf,
    handed_off: AtomicBool,
    job: Job,
}

impl Drop for Stage {
    fn drop(&mut self) {
        if !self.handed_off.load(Ordering::Acquire) {
            clean_stage(&self.directory);
        }
    }
}

#[derive(Clone)]
pub struct Pending {
    pub version: String,
    stage: Arc<Stage>,
}

pub struct Updater {
    receiver: Option<mpsc::Receiver<Result<Option<Pending>, String>>>,
    commit: usize,
    language_refreshed: Arc<AtomicBool>,
}

impl Updater {
    pub fn new() -> Self {
        Self {
            receiver: None,
            commit: 0,
            language_refreshed: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn is_busy(&self) -> bool {
        self.receiver.is_some() || self.commit != 0
    }
    pub fn check(&mut self, hwnd: HWND, previews: bool, force: bool) -> bool {
        if self.is_busy() {
            return false;
        }
        let (sender, receiver) = mpsc::channel();
        let window = hwnd as usize;
        let pid = std::process::id();
        let language_refreshed = self.language_refreshed.clone();
        let worker = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let result = prepare(window, previews, force, &language_refreshed);
                let mut owner = 0;
                if sender.send(result).is_ok()
                    && unsafe { GetWindowThreadProcessId(window as HWND, &mut owner) } != 0
                    && owner == pid
                {
                    unsafe { PostMessageW(window as HWND, UPDATE_READY, 0, 0) };
                }
            });
        if worker.is_err() {
            return false;
        }
        self.receiver = Some(receiver);
        true
    }
    pub fn take_result(&mut self) -> Option<Result<Option<Pending>, String>> {
        let result = match self.receiver.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("La vérification des mises à jour a été interrompue.".into())
            }
        };
        self.receiver = None;
        Some(result)
    }
    pub fn take_language_refresh(&self) -> bool {
        self.language_refreshed.swap(false, Ordering::AcqRel)
    }
    pub fn install(&mut self, pending: Pending) -> Result<(), String> {
        if self.is_busy() {
            return Err("Une mise à jour est déjà en cours.".into());
        }
        let helper = pending.stage.directory.join("update-helper.exe");
        fs::copy(std::env::current_exe().map_err(error)?, &helper).map_err(error)?;
        let job = pending.stage.directory.join("job.json");
        let mut request_job = pending.stage.job.clone();
        request_job.request = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as usize)
            .max(1);
        fs::write(&job, serde_json::to_vec(&request_job).map_err(error)?).map_err(error)?;
        let mut process = process_tree::spawn_detached(
            &helper,
            &[
                OPTION,
                job.to_str().ok_or("Chemin de mise à jour invalide.")?,
            ],
        )
        .map_err(error)?;
        pending.stage.handed_off.store(true, Ordering::Release);
        self.commit = request_job.request;
        let request = self.commit;
        let window = pending.stage.job.parent_hwnd;
        let pid = pending.stage.job.parent_pid;
        // Also report failures before the helper can send its own notification.
        // This waiter owns only a handle; it cannot close or kill the helper.
        if let Err(error) = std::thread::Builder::new()
            .name("update-wait".into())
            .spawn(move || {
                let status = process
                    .wait()
                    .ok()
                    .and_then(|status| status.code())
                    .unwrap_or(1);
                let mut owner = 0;
                if unsafe { GetWindowThreadProcessId(window as HWND, &mut owner) } != 0
                    && owner == pid
                {
                    unsafe {
                        PostMessageW(window as HWND, UPDATE_ABORTED, request, status as isize)
                    };
                }
            })
        {
            self.commit = 0;
            return Err(error.to_string());
        }
        Ok(())
    }
    pub fn accept_commit(&self, request: usize) -> bool {
        request != 0 && self.commit == request
    }
    pub fn abort(&mut self, request: usize) -> bool {
        if !self.accept_commit(request) {
            return false;
        }
        self.commit = 0;
        true
    }
}

fn prepare(
    hwnd: usize,
    previews: bool,
    force: bool,
    language_refreshed: &AtomicBool,
) -> Result<Option<Pending>, String> {
    let root = config::data_dir().join("updates");
    let last_check = root.join(format!(
        "{}-{}-check.txt",
        if previews { "preview" } else { "stable" },
        env!("CARGO_PKG_VERSION")
    ));
    let last = bounded_read(&last_check, 32)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|text| text.parse().ok())
        .unwrap_or(0);
    let timestamp = now();
    if !force && !due(last, timestamp) {
        return Ok(None);
    }
    fs::create_dir_all(&root).map_err(error)?;
    // Persist before the request: offline machines do not retry on every launch.
    let _ = fs::write(last_check, timestamp.to_string());
    purge_old_stages(&root, timestamp);
    let language = config::load().language;
    if crate::i18n::refresh_cached_for_version(&language, &config::data_dir().join("languages"))
        .unwrap_or(false)
    {
        language_refreshed.store(true, Ordering::Release);
    }
    let bytes = http::get(
        if previews {
            API
        } else {
            "https://api.github.com/repos/roukmoute/SuperMenu/releases/latest"
        },
        MAX_METADATA,
    )?;
    let current =
        Version::parse(env!("CARGO_PKG_VERSION")).ok_or("Version du programme invalide.")?;
    let release = if previews {
        select_release(&bytes, current, true)?
    } else {
        select_stable_release(&bytes, current)?
    };
    let Some(release) = release else {
        return Ok(None);
    };
    let target = std::env::current_exe().map_err(error)?;
    let installed = installed_at(&target);
    let (asset, hash) = asset(&release, installed)?;
    let parent = Process::open(std::process::id())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let directory = root.join(format!("stage.{}.{nonce}", std::process::id()));
    let request = (nonce as usize).max(1);
    let job = Job {
        version: release.tag_name[1..].to_string(),
        target: target.clone(),
        original_sha256: hex(&file_hash(&mut locked_file(&target).map_err(error)?).map_err(error)?),
        payload_sha256: hash.to_string(),
        payload_bytes: asset.size,
        installed,
        parent_pid: std::process::id(),
        parent_created: parent.created()?,
        parent_hwnd: hwnd,
        request,
    };
    fs::create_dir(&directory).map_err(error)?;
    let pending = Pending {
        version: job.version.clone(),
        stage: Arc::new(Stage {
            directory,
            handed_off: AtomicBool::new(false),
            job,
        }),
    };
    let payload = pending.stage.directory.join(&asset.name);
    http::download(&asset.browser_download_url, &payload, asset.size as usize)?;
    drop(verified_payload(&payload, asset.size, hash, installed)?);
    Ok(Some(pending))
}

fn stage_name(name: &str) -> bool {
    let mut parts = name.split('.');
    parts.next() == Some("stage")
        && (0..2).all(|_| {
            parts.next().is_some_and(|part| {
                !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
            })
        })
        && parts.next().is_none()
}

fn clean_stage(directory: &Path) {
    for name in [INSTALLER, PORTABLE, "job.json", "update-helper.exe"] {
        let _ = fs::remove_file(directory.join(name));
    }
    let _ = fs::remove_dir(directory);
}

fn purge_old_stages(root: &Path, timestamp: u64) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten().take(256) {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir()
            && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
            && entry.file_name().to_str().is_some_and(stage_name)
            && metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .is_some_and(|time| timestamp.saturating_sub(time.as_secs()) > 7 * CHECK_INTERVAL)
        {
            clean_stage(&entry.path());
        }
    }
}

struct Process(HANDLE);
impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}
impl Process {
    fn open(pid: u32) -> Result<Self, String> {
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            Err(error(io::Error::last_os_error()))
        } else {
            Ok(Self(handle))
        }
    }
    fn created(&self) -> Result<u64, String> {
        let mut times = [FILETIME::default(); 4];
        if unsafe {
            GetProcessTimes(
                self.0,
                &mut times[0],
                &mut times[1],
                &mut times[2],
                &mut times[3],
            )
        } == 0
        {
            return Err(error(io::Error::last_os_error()));
        }
        Ok((times[0].dwHighDateTime as u64) << 32 | times[0].dwLowDateTime as u64)
    }
    fn image(&self) -> Result<PathBuf, String> {
        let mut buffer = vec![0u16; 32768];
        let mut size = buffer.len() as u32;
        if unsafe { QueryFullProcessImageNameW(self.0, 0, buffer.as_mut_ptr(), &mut size) } == 0 {
            return Err(error(io::Error::last_os_error()));
        }
        String::from_utf16(&buffer[..size as usize])
            .map(PathBuf::from)
            .map_err(error)
    }
}

fn valid_target(path: &Path) -> bool {
    path.is_absolute()
        && path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(PORTABLE))
        && !path.to_string_lossy().chars().any(char::is_control)
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
}

fn read_job(path: &Path) -> Result<(Job, PathBuf), String> {
    let executable = std::env::current_exe().map_err(error)?;
    let directory = executable
        .parent()
        .ok_or("Dossier de mise à jour invalide.")?;
    let root = config::data_dir().join("updates");
    if !path.is_absolute()
        || path.file_name().is_none_or(|name| name != "job.json")
        || !same_path(
            path.parent().ok_or("Dossier de mise à jour invalide.")?,
            directory,
        )
        || !same_path(
            directory
                .parent()
                .ok_or("Dossier de mise à jour invalide.")?,
            &root,
        )
        || directory
            .file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| !stage_name(name))
        || executable
            .file_name()
            .is_none_or(|name| name != "update-helper.exe")
        || fs::symlink_metadata(directory)
            .map_err(error)?
            .file_attributes()
            & FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    {
        return Err("Dossier de mise à jour invalide.".into());
    }
    let bytes = bounded_read(path, 8192)?;
    let job: Job =
        serde_json::from_slice(&bytes).map_err(|_| "Demande de mise à jour invalide.")?;
    if !valid_target(&job.target)
        || job.target.starts_with(&root)
        || job.request == 0
        || job.parent_pid == 0
        || job.parent_hwnd == 0
        || digest(&job.original_sha256).is_none()
        || digest(&job.payload_sha256).is_none()
        || Version::parse(&job.version)
            .is_none_or(|version| Some(version) <= Version::parse(env!("CARGO_PKG_VERSION")))
        || installed_at(&job.target) != job.installed
    {
        return Err("Demande de mise à jour invalide.".into());
    }
    Ok((job, directory.to_path_buf()))
}

fn bounded_read(path: &Path, maximum: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    locked_file(path)
        .map_err(error)?
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > maximum {
        return Err("Demande de mise à jour trop volumineuse.".into());
    }
    Ok(bytes)
}

fn verify_parent(job: &Job) -> Result<Process, String> {
    let parent = Process::open(job.parent_pid)?;
    let mut owner = 0;
    let mut class = [0u16; 64];
    let count = unsafe {
        GetClassNameW(
            job.parent_hwnd as HWND,
            class.as_mut_ptr(),
            class.len() as i32,
        )
    };
    if parent.created()? != job.parent_created
        || !same_path(&parent.image()?, &job.target)
        || unsafe { GetWindowThreadProcessId(job.parent_hwnd as HWND, &mut owner) } == 0
        || owner != job.parent_pid
        || count <= 0
        || class[..count as usize] != "SuperMenuWindow".encode_utf16().collect::<Vec<_>>()
    {
        return Err("Le processus à mettre à jour n'est pas vérifiable.".into());
    }
    Ok(parent)
}

fn apply(job: &Job, directory: &Path) -> Result<(), String> {
    let parent = verify_parent(job)?;
    let payload = directory.join(if job.installed { INSTALLER } else { PORTABLE });
    let _payload_lock = verified_payload(
        &payload,
        job.payload_bytes,
        &job.payload_sha256,
        job.installed,
    )?;
    if digest(&job.original_sha256)
        != Some(file_hash(&mut locked_file(&job.target).map_err(error)?).map_err(error)?)
    {
        return Err("Le programme a changé depuis la vérification de mise à jour.".into());
    }
    if unsafe { PostMessageW(job.parent_hwnd as HWND, UPDATE_COMMIT, job.request, 0) } == 0 {
        return Err(error(io::Error::last_os_error()));
    }
    // The UI may refuse because a menu/timer/operation became active after staging.
    if unsafe { WaitForSingleObject(parent.0, 30_000) } != WAIT_OBJECT_0 {
        unsafe { PostMessageW(job.parent_hwnd as HWND, UPDATE_ABORTED, job.request, 3) };
        return Err(DEFERRED.into());
    }
    let result = replace(job, &payload);
    // A failed replacement leaves the previous binary available; restart it too.
    let restart = process_tree::spawn_detached(&job.target, &["--background"])
        .map_err(error)
        .map(drop);
    result.and(restart)
}

fn replace(job: &Job, payload: &Path) -> Result<(), String> {
    if digest(&job.original_sha256)
        != Some(file_hash(&mut locked_file(&job.target).map_err(error)?).map_err(error)?)
    {
        return Err("Le programme a changé depuis la vérification de mise à jour.".into());
    }
    if job.installed {
        let status = process_tree::spawn_detached(payload, &["/S"])
            .map_err(error)?
            .wait()
            .map_err(error)?;
        if !status.success() {
            return Err(format!(
                "L'installateur de mise à jour a échoué ({:?}).",
                status.code()
            ));
        }
        return Ok(());
    }
    let temporary = job
        .target
        .with_file_name(format!(".supermenu-update.{}.tmp", job.request));
    let result = (|| {
        let mut destination = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(error)?;
        let mut source = locked_file(payload).map_err(error)?;
        io::copy(&mut source, &mut destination).map_err(error)?;
        destination.sync_all().map_err(error)?;
        drop(destination);
        drop(verified_payload(
            &temporary,
            job.payload_bytes,
            &job.payload_sha256,
            false,
        )?);
        if unsafe {
            MoveFileExW(
                wide(&temporary.to_string_lossy()).as_ptr(),
                wide(&job.target.to_string_lossy()).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(error(io::Error::last_os_error()));
        }
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

pub fn run_from_args(args: &[String]) -> Option<i32> {
    if !args.iter().any(|argument| argument == OPTION) {
        return None;
    }
    let [_, option, path] = args else {
        return Some(2);
    };
    if option != OPTION {
        return Some(2);
    }
    let (job, directory) = match read_job(Path::new(path)) {
        Ok(job) => job,
        Err(message) => {
            eprintln!("Super Menu : {message}");
            return Some(1);
        }
    };
    let result = apply(&job, &directory);
    let status = match &result {
        Ok(()) => "OK".to_string(),
        Err(error) => error.clone(),
    };
    let _ = fs::write(
        config::data_dir().join("updates/result.txt"),
        format!("{}\n{status}", job.version),
    );
    if result.is_ok() {
        for name in [INSTALLER, PORTABLE, "job.json"] {
            let _ = fs::remove_file(directory.join(name));
        }
    }
    Some(match result {
        Ok(()) => 0,
        Err(message) if message == DEFERRED => 3,
        Err(_) => 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions_are_numeric_and_invalid_tags_cannot_trigger_downloads() {
        assert!(Version::parse("0.6.10") > Version::parse("0.6.9"));
        for invalid in [
            "v1.0.0",
            "1.0",
            "1.0.0.0",
            "01.0.0",
            "1.0.0-beta",
            "1.0.0/path",
            "1.0.4294967296",
            " 1.0.0",
        ] {
            assert!(Version::parse(invalid).is_none(), "{invalid}");
        }
    }
    #[test]
    fn release_selection_obeys_channel_and_never_downgrades() {
        let json = br#"[{"tag_name":"v0.6.19","draft":false,"prerelease":true,"assets":[]},{"tag_name":"v0.6.18","draft":false,"prerelease":false,"assets":[]},{"tag_name":"v99.0.0","draft":true,"prerelease":false,"assets":[]},{"tag_name":"v0.6.9","draft":false,"prerelease":false,"assets":[]}]"#;
        let current = Version::parse("0.6.17").unwrap();
        assert_eq!(
            select_release(json, current, false)
                .unwrap()
                .unwrap()
                .tag_name,
            "v0.6.18"
        );
        assert_eq!(
            select_release(json, current, true)
                .unwrap()
                .unwrap()
                .tag_name,
            "v0.6.19"
        );
        assert!(select_release(json, Version::parse("1.0.0").unwrap(), true)
            .unwrap()
            .is_none());
        for invalid in [b"null".as_slice(), b"{}", b"[{\"tag_name\":\"v1.0.0\"}]"] {
            assert!(select_release(invalid, current, true).is_err());
        }
    }
    fn fixture_release() -> Release {
        Release {
            tag_name: "v1.0.0".into(),
            draft: false,
            prerelease: false,
            assets: vec![Asset {
                name: INSTALLER.into(),
                size: 512,
                digest: Some(format!("sha256:{}", "a".repeat(64))),
                browser_download_url: format!(
                    "https://github.com/roukmoute/SuperMenu/releases/download/v1.0.0/{INSTALLER}"
                ),
                state: "uploaded".into(),
            }],
        }
    }
    #[test]
    fn assets_require_exact_official_url_digest_size_and_unique_uploaded_name() {
        assert!(asset(&fixture_release(), true).is_ok());
        assert!(asset(&fixture_release(), false).is_err());
        for field in 0..7 {
            let mut release = fixture_release();
            match field {
                0 => release.assets[0].digest = None,
                1 => release.assets[0].digest = Some("sha256:bad".into()),
                2 => release.assets[0].size = MAX_PAYLOAD as u64 + 1,
                3 => release.assets[0]
                    .browser_download_url
                    .push_str("?redirect=other"),
                4 => release.assets[0].state = "new".into(),
                5 => release.assets[0].size = 0,
                _ => release.assets.push(fixture_release().assets.remove(0)),
            }
            assert!(asset(&release, true).is_err(), "{field}");
        }
    }
    #[test]
    fn checks_are_throttled_even_after_clock_rollback_and_hashes_are_strict() {
        assert!(due(0, 1));
        assert!(!due(100, 100 + CHECK_INTERVAL - 1));
        assert!(due(100, 100 + CHECK_INTERVAL));
        assert!(due(200, 100));
        let bytes: Vec<_> = (0..32).collect();
        assert_eq!(digest(&hex(&bytes)).unwrap().as_slice(), bytes);
        for invalid in ["", "sha256:abc", &"g".repeat(64), &"a".repeat(63)] {
            assert!(digest(invalid).is_none());
        }
    }
    #[test]
    fn staging_and_target_paths_cannot_escape_the_expected_layout() {
        for valid in ["stage.1.2", "stage.123.123456789"] {
            assert!(stage_name(valid));
        }
        for invalid in [
            "stage.1",
            "stage.1.2.3",
            "stage..2",
            "stage.1.-1",
            "stage.1.x",
            "other.1.2",
        ] {
            assert!(!stage_name(invalid));
        }
        assert!(valid_target(Path::new("C:\\with space É\\supermenu.exe")));
        for invalid in [
            "supermenu.exe",
            "C:\\foo\\..\\supermenu.exe",
            "C:\\foo\\other.exe",
            "C:\\foo\0\\supermenu.exe",
        ] {
            assert!(!valid_target(Path::new(invalid)));
        }
    }
    #[test]
    fn unknown_commit_tokens_cannot_authorize_a_restart() {
        let mut updater = Updater::new();
        assert!(!updater.accept_commit(0));
        updater.commit = 7;
        assert!(!updater.abort(8));
        assert!(updater.is_busy());
        assert!(updater.accept_commit(7));
        assert!(updater.abort(7));
        assert!(!updater.is_busy());
        updater.commit = 9;
        assert!(!updater.abort(7));
        assert!(updater.is_busy());
    }

    #[test]
    fn stable_endpoint_is_independent_of_the_number_of_preview_releases() {
        let current = Version::parse("0.6.17").unwrap();
        let json = br#"{"tag_name":"v1.0.0","draft":false,"prerelease":false,"assets":[]}"#;
        assert_eq!(
            select_stable_release(json, current)
                .unwrap()
                .unwrap()
                .tag_name,
            "v1.0.0"
        );
        assert!(
            select_stable_release(json, Version::parse("1.0.0").unwrap())
                .unwrap()
                .is_none()
        );
        for json in [
            br#"{"tag_name":"v2.0.0","draft":false,"prerelease":true,"assets":[]}"#.as_slice(),
            br#"{"tag_name":"v2.0.0","draft":true,"prerelease":false,"assets":[]}"#,
        ] {
            assert!(select_stable_release(json, current).unwrap().is_none());
        }
    }

    #[test]
    fn installer_bootstrapper_and_portable_payload_keep_distinct_machine_policies() {
        let directory =
            std::env::temp_dir().join(format!("SuperMenu.UpdatePE.{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("fixture.exe");
        for (machine, installed_ok, portable_ok) in [
            ([0x4c, 0x01], true, false),
            ([0x64, 0x86], true, true),
            ([0x64, 0xaa], false, false),
        ] {
            let mut bytes = vec![0u8; 128];
            bytes[..2].copy_from_slice(b"MZ");
            bytes[60] = 64;
            bytes[64..68].copy_from_slice(b"PE\0\0");
            bytes[68..70].copy_from_slice(&machine);
            fs::write(&path, &bytes).unwrap();
            let hash = hex(&Sha256::digest(&bytes));
            assert_eq!(
                verified_payload(&path, 128, &hash, true).is_ok(),
                installed_ok
            );
            assert_eq!(
                verified_payload(&path, 128, &hash, false).is_ok(),
                portable_ok
            );
            assert!(verified_payload(&path, 127, &hash, true).is_err());
            assert!(verified_payload(&path, 128, &"0".repeat(64), true).is_err());
        }
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
