//! Application artwork comes from Windows, on a dedicated COM apartment.
//! The UI only consults a bounded memory cache and sends nonblocking requests.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::mem::{size_of, zeroed};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use windows::Win32::Foundation::SIZE;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY,
};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject, GetObjectW, BITMAP, HBITMAP};
use windows_sys::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, PostMessageW, HICON, ICONINFO,
};

const CACHE_LIMIT: usize = 256;
const PENDING_LIMIT: usize = 32;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Key {
    target: String,
    size: u32,
}

impl Key {
    fn new(target: &str, size: u32) -> Option<Self> {
        if target.is_empty()
            || target.len() > 32_768
            || target.contains('\0')
            || !(1..=256).contains(&size)
        {
            return None;
        }
        Some(Self {
            target: target.to_lowercase(),
            size,
        })
    }
}

/// A private icon can move between the worker and UI; neither thread retains a
/// borrowed handle after the cache owning it has gone away.
struct OwnedIcon(usize);

impl OwnedIcon {
    fn from_raw(icon: HICON) -> Option<Self> {
        (!icon.is_null()).then_some(Self(icon as usize))
    }

    fn raw(&self) -> HICON {
        self.0 as HICON
    }
}

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        unsafe {
            DestroyIcon(self.raw());
        }
    }
}

struct OwnedBitmap(HBITMAP);

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.0);
        }
    }
}

struct Entry {
    icon: Option<OwnedIcon>,
    last_used: Cell<u64>,
}

struct Request {
    key: Key,
    target: String,
    generation: u64,
}

struct Response {
    key: Key,
    generation: u64,
    icon: Option<OwnedIcon>,
}

struct WakeTarget {
    window: Mutex<Option<usize>>,
    message: u32,
    alive: AtomicBool,
}

impl WakeTarget {
    fn wake(&self) {
        // This lock never covers extraction. Drop clears the target under the
        // same lock, so a detached worker cannot message a reused HWND later.
        let window = self
            .window
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(window) = *window {
            unsafe {
                PostMessageW(window as HWND, self.message, 0, 0);
            }
        }
    }

    fn stop(&self) {
        self.alive.store(false, Ordering::Release);
        *self
            .window
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
    }
}

pub struct IconCache {
    requests: mpsc::SyncSender<Request>,
    responses: mpsc::Receiver<Response>,
    wake: Arc<WakeTarget>,
    entries: HashMap<Key, Entry>,
    pending: HashSet<Key>,
    generation: u64,
    clock: Cell<u64>,
    capacity: usize,
}

impl IconCache {
    pub fn new(hwnd: HWND, message: u32) -> Result<Self, String> {
        Self::with_loader(hwnd, message, CACHE_LIMIT, extract_icon)
    }

    fn with_loader(
        hwnd: HWND,
        message: u32,
        capacity: usize,
        mut load: impl FnMut(&str, u32) -> Option<OwnedIcon> + Send + 'static,
    ) -> Result<Self, String> {
        let (request_sender, request_receiver) = mpsc::sync_channel::<Request>(PENDING_LIMIT);
        let (response_sender, response_receiver) = mpsc::sync_channel(PENDING_LIMIT);
        let wake = Arc::new(WakeTarget {
            window: Mutex::new((!hwnd.is_null()).then_some(hwnd as usize)),
            message,
            alive: AtomicBool::new(true),
        });
        let worker_wake = Arc::clone(&wake);
        thread::Builder::new()
            .name("supermenu-icons".into())
            .spawn(move || {
                struct Apartment;
                impl Drop for Apartment {
                    fn drop(&mut self) {
                        unsafe { CoUninitialize() };
                    }
                }
                let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
                let _apartment = initialized.then(|| Apartment);
                while let Ok(request) = request_receiver.recv() {
                    if !worker_wake.alive.load(Ordering::Acquire) {
                        break;
                    }
                    let icon = initialized
                        .then(|| load(&request.target, request.key.size))
                        .flatten();
                    if !worker_wake.alive.load(Ordering::Acquire) {
                        break;
                    }
                    if response_sender
                        .send(Response {
                            key: request.key,
                            generation: request.generation,
                            icon,
                        })
                        .is_err()
                    {
                        break;
                    }
                    worker_wake.wake();
                }
            })
            .map_err(|error| format!("Impossible de charger les icônes : {error}"))?;
        Ok(Self {
            requests: request_sender,
            responses: response_receiver,
            wake,
            entries: HashMap::new(),
            pending: HashSet::new(),
            generation: 0,
            clock: Cell::new(0),
            capacity: capacity.max(1),
        })
    }

