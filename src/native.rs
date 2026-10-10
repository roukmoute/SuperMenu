//! Windows integration. The keyboard hook and Shell enumeration never run on the UI thread.

use crate::core::{Item, Kind};
use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmGetWindowAttribute;
use windows_sys::Win32::Storage::FileSystem::{
    FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
    GetDriveTypeW, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
    FILE_NOTIFY_CHANGE_LAST_WRITE,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, GetCurrentThreadId, SetEvent, WaitForMultipleObjects,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const WM_ACTIVATE_MENU: u32 = WM_APP + 1;

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn win_error(context: &str) -> String {
    format!("{} : {}", context, std::io::Error::last_os_error())
}

pub fn app_catalog() -> Vec<Item> {
    let mut items = shell_applications();
    let mut titles: HashSet<String> = items.iter().map(|item| item.title.to_lowercase()).collect();
    let mut targets: HashSet<String> = items
        .iter()
        .map(|item| item.target.to_lowercase())
        .collect();
    let mut add = |title: String, target: String, subtitle: &str, glyph: &str| {
        if title.trim().is_empty()
            || target.is_empty()
            || titles.contains(&title.to_lowercase())
            || targets.contains(&target.to_lowercase())
        {
            return;
        }
        titles.insert(title.to_lowercase());
        targets.insert(target.to_lowercase());
        items.push(Item::new(
            format!("app:{}", target),
            title,
            subtitle,
            glyph,
            Kind::Application,
            target,
        ));
    };
    for variable in ["APPDATA", "PROGRAMDATA"] {
        if let Some(base) = std::env::var_os(variable) {
            let root = PathBuf::from(base).join("Microsoft/Windows/Start Menu");
            let mut pending = vec![root];
            while let Some(directory) = pending.pop() {
                let Ok(entries) = std::fs::read_dir(directory) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let Ok(file_type) = entry.file_type() else {
                        continue;
                    };
                    // Skip junctions/symlinks: do not follow cycles or traverse outside the Start menu.
                    if file_type.is_symlink() {
                        continue;
                    }
                    let path = entry.path();
                    if file_type.is_dir() {
                        pending.push(path);
                    } else if path
                        .extension()
                        .is_some_and(|value| value.eq_ignore_ascii_case("lnk"))
                    {
                        let title = path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        add(
                            title,
                            path.to_string_lossy().into_owned(),
                            "Application · Menu Démarrer",
                            "\u{E71D}",
                        );
                    }
                }
            }
        }
    }
    add(
        "Paramètres Windows".into(),
        "ms-settings:".into(),
        "Configuration Windows",
        "\u{E713}",
    );
    if let Some(directory) = std::env::var_os("WINDIR") {
        let root = PathBuf::from(directory);
        for (title, relative, glyph) in [
            ("Explorateur de fichiers", "explorer.exe", "\u{E8B7}"),
            ("Bloc-notes", "System32/notepad.exe", "\u{E70B}"),
            (
                "Panneau de configuration",
                "System32/control.exe",
                "\u{E713}",
            ),
        ] {
            let path = root.join(relative);
            if path.is_file() {
                add(
                    title.into(),
                    path.to_string_lossy().into_owned(),
                    "Application Windows",
                    glyph,
                );
            }
        }
    }
    items.sort_unstable_by_key(|item| item.title.to_lowercase());
    items
}

