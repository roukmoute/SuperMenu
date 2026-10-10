//! Small native language picker. Downloads only the selected, uncached pack.
use crate::{i18n, native::wide};
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId},
    UI::{
        Input::KeyboardAndMouse::{EnableWindow, SetFocus},
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "SuperMenuLanguage";
const READY: u32 = WM_APP + 32;
static NEXT_TOKEN: AtomicUsize = AtomicUsize::new(1);

struct Dialog {
    combo: HWND,
    status: HWND,
    button: HWND,
    result: Option<String>,
    done: bool,
    loading: bool,
    token: usize,
    cache_root: PathBuf,
    receiver: Option<mpsc::Receiver<(String, Result<(), String>)>>,
}

/// Preparing a pack never changes the active language; callers apply it on save.
pub unsafe fn choose(
    parent: HWND,
    font: HFONT,
    dpi: i32,
    current: &str,
    cache_root: PathBuf,
) -> Option<String> {
    let instance = GetModuleHandleW(null());
    let class = wide(CLASS);
    RegisterClassW(&WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: (COLOR_WINDOW + 1) as _,
        lpszClassName: class.as_ptr(),
        ..zeroed()
    });
    let mut dialog = Box::new(Dialog {
        combo: null_mut(),
        status: null_mut(),
        button: null_mut(),
        result: None,
        done: false,
        loading: false,
        token: NEXT_TOKEN.fetch_add(1, Ordering::Relaxed),
        cache_root,
        receiver: None,
    });
    let scale = |n: i32| n * dpi.max(96) / 96;
    let mut monitor: MONITORINFO = zeroed();
    monitor.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(
        MonitorFromWindow(parent, MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    );
    let width = scale(480);
    let height = scale(245);
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_DLGMODALFRAME,
        class.as_ptr(),
        wide("Super Menu · Langue / Language").as_ptr(),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
        monitor.rcWork.left + (monitor.rcWork.right - monitor.rcWork.left - width) / 2,
        monitor.rcWork.top + (monitor.rcWork.bottom - monitor.rcWork.top - height) / 2,
        width,
        height,
        parent,
        null_mut(),
        instance,
        (&mut *dialog as *mut Dialog).cast(),
    );
    if hwnd.is_null() {
        return None;
    }
    let control = |class: &str, caption: &str, style: u32, y: i32, h: i32, id: usize| {
        let child = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(caption).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            scale(20),
            scale(y),
            scale(420),
            scale(h),
            hwnd,
            id as _,
            instance,
            null(),
        );
        SendMessageW(child, WM_SETFONT, font as usize, 1);
        child
    };
    control(
        "STATIC",
        "Choisissez votre langue / Choose your language",
        0,
        16,
        24,
        0,
    );
    dialog.combo = control(
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP,
        48,
        110,
        401,
    );
    for language in i18n::LANGUAGES {
        SendMessageW(
            dialog.combo,
            CB_ADDSTRING,
            0,
            wide(language.name).as_ptr() as isize,
        );
    }
    let selected = i18n::LANGUAGES
        .iter()
        .position(|language| language.code == current)
        .unwrap_or(0);
    SendMessageW(dialog.combo, CB_SETCURSEL, selected, 0);
    dialog.status = control(
        "STATIC",
        "Un seul fichier, puis hors ligne. / One file, then offline.",
        0,
        91,
        39,
        402,
    );
    dialog.button = control(
        "BUTTON",
        "Continuer / Continue",
        BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
        145,
        30,
        403,
    );
    EnableWindow(parent, 0);
    ShowWindow(hwnd, SW_SHOW);
    // A background startup may request SW_HIDE for the first ShowWindow call.
    // The first-run choice still needs to be visible.
    SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        0,
        0,
        0,
        0,
        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
    SetForegroundWindow(hwnd);
    SetFocus(dialog.combo);
    let mut message: MSG = zeroed();
    loop {
        if dialog.done {
            break;
        }
        if GetMessageW(&mut message, null_mut(), 0, 0) <= 0 {
            PostQuitMessage(0);
            break;
        }
        if IsDialogMessageW(hwnd, &message) == 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    if IsWindow(hwnd) != 0 {
        DestroyWindow(hwnd);
    }
    EnableWindow(parent, 1);
    dialog.result.take()
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: usize, lp: isize) -> isize {
    if message == WM_NCCREATE {
        let create = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return 1;
    }
    let Some(dialog) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Dialog).as_mut() else {
        return DefWindowProcW(hwnd, message, wp, lp);
    };
    match message {
        WM_COMMAND if matches!(wp & 0xffff, 403 | 1) && !dialog.loading => {
            let selected = SendMessageW(dialog.combo, CB_GETCURSEL, 0, 0);
            let Some(language) = i18n::LANGUAGES.get(selected as usize) else {
                return 0;
            };
            let code = language.code.to_string();
            let cache_root = dialog.cache_root.clone();
            let token = dialog.token;
            let target = hwnd as usize;
            let pid = GetCurrentProcessId();
            let (send, receiver) = mpsc::channel();
            dialog.receiver = Some(receiver);
            dialog.loading = true;
            EnableWindow(dialog.combo, 0);
            EnableWindow(dialog.button, 0);
            SetWindowTextW(dialog.status, wide("Chargement… / Loading…").as_ptr());
            let started = std::thread::Builder::new()
                .name("Super Menu language".into())
                .spawn(move || {
                    let result = i18n::ensure_cached(&code, &cache_root);
                    let _ = send.send((code, result));
                    // A closed picker must not post into a recycled handle in another process.
                    let mut owner = 0;
                    if unsafe { GetWindowThreadProcessId(target as HWND, &mut owner) } != 0
                        && owner == pid
                    {
                        unsafe {
                            PostMessageW(target as HWND, READY, token, 0);
                        }
                    }
                });
            if let Err(error) = started {
                dialog.loading = false;
                EnableWindow(dialog.combo, 1);
                EnableWindow(dialog.button, 1);
                SetWindowTextW(dialog.status, wide(&error.to_string()).as_ptr());
            }
            0
        }
        READY if wp == dialog.token => {
            let response = dialog
                .receiver
                .as_ref()
                .and_then(|receiver| receiver.try_recv().ok());
            if let Some((code, result)) = response {
                dialog.receiver = None;
                dialog.loading = false;
                match result {
                    Ok(()) => {
                        dialog.result = Some(code);
                        dialog.done = true;
                    }
                    Err(error) => {
                        EnableWindow(dialog.combo, 1);
                        EnableWindow(dialog.button, 1);
                        SetWindowTextW(dialog.status, wide("Échec réseau. Réessayez ou choisissez Français. / Retry or choose Français.").as_ptr());
                        MessageBoxW(
                            hwnd,
                            wide(&error).as_ptr(),
                            wide("Super Menu · Langue / Language").as_ptr(),
                            MB_OK | MB_ICONWARNING,
                        );
                    }
                }
            }
            0
        }
        WM_CLOSE => {
            dialog.done = true;
            0
        }
        WM_COMMAND if wp & 0xffff == 2 => {
            dialog.done = true;
            0
        }
        _ => DefWindowProcW(hwnd, message, wp, lp),
    }
}