    fn touch(&self) -> u64 {
        let clock = self.clock.get().saturating_add(1);
        self.clock.set(clock);
        clock
    }

    /// Requesting a missing icon never waits for Shell, disk or an occupied
    /// worker. Misses and failed extraction are cached just like successful icons.
    pub fn request(&mut self, target: &str, size: u32) {
        let Some(key) = Key::new(target, size) else {
            return;
        };
        if let Some(entry) = self.entries.get(&key) {
            entry.last_used.set(self.touch());
            return;
        }
        if self.pending.contains(&key) || self.pending.len() >= PENDING_LIMIT {
            return;
        }
        let request = Request {
            key: key.clone(),
            target: target.into(),
            generation: self.generation,
        };
        if self.requests.try_send(request).is_ok() {
            self.pending.insert(key);
        }
    }

    /// The handle is borrowed from the cache, and only used for immediate UI
    /// drawing. It must not be destroyed or retained across drain/clear calls.
    pub fn get(&self, target: &str, size: u32) -> Option<HICON> {
        let key = Key::new(target, size)?;
        let entry = self.entries.get(&key)?;
        entry.last_used.set(self.touch());
        entry.icon.as_ref().map(OwnedIcon::raw)
    }

    pub fn drain(&mut self) -> bool {
        let mut changed = false;
        while let Ok(response) = self.responses.try_recv() {
            if response.generation != self.generation {
                continue;
            }
            self.pending.remove(&response.key);
            if self.entries.len() >= self.capacity {
                if let Some(oldest) = self
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_used.get())
                    .map(|(key, _)| key.clone())
                {
                    self.entries.remove(&oldest);
                }
            }
            let last_used = self.touch();
            self.entries.insert(
                response.key,
                Entry {
                    icon: response.icon,
                    last_used: Cell::new(last_used),
                },
            );
            changed = true;
        }
        changed
    }

    pub fn loaded_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.icon.is_some())
            .count()
    }

    /// A manual refresh or package change permits new artwork extraction. Older
    /// responses carry a generation and cannot repopulate this cleared cache.
    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.entries.clear();
        self.pending.clear();
        while self.responses.try_recv().is_ok() {}
    }
}

impl Drop for IconCache {
    fn drop(&mut self) {
        // Never join: a third-party Shell icon provider may be slow or hung.
        // Closing the channels releases queued handles and stops later work.
        self.wake.stop();
    }
}

fn parsing_name(target: &str) -> Option<String> {
    let lower = target.to_ascii_lowercase();
    if lower.starts_with("shell:appsfolder\\") {
        let application = &target["shell:AppsFolder\\".len()..];
        // AppsFolder IDs are opaque application identities. The Shell moniker
        // binds them correctly; rebuilding a namespace GUID path does not.
        return (!application.is_empty()).then(|| format!("shell:AppsFolder\\{application}"));
    }
    if lower.starts_with("shell:::") {
        return Some(target["shell:".len()..].into());
    }
    // Rust accepts '/' in Windows paths, whereas these Shell entry points
    // reject mixed separators (E_INVALIDARG). Do not rewrite namespace IDs.
    Path::new(target)
        .is_absolute()
        .then(|| target.replace('/', "\\"))
}

fn extract_icon(target: &str, size: u32) -> Option<OwnedIcon> {
    let parsing = parsing_name(target)?;
    let wide: Vec<u16> = parsing.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if let Ok(factory) = SHCreateItemFromParsingName::<_, _, IShellItemImageFactory>(
            windows::core::PCWSTR(wide.as_ptr()),
            None,
        ) {
            if let Ok(bitmap) = factory.GetImage(
                SIZE {
                    cx: size as i32,
                    cy: size as i32,
                },
                SIIGBF_ICONONLY,
            ) {
                let bitmap = OwnedBitmap(bitmap.0);
                if let Some(icon) = bitmap_icon(&bitmap) {
                    return Some(icon);
                }
            }
        }
        if Path::new(target).is_absolute() {
            let mut information: SHFILEINFOW = zeroed();
            if SHGetFileInfoW(
                wide.as_ptr(),
                0,
                &mut information,
                size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON,
            ) != 0
            {
                return OwnedIcon::from_raw(information.hIcon);
            }
        }
    }
    None
}