fn shell_applications() -> Vec<Item> {
    use windows::Win32::System::Com::{
        CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, SHGetKnownFolderItem,
        KF_FLAG_DEFAULT, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
    };
    struct ComApartment;
    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    unsafe fn display_name(
        item: &IShellItem,
        mode: windows::Win32::UI::Shell::SIGDN,
    ) -> Option<String> {
        let pointer = item.GetDisplayName(mode).ok()?;
        let text = pointer.to_string().ok();
        CoTaskMemFree(Some(pointer.0.cast::<c_void>()));
        text
    }
    let mut result = Vec::new();
    unsafe {
        if CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_err() {
            return result;
        }
        let _apartment = ComApartment;
        let Ok(folder) =
            SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
        else {
            return result;
        };
        let Ok(enumerator) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems)
        else {
            return result;
        };
        let mut names = HashSet::new();
        let mut targets = HashSet::new();
        loop {
            let mut fetched = 0;
            let mut entries = [None];
            if enumerator.Next(&mut entries, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = entries[0].take() else { break };
            let Some(title) = display_name(&item, SIGDN_NORMALDISPLAY) else {
                continue;
            };
            let Some(parsing) = display_name(&item, SIGDN_DESKTOPABSOLUTEPARSING) else {
                continue;
            };
            let prefix = "::{4234d49b-0245-4df3-b780-3893943456e1}\\";
            let target = if parsing.to_ascii_lowercase().starts_with(prefix) {
                format!("shell:AppsFolder\\{}", &parsing[prefix.len()..])
            } else if parsing.starts_with("::") {
                format!("shell:{}", parsing)
            } else if Path::new(&parsing).is_absolute() {
                parsing
            } else {
                format!("shell:AppsFolder\\{}", parsing)
            };
            if title.trim().is_empty()
                || target.is_empty()
                || !names.insert(title.to_lowercase())
                || !targets.insert(target.to_lowercase())
            {
                continue;
            }
            result.push(Item::new(
                format!("app:{}", target),
                title,
                "Application Windows",
                "\u{E71D}",
                Kind::Application,
                target,
            ));
        }
    }
    result
}

pub fn windows() -> Vec<Item> {
    unsafe extern "system" fn enumerate(hwnd: HWND, parameter: LPARAM) -> i32 {
        if IsWindowVisible(hwnd) == 0
            || !GetWindow(hwnd, GW_OWNER).is_null()
            || GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW as isize != 0
        {
            return 1;
        }
        let mut process = 0;
        GetWindowThreadProcessId(hwnd, &mut process);
        if process == std::process::id() || process == 0 {
            return 1;
        }
        let mut cloaked: u32 = 0;
        if DwmGetWindowAttribute(
            hwnd,
            14,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        ) == 0
            && cloaked != 0
        {
            return 1;
        }
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return 1;
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let count = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        if count == 0 {
            return 1;
        }
        let title = String::from_utf16_lossy(&buffer[..count as usize])
            .trim()
            .to_owned();
        if title.is_empty() {
            return 1;
        }
        let result = &mut *(parameter as *mut Vec<Item>);
        let target = (hwnd as usize).to_string();
        result.push(Item::new(
            format!("window:{}", target),
            title,
            "Fenêtre ouverte",
            "\u{E737}",
            Kind::Window,
            target,
        ));
        1
    }
    let mut result = Vec::new();
    unsafe {
        EnumWindows(Some(enumerate), &mut result as *mut Vec<Item> as LPARAM);
    }
    result
}

pub fn activate_window(target: &str) -> Result<(), String> {
    let value = target
        .parse::<usize>()
        .map_err(|_| "Fenêtre inconnue.".to_string())?;
    let hwnd = value as HWND;
    focus_window(hwnd)
}

/// Requests focus only in response to an explicit launcher activation or a selected window.
/// Never attaches another app's input queue: a hung foreground app must not hang this launcher.
pub fn focus_window(hwnd: HWND) -> Result<(), String> {
    unsafe {
        if IsWindow(hwnd) == 0 {
            return Err("Cette fenêtre est fermée.".into());
        }
        if IsIconic(hwnd) != 0 {
            ShowWindowAsync(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd) == 0 {
            return Err("Windows a empêché l’activation de cette fenêtre.".into());
        }
        let owner = GetWindowThreadProcessId(hwnd, null_mut());
        if owner == GetCurrentThreadId() {
            // Own-thread activation is synchronous. Do not direct typing into a background window.
            if GetForegroundWindow() != hwnd {
                return Err("Windows n’a pas accordé le focus au menu.".into());
            }
            BringWindowToTop(hwnd);
            SetFocus(hwnd);
        }
        // External activation can complete asynchronously on the target's message queue.
        // SetForegroundWindow's acceptance is enough; keep its focused child control intact.
    }
    Ok(())
}

pub fn launch(target: &str, arguments: &str) -> Result<(), String> {
    if target.is_empty() || target.contains('\0') || arguments.contains('\0') {
        return Err("Cible de lancement invalide.".into());
    }
    let file = wide(target);
    let arguments = wide(arguments);
    let verb = wide("open");
    unsafe {
        // Shell extensions may require COM. Balance only successful initialization on this thread.
        let initialized = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        )
        .is_ok();
        let mut info: SHELLEXECUTEINFOW = zeroed();
        info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_FLAG_NO_UI;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = arguments.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        let success = ShellExecuteExW(&mut info) != 0;
        let error = if success {
            None
        } else {
            Some(win_error("Impossible de lancer cet élément"))
        };
        if initialized {
            windows::Win32::System::Com::CoUninitialize();
        }
        if let Some(error) = error {
            return Err(error);
        }
    }
    Ok(())
}

struct HookState {
    hwnd: usize,
    left_win: bool,
    right_win: bool,
    modifiers: u8,
    space_down: bool,
    suppress_space: bool,
}
#[derive(Debug, PartialEq, Eq)]
enum KeyAction {
    Pass,
    Consume,
    Activate,
}

impl HookState {
    fn key(&mut self, key: u16, down: bool) -> KeyAction {
        match key {
            VK_LWIN => self.left_win = down,
            VK_RWIN => self.right_win = down,
            key @ VK_LSHIFT..=VK_RMENU => {
                let bit = 1 << (key - VK_LSHIFT);
                if down {
                    self.modifiers |= bit
                } else {
                    self.modifiers &= !bit
                }
            }
            VK_SPACE => {
                if down {
                    let activate = !self.space_down
                        && (self.left_win || self.right_win)
                        && self.modifiers == 0;
                    if activate {
                        self.suppress_space = true;
                    }
                    self.space_down = true;
                    if activate {
                        return KeyAction::Activate;
                    }
                    if self.suppress_space {
                        return KeyAction::Consume;
                    }
                } else {
                    self.space_down = false;
                    if self.suppress_space {
                        self.suppress_space = false;
                        return KeyAction::Consume;
                    }
                }
            }
            _ => {}
        }
        KeyAction::Pass
    }
}
thread_local! { static HOOK: RefCell<Option<HookState>> = const { RefCell::new(None) }; }

unsafe extern "system" fn keyboard_hook(code: i32, message: WPARAM, parameter: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(null_mut(), code, message, parameter);
    }
    let data = &*(parameter as *const KBDLLHOOKSTRUCT);
    // Synthetic input, including our menu mask, always passes through. No keystrokes are stored.
    if data.flags & LLKHF_INJECTED != 0 {
        return CallNextHookEx(null_mut(), code, message, parameter);
    }
    let down = message as u32 == WM_KEYDOWN || message as u32 == WM_SYSKEYDOWN;
    let up = message as u32 == WM_KEYUP || message as u32 == WM_SYSKEYUP;
    if !down && !up {
        return CallNextHookEx(null_mut(), code, message, parameter);
    }
    let (action, hwnd) = HOOK.with(|cell| {
        let mut borrowed = cell.borrow_mut();
        let Some(state) = borrowed.as_mut() else {
            return (KeyAction::Pass, 0);
        };
        (state.key(data.vkCode as u16, down), state.hwnd)
    });
    if action == KeyAction::Activate {
        let mask = [
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: 0xE8,
                        ..zeroed()
                    },
                },
            },
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: 0xE8,
                        dwFlags: KEYEVENTF_KEYUP,
                        ..zeroed()
                    },
                },
            },
        ];
        // Mark Win down as a used chord so Win up does not open Start. Never swallow Win up.
        SendInput(2, mask.as_ptr(), size_of::<INPUT>() as i32);
        PostMessageW(hwnd as HWND, WM_ACTIVATE_MENU, 0, 0);
    }
    if action != KeyAction::Pass {
        1
    } else {
        CallNextHookEx(null_mut(), code, message, parameter)
    }
}