unsafe fn bitmap_icon(bitmap: &OwnedBitmap) -> Option<OwnedIcon> {
    let mut description: BITMAP = zeroed();
    if GetObjectW(
        bitmap.0,
        size_of::<BITMAP>() as i32,
        (&mut description as *mut BITMAP).cast(),
    ) == 0
        || !(1..=1024).contains(&description.bmWidth)
        || !(1..=1024).contains(&description.bmHeight)
    {
        return None;
    }
    // CreateIconIndirect copies both bitmaps. Its alpha channel comes from the
    // Shell's 32-bit artwork; an all-zero AND mask preserves that transparency.
    let stride = (description.bmWidth as usize).div_ceil(16) * 2;
    let pixels = vec![0u8; stride * description.bmHeight as usize];
    let mask = OwnedBitmap(CreateBitmap(
        description.bmWidth,
        description.bmHeight,
        1,
        1,
        pixels.as_ptr().cast(),
    ));
    if mask.0.is_null() {
        return None;
    }
    OwnedIcon::from_raw(CreateIconIndirect(&ICONINFO {
        fIcon: 1,
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask.0,
        hbmColor: bitmap.0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::null_mut;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetIconInfo;

    fn wait_for(cache: &mut IconCache, predicate: impl Fn(&IconCache) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !predicate(cache) {
            cache.drain();
            assert!(Instant::now() < deadline, "icon worker timed out");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn installed_appsfolder_items_provide_icons_without_launching_them() {
        let apps = crate::native::app_catalog();
        let apps: Vec<_> = apps
            .iter()
            .filter(|app| app.target.starts_with("shell:AppsFolder\\"))
            .take(16)
            .collect();
        let mut cache = IconCache::new(null_mut(), 0).unwrap();
        for app in &apps {
            cache.request(&app.target, 24);
        }
        wait_for(&mut cache, |cache| cache.pending.is_empty());
        assert!(!apps.is_empty(), "Windows AppsFolder has no applications");
        assert!(
            cache.loaded_count() > 0,
            "Installed AppsFolder artwork unavailable"
        );
    }

    #[test]
    fn shell_namespace_and_physical_paths_are_preserved() {
        let name = "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App";
        assert_eq!(
            parsing_name(&format!("shell:AppsFolder\\{name}")),
            Some(format!("shell:AppsFolder\\{name}"))
        );
        assert_eq!(
            parsing_name("shell:::{12345678-1234-1234-1234-123456789012}\\App"),
            Some("::{12345678-1234-1234-1234-123456789012}\\App".into())
        );
        assert_eq!(
            parsing_name("C:\\Program Files\\Application\\App.exe"),
            Some("C:\\Program Files\\Application\\App.exe".into())
        );
        assert_eq!(
            parsing_name("C:\\Windows\\System32/cmd.exe"),
            Some("C:\\Windows\\System32\\cmd.exe".into())
        );
        assert!(parsing_name("ms-settings:").is_none());
        assert!(parsing_name("relative.exe").is_none());
        assert!(parsing_name("shell:AppsFolder\\").is_none());
    }

    #[test]
    fn keys_separate_dpi_and_reject_invalid_values() {
        assert_eq!(Key::new("C:\\App.exe", 24), Key::new("c:\\APP.EXE", 24));
        assert_ne!(Key::new("C:\\App.exe", 24), Key::new("C:\\App.exe", 48));
        assert!(Key::new("", 24).is_none());
        assert!(Key::new("C:\\app\0.exe", 24).is_none());
        assert!(Key::new("C:\\app.exe", 0).is_none());
        assert!(Key::new("C:\\app.exe", 257).is_none());
    }

    #[test]
    fn requests_are_deduplicated_off_thread_and_negative_results_are_cached() {
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = Arc::clone(&calls);
        let ui_thread = thread::current().id();
        let mut cache = IconCache::with_loader(null_mut(), 0, 2, move |_, _| {
            assert_ne!(thread::current().id(), ui_thread);
            worker_calls.fetch_add(1, Ordering::SeqCst);
            None
        })
        .unwrap();
        for _ in 0..20 {
            cache.request("C:\\missing.exe", 24);
        }
        wait_for(&mut cache, |cache| cache.entries.len() == 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(cache.get("C:\\missing.exe", 24).is_none());
        cache.request("c:\\MISSING.exe", 24);
        assert!(cache.pending.is_empty());
        cache.request("C:\\missing.exe", 48);
        wait_for(&mut cache, |cache| cache.entries.len() == 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn lru_bounds_successes_and_failures_and_refresh_retries() {
        let calls = Arc::new(AtomicUsize::new(0));
        let worker_calls = Arc::clone(&calls);
        let mut cache = IconCache::with_loader(null_mut(), 0, 2, move |_, _| {
            worker_calls.fetch_add(1, Ordering::SeqCst);
            None
        })
        .unwrap();
        for target in ["C:\\one.exe", "C:\\two.exe"] {
            cache.request(target, 24);
            wait_for(&mut cache, |cache| cache.pending.is_empty());
        }
        cache.get("C:\\one.exe", 24);
        cache.request("C:\\three.exe", 24);
        wait_for(&mut cache, |cache| cache.pending.is_empty());
        assert_eq!(cache.entries.len(), 2);
        assert!(cache
            .entries
            .contains_key(&Key::new("C:\\one.exe", 24).unwrap()));
        assert!(!cache
            .entries
            .contains_key(&Key::new("C:\\two.exe", 24).unwrap()));
        cache.clear();
        cache.request("C:\\one.exe", 24);
        wait_for(&mut cache, |cache| cache.entries.len() == 1);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn refresh_ignores_in_flight_old_generation() {
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let mut first = true;
        let mut cache = IconCache::with_loader(null_mut(), 0, 2, move |_, _| {
            if first {
                first = false;
                entered_sender.send(()).unwrap();
                release_receiver.recv().unwrap();
            }
            None
        })
        .unwrap();
        cache.request("C:\\old.exe", 24);
        entered_receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        cache.clear();
        cache.request("C:\\new.exe", 24);
        release_sender.send(()).unwrap();
        wait_for(&mut cache, |cache| cache.pending.is_empty());
        assert_eq!(cache.entries.len(), 1);
        assert!(cache
            .entries
            .contains_key(&Key::new("C:\\new.exe", 24).unwrap()));
    }

    #[test]
    fn dropping_ui_does_not_wait_for_a_busy_shell_provider() {
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let (finished_sender, finished_receiver) = mpsc::channel();
        let cache = IconCache::with_loader(null_mut(), 0, 2, move |_, _| {
            entered_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
            finished_sender.send(()).unwrap();
            None
        })
        .unwrap();
        let wake = Arc::clone(&cache.wake);
        let mut cache = cache;
        cache.request("C:\\slow.exe", 24);
        entered_receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        // Keep this regression bounded even if a future implementation joins.
        // The provider stays blocked until after the Drop acknowledgement.
        let (dropped_sender, dropped_receiver) = mpsc::channel();
        let dropper = thread::spawn(move || {
            drop(cache);
            dropped_sender.send(()).unwrap();
        });
        let dropped_without_waiting = dropped_receiver
            .recv_timeout(Duration::from_secs(2))
            .is_ok();
        release_sender.send(()).unwrap();
        dropper.join().unwrap();
        assert!(dropped_without_waiting, "UI shutdown waited for Shell");
        assert!(!wake.alive.load(Ordering::Acquire));
        assert!(wake.window.lock().unwrap().is_none());
        finished_receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
    }

    #[test]
    fn windows_executable_provides_owned_icon_and_releases_gdi_bitmaps() {
        let windows = std::env::var_os("WINDIR").expect("Windows directory");
        let executable = Path::new(&windows).join("System32/cmd.exe");
        assert!(executable.is_file());
        let target = executable.to_string_lossy();
        let mut cache = IconCache::new(null_mut(), 0).unwrap();
        cache.request(&target, 24);
        wait_for(&mut cache, |cache| cache.pending.is_empty());
        let icon = cache.get(&target, 24).expect("Windows executable icon");
        assert_eq!(cache.loaded_count(), 1);
        unsafe {
            let mut information: ICONINFO = zeroed();
            assert_ne!(GetIconInfo(icon, &mut information), 0);
            // GetIconInfo allocates copies of the source bitmaps: both belong
            // to this inspection, independently of the cached icon's lifetime.
            let color = OwnedBitmap(information.hbmColor);
            let mask = OwnedBitmap(information.hbmMask);
            assert!(!color.0.is_null());
            assert!(!mask.0.is_null());
            let mut description: BITMAP = zeroed();
            assert_ne!(
                GetObjectW(
                    color.0,
                    size_of::<BITMAP>() as i32,
                    (&mut description as *mut BITMAP).cast()
                ),
                0
            );
            assert!(description.bmWidth >= 24);
            assert!(description.bmHeight >= 24);
        }
        cache.clear();
        assert_eq!(cache.loaded_count(), 0);
        assert!(cache.get(&target, 24).is_none());
    }
}