pub struct Hotkey {
    thread_id: u32,
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Drop for Hotkey {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        unsafe {
            PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn parse_hotkey(hotkey: &str) -> Result<(u32, u32), String> {
    let parts: Vec<_> = hotkey.split('+').map(str::trim).collect();
    if !(2..=5).contains(&parts.len()) || parts.iter().any(|part| part.is_empty()) {
        return Err(
            "Raccourci invalide : utilisez un modificateur et une touche, par exemple Ctrl+Alt+K."
                .into(),
        );
    }
    let mut modifiers = 0;
    for part in &parts[..parts.len() - 1] {
        let modifier = match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => MOD_CONTROL,
            "alt" => MOD_ALT,
            "shift" => MOD_SHIFT,
            "win" | "windows" => MOD_WIN,
            _ => return Err(format!("Modificateur de raccourci inconnu : {part}.")),
        };
        if modifiers & modifier != 0 {
            return Err(format!("Modificateur de raccourci répété : {part}."));
        }
        modifiers |= modifier;
    }
    let key = parts[parts.len() - 1].to_ascii_lowercase();
    let virtual_key = match key.as_str() {
        "space" => VK_SPACE,
        "enter" => VK_RETURN,
        "tab" => VK_TAB,
        "escape" => VK_ESCAPE,
        "home" => VK_HOME,
        "end" => VK_END,
        "insert" => VK_INSERT,
        "delete" => VK_DELETE,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "pageup" => VK_PRIOR,
        "pagedown" => VK_NEXT,
        _ if key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric() => {
            key.as_bytes()[0].to_ascii_uppercase() as u16
        }
        _ => {
            let function = key
                .strip_prefix('f')
                .and_then(|number| number.parse::<u16>().ok())
                .filter(|number| (1..=24).contains(number))
                .ok_or_else(|| {
                    format!("Touche de raccourci inconnue : {}.", parts[parts.len() - 1])
                })?;
            VK_F1 + function - 1
        }
    };
    if virtual_key == VK_F12 {
        return Err(
            "F12 est réservée au débogueur par Windows. Choisissez une autre touche.".into(),
        );
    }
    // Win+L is intercepted by Windows security and must never be reported as available.
    if modifiers == MOD_WIN && virtual_key == b'L' as u16 {
        return Err(
            "Win+L est réservé au verrouillage de Windows. Choisissez un autre raccourci.".into(),
        );
    }
    Ok((modifiers, virtual_key as u32))
}

pub fn install_hotkey(hwnd: HWND, hotkey: &str) -> Result<Hotkey, String> {
    let (modifiers, virtual_key) = parse_hotkey(hotkey)?;
    let win = modifiers == MOD_WIN && virtual_key == VK_SPACE as u32;
    let label = hotkey.to_owned();
    let hwnd = hwnd as usize;
    let (send, receive) = mpsc::sync_channel(1);
    let running = Arc::new(AtomicBool::new(true));
    let thread_running = running.clone();
    let worker = thread::Builder::new()
        .name("Super Menu hotkey".into())
        .spawn(move || unsafe {
            let thread_id = GetCurrentThreadId();
            let mut message: MSG = zeroed();
            // Create the thread message queue before publishing the thread ID for disposal.
            PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
            let hook = if win {
                let pressed = |key: u16| GetAsyncKeyState(key as i32) < 0;
                let mut modifiers = 0;
                for key in VK_LSHIFT..=VK_RMENU {
                    if pressed(key) {
                        modifiers |= 1 << (key - VK_LSHIFT);
                    }
                }
                HOOK.with(|cell| {
                    *cell.borrow_mut() = Some(HookState {
                        hwnd,
                        left_win: pressed(VK_LWIN),
                        right_win: pressed(VK_RWIN),
                        modifiers,
                        space_down: pressed(VK_SPACE),
                        suppress_space: false,
                    })
                });
                let hook = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(keyboard_hook),
                    GetModuleHandleW(null()),
                    0,
                );
                if hook.is_null() {
                    let _ = send.send(Err(win_error("Impossible d’installer Win+Space")));
                    HOOK.with(|cell| cell.borrow_mut().take());
                    return;
                }
                hook
            } else {
                if RegisterHotKey(null_mut(), 1, modifiers | MOD_NOREPEAT, virtual_key) == 0 {
                    let os_error = std::io::Error::last_os_error();
                    let reserved = if modifiers & MOD_WIN != 0 {
                        " Windows réserve certaines combinaisons utilisant Win."
                    } else { "" };
                    let _ = send.send(Err(format!("Le raccourci {label} est indisponible ou déjà utilisé : {os_error}.{reserved}")));
                    return;
                }
                null_mut()
            };
            let _ = send.send(Ok(thread_id));
            while thread_running.load(Ordering::Acquire)
                && GetMessageW(&mut message, null_mut(), 0, 0) > 0
            {
                if message.message == WM_HOTKEY {
                    PostMessageW(hwnd as HWND, WM_ACTIVATE_MENU, 0, 0);
                } else {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            if !hook.is_null() {
                UnhookWindowsHookEx(hook);
                HOOK.with(|cell| cell.borrow_mut().take());
            } else {
                UnregisterHotKey(null_mut(), 1);
            }
        })
        .map_err(|error| error.to_string())?;
    match receive.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(thread_id)) => Ok(Hotkey {
            thread_id,
            running,
            worker: Some(worker),
        }),
        Ok(Err(error)) => {
            let _ = worker.join();
            Err(error)
        }
        Err(_) => {
            running.store(false, Ordering::Release);
            Err("L’initialisation du raccourci a expiré.".into())
        }
    }
}

pub struct Instance(HANDLE);
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn acquire_instance() -> Option<Instance> {
    let user = format!(
        "{}-{}",
        std::env::var("USERDOMAIN").unwrap_or_default(),
        std::env::var("USERNAME").unwrap_or_default()
    );
    let name = wide(&format!(
        "Local\\SuperMenu.{}",
        user.replace(['\\', '/'], "_")
    ));
    unsafe {
        let handle = CreateMutexW(null(), 0, name.as_ptr());
        if handle.is_null() {
            return None;
        }
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return Some(Instance(handle));
        }
        let class = wide("SuperMenuWindow");
        // A simultaneous second start can precede window creation by a few milliseconds.
        for _ in 0..20 {
            let existing = FindWindowW(class.as_ptr(), null());
            if !existing.is_null() {
                PostMessageW(existing, WM_ACTIVATE_MENU, 0, 0);
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        CloseHandle(handle);
    }
    None
}

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

pub fn startup_enabled() -> bool {
    let key = wide(RUN_KEY);
    let value = wide("SuperMenu");
    let mut size = 0;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            &mut size,
        ) == ERROR_SUCCESS
            && size > 2
    }
}

pub fn set_startup(enabled: bool) -> Result<(), String> {
    let key_path = wide(RUN_KEY);
    let value_name = wide("SuperMenu");
    unsafe {
        let mut key = null_mut();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            null(),
            &mut key,
            null_mut(),
        );
        if status != ERROR_SUCCESS {
            return Err(format!(
                "Impossible de configurer le démarrage Windows ({status})."
            ));
        }
        let status = if enabled {
            let executable = std::env::current_exe().map_err(|error| error.to_string());
            let result = match executable {
                Ok(executable) => {
                    let command = wide(&format!("\"{}\" --background", executable.display()));
                    RegSetValueExW(
                        key,
                        value_name.as_ptr(),
                        0,
                        REG_SZ,
                        command.as_ptr().cast(),
                        (command.len() * 2) as u32,
                    )
                }
                Err(error) => {
                    RegCloseKey(key);
                    return Err(error);
                }
            };
            result
        } else {
            RegDeleteValueW(key, value_name.as_ptr())
        };
        RegCloseKey(key);
        if status != ERROR_SUCCESS && (status != ERROR_FILE_NOT_FOUND || enabled) {
            return Err(format!(
                "Impossible de configurer le démarrage Windows ({status})."
            ));
        }
    }
    Ok(())
}

/// An OS notification waiter: sleeps indefinitely while the catalog is unchanged.
pub struct Watcher {
    stop: HANDLE,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            SetEvent(self.stop);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        unsafe {
            CloseHandle(self.stop);
        }
    }
}

pub fn watch(hwnd: HWND, roots: &[String]) -> Option<Watcher> {
    let mut paths: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();
    for variable in ["APPDATA", "PROGRAMDATA"] {
        if let Some(base) = std::env::var_os(variable) {
            paths.push(PathBuf::from(base).join("Microsoft/Windows/Start Menu"));
        }
    }
    let mut seen = HashSet::new();
    let mut notifications = Vec::new();
    for path in paths {
        let text = path.to_string_lossy();
        if !path.is_dir()
            || path.parent().is_none()
            || text.starts_with("\\\\")
            || !seen.insert(text.to_lowercase())
        {
            continue;
        }
        if let Some(drive) = text.get(..3).filter(|value| value.as_bytes()[1] == b':') {
            if unsafe { GetDriveTypeW(wide(drive).as_ptr()) } == 4 {
                continue;
            } // DRIVE_REMOTE
        }
        // Windows' wait limit is 64 including the stop event. Typical config has only a few roots.
        if notifications.len() == 63 {
            break;
        }
        let name = wide(&text);
        let handle = unsafe {
            FindFirstChangeNotificationW(
                name.as_ptr(),
                1,
                FILE_NOTIFY_CHANGE_FILE_NAME
                    | FILE_NOTIFY_CHANGE_DIR_NAME
                    | FILE_NOTIFY_CHANGE_LAST_WRITE,
            )
        };
        if handle != INVALID_HANDLE_VALUE && !handle.is_null() {
            notifications.push(handle as usize);
        }
    }
    if notifications.is_empty() {
        return None;
    }
    let stop = unsafe { CreateEventW(null(), 1, 0, null()) };
    if stop.is_null() {
        for handle in notifications {
            unsafe {
                FindCloseChangeNotification(handle as HANDLE);
            }
        }
        return None;
    }
    let stop_value = stop as usize;
    let hwnd_value = hwnd as usize;
    let cleanup = notifications.clone();
    let worker = thread::Builder::new()
        .name("Super Menu catalog watcher".into())
        .spawn(move || {
            let mut handles: Vec<HANDLE> = std::iter::once(stop_value as HANDLE)
                .chain(notifications.iter().map(|value| *value as HANDLE))
                .collect();
            let mut dirty_since: Option<Instant> = None;
            let mut last_change = Instant::now();
            loop {
                if dirty_since.is_some_and(|first| first.elapsed() >= Duration::from_secs(1)) {
                    dirty_since = None;
                    unsafe {
                        PostMessageW(hwnd_value as HWND, WM_APP + 3, 0, 0);
                    }
                }
                let timeout = match dirty_since {
                    None => u32::MAX,
                    Some(first) => {
                        let trailing =
                            Duration::from_millis(250).saturating_sub(last_change.elapsed());
                        let maximum = Duration::from_secs(1).saturating_sub(first.elapsed());
                        trailing.min(maximum).as_millis() as u32
                    }
                };
                let result = unsafe {
                    WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, timeout)
                };
                if result == WAIT_OBJECT_0 || result == WAIT_FAILED {
                    break;
                }
                if result == WAIT_TIMEOUT {
                    if dirty_since.take().is_some() {
                        unsafe {
                            PostMessageW(hwnd_value as HWND, WM_APP + 3, 0, 0);
                        }
                    }
                    continue;
                }
                let index = result.wrapping_sub(WAIT_OBJECT_0) as usize;
                if index > 0 && index < handles.len() {
                    let now = Instant::now();
                    dirty_since.get_or_insert(now);
                    last_change = now;
                    if unsafe { FindNextChangeNotification(handles[index]) } == 0 {
                        unsafe {
                            FindCloseChangeNotification(handles[index]);
                        }
                        handles.remove(index);
                    }
                } else {
                    break;
                }
            }
            for handle in handles.iter().skip(1) {
                unsafe {
                    FindCloseChangeNotification(*handle);
                }
            }
        });
    match worker {
        Ok(worker) => Some(Watcher {
            stop,
            worker: Some(worker),
        }),
        Err(_) => {
            for handle in cleanup {
                unsafe {
                    FindCloseChangeNotification(handle as HANDLE);
                }
            }
            unsafe {
                CloseHandle(stop);
            }
            None
        }
    }
}

pub fn lock_workstation() -> Result<(), String> {
    // This User32 export is stable since Windows 2000; avoid another generated module for one symbol.
    #[link(name = "user32")]
    extern "system" {
        fn LockWorkStation() -> i32;
    }
    if unsafe { LockWorkStation() } == 0 {
        Err(win_error("Impossible de verrouiller Windows"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hotkey_parser_accepts_modifiers_letters_digits_functions_and_named_keys() {
        for (input, modifiers, key) in [
            ("Ctrl+Alt+K", MOD_CONTROL | MOD_ALT, b'K' as u32),
            (
                " control + shift + 7 ",
                MOD_CONTROL | MOD_SHIFT,
                b'7' as u32,
            ),
            ("Shift+Windows+F24", MOD_SHIFT | MOD_WIN, VK_F24 as u32),
            ("Alt+F1", MOD_ALT, VK_F1 as u32),
            ("Win+Space", MOD_WIN, VK_SPACE as u32),
            ("Alt+PageDown", MOD_ALT, VK_NEXT as u32),
            ("Ctrl+Enter", MOD_CONTROL, VK_RETURN as u32),
        ] {
            assert_eq!(parse_hotkey(input).unwrap(), (modifiers, key));
        }
    }

    #[test]
    fn hotkey_parser_rejects_duplicates_invalid_keys_and_reserved_combinations() {
        for input in [
            "A",
            "Win++A",
            "Ctrl+Control+K",
            "Alt+F25",
            "Meta+K",
            "Ctrl+é",
            "Ctrl+Alt+F12",
            "Win+L",
        ] {
            assert!(parse_hotkey(input).is_err(), "{input} should be rejected");
        }
        assert!(parse_hotkey("Alt+F12").unwrap_err().contains("débogueur"));
        assert!(parse_hotkey("Win+L").unwrap_err().contains("verrouillage"));
    }
    fn state() -> HookState {
        HookState {
            hwnd: 0,
            left_win: false,
            right_win: false,
            modifiers: 0,
            space_down: false,
            suppress_space: false,
        }
    }

    #[test]
    fn win_space_consumes_press_repeat_release_but_delivers_win_up() {
        let mut keys = state();
        assert_eq!(keys.key(VK_LWIN, true), KeyAction::Pass);
        assert_eq!(keys.key(VK_SPACE, true), KeyAction::Activate);
        assert_eq!(keys.key(VK_SPACE, true), KeyAction::Consume);
        assert_eq!(keys.key(VK_LWIN, false), KeyAction::Pass);
        assert_eq!(keys.key(VK_SPACE, false), KeyAction::Consume);
        assert_eq!(keys.key(VK_SPACE, true), KeyAction::Pass);
        assert_eq!(keys.key(VK_SPACE, false), KeyAction::Pass);
    }

    #[test]
    fn both_win_keys_work_and_normal_win_shortcuts_pass() {
        for win in [VK_LWIN, VK_RWIN] {
            let mut keys = state();
            assert_eq!(keys.key(win, true), KeyAction::Pass);
            assert_eq!(keys.key(b'E' as u16, true), KeyAction::Pass);
            assert_eq!(keys.key(b'E' as u16, false), KeyAction::Pass);
            assert_eq!(keys.key(VK_SPACE, true), KeyAction::Activate);
            assert_eq!(keys.key(VK_SPACE, false), KeyAction::Consume);
            assert_eq!(keys.key(win, false), KeyAction::Pass);
        }
    }

    #[test]
    fn modified_win_space_and_preheld_space_are_not_captured() {
        for modifier in VK_LSHIFT..=VK_RMENU {
            let mut keys = state();
            keys.key(modifier, true);
            keys.key(VK_LWIN, true);
            assert_eq!(keys.key(VK_SPACE, true), KeyAction::Pass);
            assert_eq!(keys.key(VK_SPACE, false), KeyAction::Pass);
            keys.key(modifier, false);
            assert_eq!(keys.key(VK_SPACE, true), KeyAction::Activate);
        }
        let mut keys = state();
        assert_eq!(keys.key(VK_SPACE, true), KeyAction::Pass);
        keys.key(VK_LWIN, true);
        assert_eq!(keys.key(VK_SPACE, true), KeyAction::Pass);
    }

    #[test]
    fn directory_watcher_delivers_a_notification_and_stops() {
        let folder =
            std::env::temp_dir().join(format!("supermenu-watch-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let class = wide("STATIC");
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                0,
                0,
                0,
                0,
                0,
                -3isize as HWND,
                null_mut(),
                null_mut(),
                null(),
            )
        };
        assert!(!hwnd.is_null());
        let watcher = watch(hwnd, &[folder.to_string_lossy().into_owned()]).expect("watcher");
        let file = folder.join("notification.txt");
        std::fs::write(&file, "catalog change").unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut message: MSG = unsafe { zeroed() };
        let mut delivered = false;
        while Instant::now() < deadline {
            if unsafe { PeekMessageW(&mut message, hwnd, WM_APP + 3, WM_APP + 3, PM_REMOVE) } != 0 {
                delivered = true;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let stop_start = Instant::now();
        drop(watcher);
        unsafe {
            DestroyWindow(hwnd);
        }
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(folder).unwrap();
        assert!(
            delivered,
            "filesystem change should post a catalog notification"
        );
        assert!(
            stop_start.elapsed() < Duration::from_secs(1),
            "disposal should interrupt the idle wait"
        );
    }
}
