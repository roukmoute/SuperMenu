use crate::{catalog, config, core, native};
use core::{Item, Kind};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Instant;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::{DataExchange::*, LibraryLoader::*, Memory::*, ProcessStatus::*, Threading::*},
    UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

const SHOW: u32 = WM_APP + 1;
const INDEX: u32 = WM_APP + 2;
const REFRESH: u32 = WM_APP + 3;
const TRAY: u32 = WM_APP + 4;
const EDIT_ID: usize = 100;
const ROW_TOP: i32 = 177;
const ROW_HEIGHT: i32 = 48;
const CLASS: &str = "SuperMenuWindow";
const CF_UNICODETEXT: u32 = 13;
thread_local! { static DEFERRED:Cell<u8>=const {Cell::new(0)}; }

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn color(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

struct Palette {
    bg: u32,
    panel: u32,
    text: u32,
    muted: u32,
    line: u32,
    accent: u32,
    selected: u32,
}
impl Palette {
    fn from(s: &config::Settings) -> Self {
        let accent =
            u32::from_str_radix(s.accent_color.trim_start_matches('#'), 16).unwrap_or(0x9a8cff);
        let accent = color((accent >> 16) & 255, (accent >> 8) & 255, accent & 255);
        if s.theme == "light" {
            Self {
                bg: color(247, 247, 251),
                panel: color(235, 235, 243),
                text: color(30, 28, 43),
                muted: color(103, 101, 119),
                line: color(213, 211, 224),
                accent,
                selected: color(225, 220, 245),
            }
        } else {
            Self {
                bg: color(17, 18, 22),
                panel: color(25, 26, 33),
                text: color(241, 240, 247),
                muted: color(148, 147, 165),
                line: color(45, 44, 57),
                accent,
                selected: color(41, 37, 53),
            }
        }
    }
}

struct State {
    hwnd: HWND,
    edit: HWND,
    settings: config::Settings,
    palette: Palette,
    index: Arc<Mutex<Vec<Item>>>,
    indexing: Arc<AtomicBool>,
    usage: catalog::Usage,
    weights: HashMap<String, u32>,
    clipboard: catalog::ClipHistory,
    items: Vec<Item>,
    selected: usize,
    query: String,
    section: String,
    timing: String,
    status: String,
    fonts: [HFONT; 5],
    edit_bg: HBRUSH,
    icon: HICON,
    hotkey: Option<native::Hotkey>,
    watcher: Option<native::Watcher>,
    dpi: i32,
    visible_rows: usize,
    previous: HWND,
    settings_open: bool,
    smoke: bool,
    pending_refresh: bool,
    checks: Vec<(String, bool)>,
    started: Instant,
    first_frame_ms: f64,
}

pub fn show_error(error: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(error).as_ptr(),
            wide("Super Menu").as_ptr(),
            MB_ICONERROR | MB_OK,
        );
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let started = Instant::now();
    if !cfg!(target_os = "windows") {
        return Err("Super Menu fonctionne uniquement sous Windows.".into());
    }
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let smoke = args.iter().any(|a| a == "--smoke-test");
        let _instance = if smoke {
            None
        } else {
            match native::acquire_instance() {
                Some(x) => Some(x),
                None => return Ok(()),
            }
        };
        let instance = GetModuleHandleW(null());
        let class_name = wide(CLASS);
        let icon = make_icon(instance);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class_name.as_ptr(),
            hIcon: icon,
            hIconSm: icon,
            style: CS_HREDRAW | CS_VREDRAW,
            ..zeroed()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(format!(
                "Création de la fenêtre impossible ({})",
                GetLastError()
            ));
        }
        let settings = config::load();
        let cache = Vec::new();
        let usage = catalog::Usage::new();
        let weights = usage.snapshot_for(&cache);
        let state_cell = Box::new(RefCell::new(State {
            hwnd: null_mut(),
            edit: null_mut(),
            palette: Palette::from(&settings),
            settings,
            index: Arc::new(Mutex::new(cache)),
            indexing: Arc::new(AtomicBool::new(false)),
            usage,
            weights,
            clipboard: catalog::ClipHistory::new(),
            items: vec![],
            selected: 0,
            query: String::new(),
            section: "EXPLORER".into(),
            timing: String::new(),
            status: "Index local".into(),
            fonts: [null_mut(); 5],
            edit_bg: null_mut(),
            icon,
            hotkey: None,
            watcher: None,
            dpi: 96,
            visible_rows: 9,
            previous: null_mut(),
            settings_open: false,
            smoke,
            pending_refresh: false,
            checks: Vec::new(),
            started,
            first_frame_ms: 0.,
        }));
        let ptr = &*state_cell as *const RefCell<State>;
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            wide("Super Menu").as_ptr(),
            WS_POPUP | WS_THICKFRAME | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            720,
            660,
            null_mut(),
            null_mut(),
            instance,
            ptr.cast(),
        );
        if hwnd.is_null() {
            return Err(format!("CreateWindowExW ({})", GetLastError()));
        }
        let mut state = state_cell.borrow_mut();
        state.hwnd = hwnd;
        let dark: i32 = if state.settings.theme == "light" {
            0
        } else {
            1
        };
        DwmSetWindowAttribute(hwnd, 20, (&dark as *const i32).cast(), 4);
        let round: i32 = 2;
        DwmSetWindowAttribute(hwnd, 33, (&round as *const i32).cast(), 4);
        let margins = MARGINS {
            cxLeftWidth: 1,
            cxRightWidth: 1,
            cyTopHeight: 1,
            cyBottomHeight: 1,
        };
        DwmExtendFrameIntoClientArea(hwnd, &margins);
        state.edit = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            null(),
            WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL as u32,
            69,
            72,
            440,
            36,
            hwnd,
            EDIT_ID as _,
            instance,
            null(),
        );
        SetWindowSubclass(state.edit, Some(edit_proc), 1, 0);
        SendMessageW(state.edit, EM_SETLIMITTEXT, 1024, 0);
        SendMessageW(
            state.edit,
            EM_SETCUEBANNER,
            1,
            wide("Où veux-tu aller ?").as_ptr() as isize,
        );
        rebuild_fonts(&mut state);
        place(&mut state);
        update_results(&mut state);
        if !smoke {
            match native::install_hotkey(hwnd, &state.settings.hotkey) {
                Ok(h) => state.hotkey = Some(h),
                Err(e) => {
                    state.status = "Raccourci indisponible · icône de notification".into();
                    show_error(&e);
                }
            }
            AddClipboardFormatListener(hwnd);
            tray(&state, NIM_ADD);
            state.watcher = native::watch(hwnd, &state.settings.file_roots);
        }
        if !args.iter().any(|a| a == "--background") || smoke {
            show(&mut state);
        }
        reindex(&mut state);
        if smoke {
            SetTimer(hwnd, 99, 8500, None);
        }
        drop(state);
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let mut state = state_cell.borrow_mut();
        if smoke {
            if let Some(pos) = args.iter().position(|a| a == "--report") {
                if let Some(path) = args.get(pos + 1) {
                    let items = state.index.lock().unwrap_or_else(|e| e.into_inner());
                    let mut memory: PROCESS_MEMORY_COUNTERS_EX = zeroed();
                    memory.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
                    K32GetProcessMemoryInfo(
                        GetCurrentProcess(),
                        (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                        memory.cb,
                    );
                    let report = serde_json::json!({"native_window_created":true,"edit_created":!state.edit.is_null(),
                        "index_items":items.len(),"applications":items.iter().filter(|i|i.kind==Kind::Application).count(),
                        "files":items.iter().filter(|i|matches!(i.kind,Kind::File|Kind::Folder)).count(),
                        "visible_results":state.items.len(),"indexing_finished":!state.indexing.load(Ordering::Relaxed),
                        "first_frame_ms":state.first_frame_ms,"private_bytes":memory.PrivateUsage,"working_set_bytes":memory.WorkingSetSize,"checks":state.checks});
                    let _ = std::fs::write(path, serde_json::to_string_pretty(&report).unwrap());
                }
            }
        }
        RemoveClipboardFormatListener(hwnd);
        state.hotkey.take();
        state.watcher.take();
        tray(&state, NIM_DELETE);
        DestroyWindow(hwnd);
        for font in state.fonts {
            if !font.is_null() {
                DeleteObject(font);
            }
        }
        DeleteObject(state.edit_bg);
        DestroyIcon(icon);
        Ok(())
    }
}

unsafe fn state(hwnd: HWND) -> Option<&'static RefCell<State>> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<State>).as_ref()
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_NCCREATE {
        let create = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return 1;
    }
    if msg == WM_NCCALCSIZE && wp != 0 {
        return 0;
    }
    let Some(cell) = state(hwnd) else {
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    let Ok(mut borrowed) = cell.try_borrow_mut() else {
        if msg == INDEX || msg == REFRESH {
            DEFERRED.with(|d| d.set(d.get() | if msg == INDEX { 1 } else { 2 }));
        }
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    let s = &mut *borrowed;
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            paint(s, dc);
            EndPaint(hwnd, &ps);
            if s.first_frame_ms == 0. {
                s.first_frame_ms = s.started.elapsed().as_secs_f64() * 1000.;
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_PRINTCLIENT => {
            paint(s, wp as HDC);
            0
        }
        WM_CTLCOLOREDIT => {
            let dc = wp as HDC;
            SetTextColor(dc, s.palette.text);
            SetBkColor(dc, s.palette.panel);
            s.edit_bg as isize
        }
        WM_COMMAND => {
            if wp & 0xffff == EDIT_ID && (wp >> 16) & 0xffff == EN_CHANGE as usize {
                update_results(s);
            }
            0
        }
        WM_KEYDOWN => {
            key(s, wp as u32);
            0
        }
        WM_LBUTTONDOWN => {
            let x = (lp as i16) as i32;
            let y = ((lp >> 16) as i16) as i32;
            let x = x * 96 / s.dpi;
            let y = y * 96 / s.dpi;
            if y < 52 {
                let mut bounds: RECT = zeroed();
                GetClientRect(hwnd, &mut bounds);
                let width = bounds.right * 96 / s.dpi;
                if x > width - 56 {
                    hide(s);
                } else if x > width - 98 {
                    settings(s);
                } else {
                    ReleaseCapture();
                    SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
                }
            } else if (128..158).contains(&y) {
                let prefix = if x < 75 {
                    ""
                } else if x < 215 {
                    "@ "
                } else if x < 310 {
                    "/ "
                } else if x < 375 {
                    "? "
                } else {
                    ";"
                };
                set_query(s, prefix);
            } else if y >= ROW_TOP {
                let row = ((y - ROW_TOP) / ROW_HEIGHT) as usize;
                let offset = s.selected.saturating_sub(s.visible_rows.saturating_sub(1));
                if row < s.visible_rows && row + offset < s.items.len() {
                    s.selected = row + offset;
                    execute(s, false);
                }
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp >> 16) as i16) as i32;
            if delta < 0 {
                select(s, 1);
            } else {
                select(s, -1);
            }
            0
        }
        WM_ACTIVATE => {
            if wp & 0xffff == WA_INACTIVE as usize && !s.settings_open && !s.smoke {
                hide(s);
            }
            0
        }
        WM_DPICHANGED => {
            s.dpi = (wp & 0xffff) as i32;
            rebuild_fonts(s);
            let rect = &*(lp as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            layout_edit(s);
            0
        }
        WM_SIZE => {
            layout_edit(s);
            0
        }
        WM_CLIPBOARDUPDATE => {
            if s.settings.clipboard_enabled
                && s.clipboard.capture(s.settings.max_clipboard_entries)
                && s.query.starts_with(':')
            {
                update_results(s);
            }
            0
        }
        SHOW => {
            if IsWindowVisible(hwnd) != 0 {
                hide(s);
            } else {
                show(s);
            }
            0
        }
        INDEX => {
            let index = s.index.lock().unwrap_or_else(|e| e.into_inner());
            s.weights = s.usage.snapshot_for(&index);
            let apps = index.iter().filter(|i| i.kind == Kind::Application).count();
            s.status = if s.indexing.load(Ordering::Relaxed) {
                format!("{apps} applis · indexation…")
            } else {
                format!("{apps} applis · {} entrées", index.len())
            };
            drop(index);
            update_results(s);
            if !s.indexing.load(Ordering::Relaxed) && s.pending_refresh {
                s.pending_refresh = false;
                reindex(s);
            }
            0
        }
        REFRESH => {
            reindex(s);
            0
        }
        TRAY => {
            if lp as u32 == WM_LBUTTONUP || lp as u32 == WM_LBUTTONDBLCLK {
                show(s);
            } else if lp as u32 == WM_RBUTTONUP {
                let menu = CreatePopupMenu();
                AppendMenuW(menu, MF_STRING, 1, wide("Ouvrir Super Menu").as_ptr());
                AppendMenuW(menu, MF_STRING, 2, wide("Paramètres").as_ptr());
                AppendMenuW(menu, MF_STRING, 3, wide("Actualiser l'index").as_ptr());
                AppendMenuW(menu, MF_SEPARATOR, 0, null());
                AppendMenuW(menu, MF_STRING, 4, wide("Quitter").as_ptr());
                let mut p: POINT = zeroed();
                GetCursorPos(&mut p);
                SetForegroundWindow(hwnd);
                let action = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    p.x,
                    p.y,
                    0,
                    hwnd,
                    null(),
                );
                DestroyMenu(menu);
                match action {
                    1 => show(s),
                    2 => {
                        show(s);
                        settings(s);
                    }
                    3 => reindex(s),
                    4 => {
                        PostQuitMessage(0);
                    }
                    _ => {}
                }
            }
            0
        }
        WM_TIMER => {
            if wp == 99 {
                smoke_checks(s);
                capture_bmp(s);
                PostQuitMessage(0);
            } else if !s.edit.is_null() && IsWindowVisible(hwnd) != 0 && s.query.starts_with('~') {
                update_results(s);
            }
            0
        }
        WM_CLOSE => {
            hide(s);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, 1);
            KillTimer(hwnd, 99);
            PostQuitMessage(0);
            0
        }
        WM_NCHITTEST => HTCLIENT as isize,
        _ => {
            drop(borrowed);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _id: usize,
    _data: usize,
) -> isize {
    if msg == WM_KEYDOWN {
        if let Some(cell) = state(GetParent(hwnd)) {
            let Ok(mut borrowed) = cell.try_borrow_mut() else {
                return DefSubclassProc(hwnd, msg, wp, lp);
            };
            let s = &mut *borrowed;
            let ctrl = GetKeyState(VK_CONTROL as i32) < 0;
            if matches!(
                wp as u16,
                VK_UP | VK_DOWN | VK_RETURN | VK_ESCAPE | VK_TAB | VK_F1 | VK_F5
            ) || (ctrl && wp == VK_OEM_COMMA as usize)
            {
                key(s, wp as u32);
                return 0;
            }
        }
    }
    if msg == WM_CHAR
        && (wp == VK_RETURN as usize || wp == VK_ESCAPE as usize || wp == VK_TAB as usize)
    {
        return 0;
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

unsafe fn key(s: &mut State, key: u32) {
    match key as u16 {
        VK_UP => select(s, -1),
        VK_DOWN => select(s, 1),
        VK_RETURN => execute(s, GetKeyState(VK_CONTROL as i32) < 0),
        VK_ESCAPE => {
            if s.query.is_empty() {
                hide(s);
            } else {
                set_query(s, "");
            }
        }
        VK_TAB => set_query(s, ";"),
        VK_F5 => reindex(s),
        VK_F1 => {
            s.settings_open = true;
            MessageBoxW(s.hwnd,wide("Win + Espace : ouvrir / masquer\n↑ ↓ : sélectionner · Entrée : ouvrir\nCtrl + Entrée : dossier parent d'un fichier\nÉchap : effacer puis masquer · F5 : réindexer\nCtrl + , : paramètres · Tab / ; : modules\n\n@ applications    / fichiers    = calcul\n? recherche web    > commandes    : presse-papiers\n. symboles    ~ fenêtres    ! système\n\nLa recherche et l'index restent locaux.\nLe presse-papiers est désactivé par défaut.").as_ptr(),wide("Super Menu · Aide").as_ptr(),MB_OK);
            s.settings_open = false;
            finish_modal(s);
            SetFocus(s.edit);
        }
        VK_OEM_COMMA if GetKeyState(VK_CONTROL as i32) < 0 => settings(s),
        _ => {}
    }
}
unsafe fn select(s: &mut State, delta: i32) {
    if s.items.is_empty() {
        return;
    }
    s.selected = (s.selected as i32 + delta).rem_euclid(s.items.len() as i32) as usize;
    InvalidateRect(s.hwnd, null(), 0);
}
unsafe fn set_query(s: &mut State, text: &str) {
    SetWindowTextW(s.edit, wide(text).as_ptr());
    SetFocus(s.edit);
    SendMessageW(s.edit, EM_SETSEL, usize::MAX, -1);
    update_results(s);
}

fn navigation() -> Vec<Item> {
    [
        (
            "Applications",
            "Lancer tes applications installées",
            "▦",
            "@ ",
        ),
        (
            "Fichiers",
            "Retrouver un fichier dans tes dossiers",
            "▱",
            "/ ",
        ),
        (
            "Calculatrice",
            "Calculer puis copier le résultat",
            "=",
            "= ",
        ),
        ("Recherche web", "Chercher dans ton navigateur", "⌕", "? "),
        (
            "Commandes",
            "Exécuter une commande ou un raccourci",
            "›",
            "> ",
        ),
        ("Presse-papiers", "Historique local, si activé", "▤", ": "),
        ("Fenêtres", "Revenir à une fenêtre ouverte", "▣", "~ "),
        ("Symboles", "Copier un caractère spécial", "◇", ". "),
        (
            "Système",
            "Paramètres Windows et actions rapides",
            "⏻",
            "! ",
        ),
    ]
    .into_iter()
    .map(|(title, sub, glyph, target)| {
        Item::new(
            format!("nav:{target}"),
            title,
            sub,
            glyph,
            Kind::Navigation,
            target,
        )
    })
    .collect()
}

unsafe fn update_results(s: &mut State) {
    if s.edit.is_null() {
        return;
    }
    let len = GetWindowTextLengthW(s.edit).max(0) as usize;
    let mut text = vec![0u16; len + 1];
    GetWindowTextW(s.edit, text.as_mut_ptr(), text.len() as i32);
    s.query = String::from_utf16_lossy(&text[..len]);
    let start = Instant::now();
    let trimmed = s.query.trim();
    let prefix = trimmed.chars().next().unwrap_or('\0');
    let body = if "@/=?>:.~!;".contains(prefix) && prefix != '\0' {
        trimmed.get(prefix.len_utf8()..).unwrap_or("").trim()
    } else {
        trimmed
    };
    let limit = s.settings.max_results.max(1);
    let weights = &s.weights;
    s.items = match prefix {
        '\0' | ';' => {
            s.section = "EXPLORER".into();
            if body.is_empty() {
                navigation()
            } else {
                core::rank(&navigation(), body, 9, weights)
            }
        }
        '=' => {
            s.section = "CALCULATRICE".into();
            match core::calculate(body) {
                Some(result) => vec![Item::new(
                    "calc",
                    result.clone(),
                    format!("{body} · Entrée pour copier"),
                    "=",
                    Kind::Calculator,
                    result,
                )],
                None if body.is_empty() => vec![Item::new(
                    "example",
                    "Essaie = (12 + 8) × 3",
                    "Opérateurs + - * / ^ %, fonctions sqrt, sin, cos",
                    "=",
                    Kind::Navigation,
                    "= (12 + 8) * 3",
                )],
                None => vec![],
            }
        }
        '?' => {
            s.section = "RECHERCHE WEB".into();
            if body.is_empty() {
                vec![]
            } else {
                let url = s
                    .settings
                    .web_search_template
                    .replace("{query}", &encode(body));
                vec![Item::new(
                    "web",
                    format!("Rechercher « {body} »"),
                    "Ouvrir dans le navigateur par défaut",
                    "⌕",
                    Kind::Web,
                    url,
                )]
            }
        }
        '>' => {
            s.section = "COMMANDES".into();
            let commands: Vec<Item> = s
                .settings
                .commands
                .iter()
                .map(|c| {
                    let mut i = Item::new(
                        format!("command:{}", c.name),
                        &c.name,
                        &c.file_name,
                        "›",
                        Kind::Command,
                        &c.file_name,
                    );
                    i.arguments = c.arguments.clone();
                    i
                })
                .collect();
            let mut found = core::rank(&commands, body, limit, weights);
            if !body.is_empty() && found.len() < limit {
                let mut cmd = Item::new(
                    "shell",
                    format!("Exécuter : {body}"),
                    "Commande Windows · exécutée uniquement avec Entrée",
                    "›",
                    Kind::Command,
                    "cmd.exe",
                );
                cmd.arguments = format!("/D /C {body}");
                found.push(cmd);
            }
            found
        }
        ':' => {
            s.section = "PRESSE-PAPIERS".into();
            if !s.settings.clipboard_enabled {
                vec![Item::new(
                    "clipboard-settings",
                    "Activer l'historique du presse-papiers",
                    "Désactivé par défaut · conservé uniquement en mémoire",
                    "▤",
                    Kind::System,
                    "settings",
                )]
            } else {
                core::rank(&s.clipboard.items(), body, limit, weights)
            }
        }
        '.' => {
            s.section = "SYMBOLES".into();
            let items: Vec<Item> = [
                ("→", "flèche droite"),
                ("←", "flèche gauche"),
                ("✓", "coche"),
                ("×", "multiplication"),
                ("—", "tiret cadratin"),
                ("€", "euro"),
                ("°", "degré"),
                ("π", "pi"),
                ("©", "copyright"),
                ("∞", "infini"),
                ("♥", "cœur"),
                ("…", "points de suspension"),
            ]
            .into_iter()
            .map(|(symbol, title)| {
                Item::new(
                    format!("symbol:{symbol}"),
                    format!("{symbol}  {title}"),
                    "Entrée pour copier",
                    symbol,
                    Kind::Symbol,
                    symbol,
                )
            })
            .collect();
            core::rank(&items, body, limit, weights)
        }
        '~' => {
            s.section = "FENÊTRES OUVERTES".into();
            core::rank(&native::windows(), body, limit, weights)
        }
        '!' => {
            s.section = "SYSTÈME".into();
            core::rank(
                &system_items(),
                body,
                if body.is_empty() { 20 } else { limit },
                weights,
            )
        }
        _ => {
            s.section = match prefix {
                '@' => "APPLICATIONS",
                '/' => "FICHIERS",
                _ => "RÉSULTATS",
            }
            .into();
            let index = s.index.lock().unwrap_or_else(|e| e.into_inner());
            if prefix == '@' || prefix == '/' {
                core::rank_filtered(&index, body, limit, weights, |i| {
                    if prefix == '@' {
                        i.kind == Kind::Application
                    } else {
                        matches!(i.kind, Kind::File | Kind::Folder)
                    }
                })
            } else {
                core::rank(&index, body, limit, weights)
            }
        }
    };
    s.selected = 0;
    s.timing = format!("{:.1} ms", start.elapsed().as_secs_f64() * 1000.);
    InvalidateRect(s.hwnd, null(), 0);
}
fn system_items() -> Vec<Item> {
    [
        (
            "Super Menu · Paramètres",
            "Personnaliser ton menu",
            "settings",
        ),
        (
            "Paramètres Windows",
            "Ouvrir les réglages du système",
            "ms-settings:",
        ),
        ("Bluetooth", "Gérer tes appareils", "ms-settings:bluetooth"),
        ("Son", "Sorties et entrées audio", "ms-settings:sound"),
        (
            "Windows Update",
            "Mises à jour Windows",
            "ms-settings:windowsupdate",
        ),
        (
            "Gestionnaire des tâches",
            "Voir les processus",
            "taskmgr.exe",
        ),
        ("Verrouiller la session", "Verrouiller ce PC", "lock"),
        (
            "Actualiser l'index",
            "Recharger applications et fichiers",
            "refresh",
        ),
        (
            "Effacer le presse-papiers de Super Menu",
            "Effacer uniquement l'historique en mémoire",
            "clear-clipboard",
        ),
        (
            "Réinitialiser le classement",
            "Effacer les compteurs locaux de lancement",
            "reset-usage",
        ),
        (
            "Quitter Super Menu",
            "Arrêter le menu et son raccourci",
            "quit",
        ),
    ]
    .into_iter()
    .map(|(title, sub, target)| {
        Item::new(
            format!("system:{target}"),
            title,
            sub,
            "⏻",
            Kind::System,
            target,
        )
    })
    .collect()
}
fn encode(text: &str) -> String {
    let mut result = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            result.push(b as char);
        } else {
            result.push_str(&format!("%{b:02X}"));
        }
    }
    result
}

unsafe fn execute(s: &mut State, parent: bool) {
    let Some(item) = s.items.get(s.selected).cloned() else {
        return;
    };
    let result = match item.kind {
        Kind::Navigation => {
            set_query(s, &item.target);
            return;
        }
        Kind::Calculator | Kind::Clipboard | Kind::Symbol => copy_text(s.hwnd, &item.target),
        Kind::Window => {
            hide(s);
            native::activate_window(&item.target)
        }
        Kind::System => match item.target.as_str() {
            "settings" => {
                settings(s);
                return;
            }
            "refresh" => {
                reindex(s);
                return;
            }
            "quit" => {
                PostQuitMessage(0);
                return;
            }
            "lock" => {
                hide(s);
                native::lock_workstation()
            }
            "clear-clipboard" => {
                s.clipboard.clear();
                update_results(s);
                return;
            }
            "reset-usage" => {
                s.usage.reset();
                s.weights.clear();
                update_results(s);
                return;
            }
            _ => native::launch(&item.target, &item.arguments),
        },
        Kind::File if parent => {
            native::launch("explorer.exe", &format!("/select,\"{}\"", item.target))
        }
        _ => native::launch(&item.target, &item.arguments),
    };
    match result {
        Ok(()) => {
            hide(s);
            if matches!(
                item.kind,
                Kind::Application | Kind::File | Kind::Folder | Kind::Command
            ) && item.id != "shell"
            {
                s.usage.record(&item.id);
                let index = s.index.lock().unwrap_or_else(|e| e.into_inner());
                s.weights = s.usage.snapshot_for(&index);
            }
        }
        Err(error) => {
            s.settings_open = true;
            show_error(&error);
            s.settings_open = false;
            finish_modal(s);
            SetFocus(s.edit);
        }
    }
}

unsafe fn copy_text(hwnd: HWND, text: &str) -> Result<(), String> {
    if OpenClipboard(hwnd) == 0 {
        return Err("Le presse-papiers est occupé. Réessaie.".into());
    }
    let value = wide(text);
    let memory = GlobalAlloc(GMEM_MOVEABLE, value.len() * 2);
    if memory.is_null() {
        CloseClipboard();
        return Err("Mémoire insuffisante.".into());
    }
    let ptr = GlobalLock(memory);
    if ptr.is_null() {
        GlobalFree(memory);
        CloseClipboard();
        return Err("Accès au presse-papiers impossible.".into());
    }
    std::ptr::copy_nonoverlapping(value.as_ptr(), ptr as *mut u16, value.len());
    GlobalUnlock(memory);
    EmptyClipboard();
    if SetClipboardData(CF_UNICODETEXT, memory).is_null() {
        GlobalFree(memory);
        CloseClipboard();
        return Err("Copie impossible.".into());
    }
    CloseClipboard();
    Ok(())
}

unsafe fn reindex(s: &mut State) {
    if s.indexing.swap(true, Ordering::AcqRel) {
        s.pending_refresh = true;
        return;
    }
    s.status = "Indexation en arrière-plan…".into();
    InvalidateRect(s.hwnd, null(), 0);
    let index = s.index.clone();
    let busy = s.indexing.clone();
    let settings = s.settings.clone();
    let hwnd = s.hwnd as usize;
    std::thread::spawn(move || {
        if index.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
            let cache = catalog::load_cache();
            if !cache.is_empty() {
                *index.lock().unwrap_or_else(|e| e.into_inner()) = cache;
                unsafe {
                    PostMessageW(hwnd as HWND, INDEX, 0, 0);
                }
            }
        }
        let apps = native::app_catalog();
        {
            let mut index = index.lock().unwrap_or_else(|e| e.into_inner());
            let previous_files: Vec<Item> = index
                .iter()
                .filter(|i| matches!(i.kind, Kind::File | Kind::Folder))
                .cloned()
                .collect();
            *index = apps.clone();
            index.extend(previous_files);
        }
        unsafe {
            PostMessageW(hwnd as HWND, INDEX, 0, 0);
        }
        let mut items = apps;
        items.extend(catalog::files(&settings));
        let _ = catalog::save_cache(&items);
        *index.lock().unwrap_or_else(|e| e.into_inner()) = items;
        busy.store(false, Ordering::Release);
        unsafe {
            PostMessageW(hwnd as HWND, INDEX, 0, 0);
        }
    });
}

unsafe fn show(s: &mut State) {
    let foreground = GetForegroundWindow();
    if foreground != s.hwnd {
        s.previous = foreground;
    }
    place(s);
    ShowWindow(s.hwnd, SW_SHOW);
    SetWindowPos(s.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    if native::focus_window(s.hwnd).is_ok() {
        SetFocus(s.edit);
    }
    set_query(s, "");
}
unsafe fn hide(s: &mut State) {
    ShowWindow(s.hwnd, SW_HIDE);
    if !s.previous.is_null() {
        SetForegroundWindow(s.previous);
    }
}
unsafe fn place(s: &mut State) {
    let mut point: POINT = zeroed();
    GetCursorPos(&mut point);
    let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
    let mut info: MONITORINFO = zeroed();
    info.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(monitor, &mut info);
    let area = info.rcWork;
    SetWindowPos(
        s.hwnd,
        null_mut(),
        area.left + 40,
        area.top + 40,
        0,
        0,
        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
    let old_dpi = s.dpi;
    s.dpi = GetDpiForWindow(s.hwnd).max(96) as i32;
    let width = (s.settings.window_width * s.dpi / 96).min(area.right - area.left - 32);
    let height = (680 * s.dpi / 96).min(area.bottom - area.top - 32);
    s.visible_rows = (((height * 96 / s.dpi) - ROW_TOP - 58) / ROW_HEIGHT).max(1) as usize;
    let x = area.left + (area.right - area.left - width) / 2;
    let y = area.top + ((area.bottom - area.top - height) / 4).max(16);
    SetWindowPos(s.hwnd, HWND_TOPMOST, x, y, width, height, SWP_NOACTIVATE);
    if old_dpi != s.dpi {
        rebuild_fonts(s);
    }
    layout_edit(s);
}
unsafe fn layout_edit(s: &State) {
    if s.edit.is_null() {
        return;
    }
    let mut rect: RECT = zeroed();
    GetClientRect(s.hwnd, &mut rect);
    MoveWindow(
        s.edit,
        68 * s.dpi / 96,
        73 * s.dpi / 96,
        (rect.right - 240 * s.dpi / 96).max(100),
        34 * s.dpi / 96,
        1,
    );
}
unsafe fn rebuild_fonts(s: &mut State) {
    for font in s.fonts {
        if !font.is_null() {
            DeleteObject(font);
        }
    }
    for (i, (size, weight)) in [(20, 400), (15, 400), (11, 400), (11, 600), (20, 400)]
        .into_iter()
        .enumerate()
    {
        s.fonts[i] = CreateFontW(
            -size * s.dpi / 96,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            wide(if i == 4 {
                "Segoe UI Symbol"
            } else {
                "Segoe UI"
            })
            .as_ptr(),
        );
    }
    if !s.edit.is_null() {
        SendMessageW(s.edit, WM_SETFONT, s.fonts[0] as usize, 1);
    }
    if !s.edit_bg.is_null() {
        DeleteObject(s.edit_bg);
    }
    s.edit_bg = CreateSolidBrush(s.palette.panel);
}

#[allow(clippy::too_many_arguments)] // Mirrors DrawText's native geometry and style inputs.
unsafe fn text(
    dc: HDC,
    font: HFONT,
    color: u32,
    value: &str,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    right: bool,
) {
    let old = SelectObject(dc, font);
    SetTextColor(dc, color);
    SetBkMode(dc, TRANSPARENT as i32);
    let mut rect = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    let value = wide(value);
    DrawTextW(
        dc,
        value.as_ptr(),
        -1,
        &mut rect,
        DT_SINGLELINE
            | DT_VCENTER
            | DT_END_ELLIPSIS
            | DT_NOPREFIX
            | if right { DT_RIGHT } else { DT_LEFT },
    );
    SelectObject(dc, old);
}
unsafe fn rect(dc: HDC, c: u32, x: i32, y: i32, w: i32, h: i32, radius: i32) {
    let brush = CreateSolidBrush(c);
    let old = SelectObject(dc, brush);
    let pen = SelectObject(dc, GetStockObject(NULL_PEN));
    if radius > 0 {
        RoundRect(dc, x, y, x + w, y + h, radius, radius);
    } else {
        PatBlt(dc, x, y, w, h, PATCOPY);
    }
    SelectObject(dc, old);
    SelectObject(dc, pen);
    DeleteObject(brush);
}
unsafe fn paint(s: &State, dc: HDC) {
    let mut bounds: RECT = zeroed();
    GetClientRect(s.hwnd, &mut bounds);
    let scale = |n: i32| n * s.dpi / 96;
    let w = bounds.right * 96 / s.dpi;
    let h = bounds.bottom * 96 / s.dpi;
    rect(dc, s.palette.bg, 0, 0, bounds.right, bounds.bottom, 0);
    for row in 0..3 {
        for col in 0..3 {
            rect(
                dc,
                if row == 1 && col == 1 {
                    s.palette.text
                } else {
                    s.palette.accent
                },
                scale(26 + col * 6),
                scale(26 + row * 6),
                scale(4),
                scale(4),
                0,
            );
        }
    }
    text(
        dc,
        s.fonts[3],
        s.palette.text,
        "SUPER MENU",
        scale(56),
        scale(20),
        scale(104),
        scale(28),
        false,
    );
    text(
        dc,
        s.fonts[2],
        s.palette.muted,
        "/   À portée de clavier",
        scale(162),
        scale(20),
        scale(250),
        scale(28),
        false,
    );
    text(
        dc,
        s.fonts[4],
        s.palette.muted,
        "⚙",
        scale(w - 85),
        scale(17),
        scale(24),
        scale(30),
        false,
    );
    text(
        dc,
        s.fonts[0],
        s.palette.muted,
        "×",
        scale(w - 49),
        scale(17),
        scale(25),
        scale(30),
        false,
    );
    rect(
        dc,
        s.palette.line,
        scale(22),
        scale(62),
        scale(w - 44),
        scale(58),
        scale(12),
    );
    rect(
        dc,
        s.palette.panel,
        scale(23),
        scale(63),
        scale(w - 46),
        scale(56),
        scale(10),
    );
    text(
        dc,
        s.fonts[4],
        s.palette.accent,
        "⌕",
        scale(39),
        scale(72),
        scale(26),
        scale(36),
        false,
    );
    text(
        dc,
        s.fonts[2],
        s.palette.muted,
        &s.settings.hotkey.replace('+', " + "),
        scale(w - 165),
        scale(78),
        scale(126),
        scale(25),
        true,
    );
    for (label, x, width) in [
        ("Tout", 28, 48),
        ("Applications", 89, 117),
        ("Fichiers", 224, 77),
        ("Web", 322, 48),
        ("Modules  ;", 389, 112),
    ] {
        text(
            dc,
            s.fonts[2],
            if (label == "Tout" && !s.query.starts_with(['@', '/', '?']))
                || (label == "Applications" && s.query.starts_with('@'))
                || (label == "Fichiers" && s.query.starts_with('/'))
                || (label == "Web" && s.query.starts_with('?'))
            {
                s.palette.accent
            } else {
                s.palette.muted
            },
            label,
            scale(x),
            scale(128),
            scale(width),
            scale(25),
            false,
        );
    }
    text(
        dc,
        s.fonts[3],
        s.palette.muted,
        &s.section,
        scale(29),
        scale(159),
        scale(w - 150),
        scale(16),
        false,
    );
    text(
        dc,
        s.fonts[2],
        s.palette.muted,
        &s.timing,
        scale(w - 120),
        scale(159),
        scale(91),
        scale(16),
        true,
    );
    let offset = s.selected.saturating_sub(s.visible_rows.saturating_sub(1));
    for (row, item) in s.items.iter().skip(offset).take(s.visible_rows).enumerate() {
        let y = ROW_TOP + row as i32 * ROW_HEIGHT;
        if row + offset == s.selected {
            rect(
                dc,
                s.palette.selected,
                scale(23),
                scale(y + 1),
                scale(w - 46),
                scale(46),
                scale(10),
            );
        }
        rect(
            dc,
            s.palette.panel,
            scale(36),
            scale(y + 8),
            scale(31),
            scale(31),
            scale(10),
        );
        text(
            dc,
            s.fonts[4],
            s.palette.accent,
            &item.glyph,
            scale(43),
            scale(y + 8),
            scale(23),
            scale(30),
            false,
        );
        text(
            dc,
            s.fonts[1],
            if row + offset == s.selected {
                s.palette.accent
            } else {
                s.palette.text
            },
            &item.title,
            scale(83),
            scale(y + 4),
            scale(w - 134),
            scale(23),
            false,
        );
        text(
            dc,
            s.fonts[2],
            s.palette.muted,
            &item.subtitle,
            scale(83),
            scale(y + 26),
            scale(w - 134),
            scale(16),
            false,
        );
        text(
            dc,
            s.fonts[0],
            s.palette.muted,
            "›",
            scale(w - 49),
            scale(y + 8),
            scale(19),
            scale(31),
            true,
        );
    }
    if s.items.is_empty() {
        let y = ROW_TOP + 60;
        text(
            dc,
            s.fonts[0],
            s.palette.text,
            "Aucun résultat",
            scale(28),
            scale(y),
            scale(w - 56),
            scale(40),
            false,
        );
        text(
            dc,
            s.fonts[2],
            s.palette.muted,
            if s.indexing.load(Ordering::Relaxed) {
                "L'index se construit en arrière-plan…"
            } else {
                "Essaie un autre mot ou change de module avec ;"
            },
            scale(28),
            scale(y + 42),
            scale(w - 56),
            scale(30),
            false,
        );
    }
    rect(
        dc,
        s.palette.line,
        scale(23),
        scale(h - 48),
        scale(w - 46),
        scale(1),
        0,
    );
    text(
        dc,
        s.fonts[2],
        s.palette.muted,
        "↑ ↓ naviguer   ↵ ouvrir   Échap fermer   F1 aide",
        scale(28),
        scale(h - 39),
        scale(347),
        scale(25),
        false,
    );
    text(
        dc,
        s.fonts[2],
        s.palette.muted,
        &s.status,
        scale(377),
        scale(h - 39),
        scale(w - 405),
        scale(25),
        true,
    );
}

unsafe fn make_icon(instance: HINSTANCE) -> HICON {
    let mut pixels = [0u8; 32 * 32 * 4];
    let mut mask = [0u8; 128];
    for y in 0..32 {
        for x in 0..32 {
            let n = (y * 32 + x) * 4;
            pixels[n] = 27;
            pixels[n + 1] = 23;
            pixels[n + 2] = 22;
            pixels[n + 3] = 255;
        }
    }
    for row in 0..3 {
        for col in 0..3 {
            for y in (7 + row * 7)..(12 + row * 7) {
                for x in (7 + col * 7)..(12 + col * 7) {
                    let n = (y * 32 + x) * 4;
                    pixels[n] = 255;
                    pixels[n + 1] = 140;
                    pixels[n + 2] = 154;
                }
            }
        }
    }
    let icon = CreateIcon(instance, 32, 32, 1, 32, mask.as_mut_ptr(), pixels.as_ptr());
    if icon.is_null() {
        LoadIconW(null_mut(), IDI_APPLICATION)
    } else {
        icon
    }
}
unsafe fn tray(s: &State, operation: u32) {
    if s.smoke {
        return;
    }
    let mut data: NOTIFYICONDATAW = zeroed();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = s.hwnd;
    data.uID = 1;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = TRAY;
    data.hIcon = s.icon;
    let tip = wide("Super Menu · Win + Espace");
    data.szTip[..tip.len()].copy_from_slice(&tip);
    Shell_NotifyIconW(operation, &data);
}

// Captures only this application's rendering, never the desktop or other apps.
unsafe fn capture_bmp(s: &State) {
    let args: Vec<String> = std::env::args().collect();
    let Some(pos) = args.iter().position(|a| a == "--snapshot") else {
        return;
    };
    let Some(path) = args.get(pos + 1) else {
        return;
    };
    let mut bounds: RECT = zeroed();
    GetClientRect(s.hwnd, &mut bounds);
    let width = bounds.right;
    let height = bounds.bottom;
    let dc = GetDC(s.hwnd);
    let mem = CreateCompatibleDC(dc);
    let mut info: BITMAPINFO = zeroed();
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = width;
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let mut pixels = null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
    if !bitmap.is_null() {
        let old = SelectObject(mem, bitmap);
        paint(s, mem);
        let text_value = if s.query.is_empty() {
            "Où veux-tu aller ?"
        } else {
            &s.query
        };
        text(
            mem,
            s.fonts[0],
            if s.query.is_empty() {
                s.palette.muted
            } else {
                s.palette.text
            },
            text_value,
            68 * s.dpi / 96,
            73 * s.dpi / 96,
            width - 240 * s.dpi / 96,
            34 * s.dpi / 96,
            false,
        );
        let bytes = std::slice::from_raw_parts(pixels as *const u8, (width * height * 4) as usize);
        let mut out = Vec::with_capacity(bytes.len() + 54);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&((bytes.len() + 54) as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&(-height).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        out.extend_from_slice(bytes);
        let _ = std::fs::write(path, out);
        SelectObject(mem, old);
        DeleteObject(bitmap);
    }
    DeleteDC(mem);
    ReleaseDC(s.hwnd, dc);
}

// A native modal settings window. No embedded browser and no UI framework runtime.
unsafe fn settings(s: &mut State) {
    if s.settings_open {
        return;
    }
    s.settings_open = true;
    if let Some(settings) = settings_dialog(s) {
        s.settings = settings;
        s.palette = Palette::from(&s.settings);
        s.clipboard.clear();
        if let Err(e) = config::save(&s.settings) {
            show_error(&format!("Configuration non enregistrée : {e}"));
        }
        s.hotkey.take();
        match native::install_hotkey(s.hwnd, &s.settings.hotkey) {
            Ok(h) => s.hotkey = Some(h),
            Err(e) => show_error(&e),
        }
        rebuild_fonts(s);
        place(s);
        reindex(s);
        update_results(s);
        s.watcher.take();
        s.watcher = native::watch(s.hwnd, &s.settings.file_roots);
    }
    s.settings_open = false;
    finish_modal(s);
    SetForegroundWindow(s.hwnd);
    SetFocus(s.edit);
}

unsafe fn smoke_checks(s: &mut State) {
    set_query(s, "");
    s.checks.push(("all_providers".into(), s.items.len() == 9));
    set_query(s, "@ ");
    s.checks.push((
        "applications".into(),
        !s.items.is_empty() && s.items.iter().all(|i| i.kind == Kind::Application),
    ));
    set_query(s, "= (12 + 8) * 3");
    s.checks.push((
        "calculator".into(),
        s.items.first().is_some_and(|i| i.target == "60"),
    ));
    set_query(s, "? c++ é");
    s.checks.push((
        "web_encoding".into(),
        s.items
            .first()
            .is_some_and(|i| i.target.contains("c%2B%2B%20%C3%A9")),
    ));
    set_query(s, "> echo hello");
    s.checks.push((
        "explicit_command".into(),
        s.items
            .iter()
            .any(|i| i.target == "cmd.exe" && i.arguments == "/D /C echo hello"),
    ));
    set_query(s, ".");
    key(s, VK_DOWN as u32);
    s.checks
        .push(("keyboard_selection".into(), s.selected == 1));
    key(s, VK_ESCAPE as u32);
    s.checks.push(("escape_clears".into(), s.query.is_empty()));
    set_query(s, "!");
    s.checks.push((
        "system_actions".into(),
        s.items.iter().any(|i| i.target == "quit"),
    ));
    set_query(s, "");
}

unsafe fn finish_modal(s: &State) {
    let deferred = DEFERRED.with(|d| d.replace(0));
    PostMessageW(s.hwnd, INDEX, 0, 0);
    if deferred & 2 != 0 {
        PostMessageW(s.hwnd, REFRESH, 0, 0);
    }
}

struct SettingsDialog {
    source: config::Settings,
    result: Option<config::Settings>,
    done: bool,
    controls: Vec<HWND>,
}
unsafe fn settings_dialog(s: &State) -> Option<config::Settings> {
    let instance = GetModuleHandleW(null());
    let class = wide("SuperMenuSettings");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(settings_proc),
        hInstance: instance,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: (COLOR_WINDOW + 1) as _,
        lpszClassName: class.as_ptr(),
        ..zeroed()
    };
    RegisterClassW(&wc);
    let mut dialog = Box::new(SettingsDialog {
        source: s.settings.clone(),
        result: None,
        done: false,
        controls: vec![],
    });
    let mut parent: RECT = zeroed();
    GetWindowRect(s.hwnd, &mut parent);
    let scale = |value: i32| value * s.dpi / 96;
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_DLGMODALFRAME,
        class.as_ptr(),
        wide("Super Menu · Paramètres").as_ptr(),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
        parent.left + scale(20),
        parent.top + scale(15),
        scale(640),
        scale(610),
        s.hwnd,
        null_mut(),
        instance,
        (&mut *dialog as *mut SettingsDialog).cast(),
    );
    if hwnd.is_null() {
        return None;
    }
    let control =
        |class: &str, label: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: usize| {
            let child = CreateWindowExW(
                if class == "EDIT" { WS_EX_CLIENTEDGE } else { 0 },
                wide(class).as_ptr(),
                wide(label).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                scale(x),
                scale(y),
                scale(w),
                scale(h),
                hwnd,
                id as _,
                instance,
                null(),
            );
            SendMessageW(child, WM_SETFONT, s.fonts[1] as usize, 1);
            child
        };
    control(
        "STATIC",
        "Une interface native. Un index local. Tes réglages.",
        0,
        22,
        16,
        580,
        27,
        0,
    );
    control("STATIC", "Raccourci", 0, 22, 57, 155, 23, 0);
    let hotkey = control(
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP,
        187,
        54,
        260,
        150,
        201,
    );
    for key in ["Win+Space", "Alt+Space", "Ctrl+Space"] {
        SendMessageW(hotkey, CB_ADDSTRING, 0, wide(key).as_ptr() as isize);
    }
    if !["Win+Space", "Alt+Space", "Ctrl+Space"].contains(&s.settings.hotkey.as_str()) {
        SendMessageW(
            hotkey,
            CB_ADDSTRING,
            0,
            wide(&s.settings.hotkey).as_ptr() as isize,
        );
    }
    SendMessageW(
        hotkey,
        CB_SETCURSEL,
        match s.settings.hotkey.as_str() {
            "Alt+Space" => 1,
            "Ctrl+Space" => 2,
            "Win+Space" => 0,
            _ => 3,
        },
        0,
    );
    control("STATIC", "Thème", 0, 22, 95, 155, 23, 0);
    let theme = control(
        "COMBOBOX",
        "",
        CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP,
        187,
        92,
        260,
        120,
        202,
    );
    for label in ["Sombre", "Clair"] {
        SendMessageW(theme, CB_ADDSTRING, 0, wide(label).as_ptr() as isize);
    }
    SendMessageW(
        theme,
        CB_SETCURSEL,
        if s.settings.theme == "light" { 1 } else { 0 },
        0,
    );
    control("STATIC", "Couleur d'accent", 0, 22, 134, 155, 23, 0);
    let accent = control(
        "EDIT",
        &s.settings.accent_color,
        ES_AUTOHSCROLL as u32 | WS_TABSTOP,
        187,
        129,
        130,
        28,
        203,
    );
    let clip = control(
        "BUTTON",
        "Historique du presse-papiers (en mémoire uniquement)",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        22,
        175,
        580,
        26,
        204,
    );
    SendMessageW(
        clip,
        BM_SETCHECK,
        if s.settings.clipboard_enabled {
            BST_CHECKED as usize
        } else {
            BST_UNCHECKED as usize
        },
        0,
    );
    let startup = control(
        "BUTTON",
        "Lancer Super Menu à l'ouverture de session",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        22,
        211,
        580,
        26,
        205,
    );
    SendMessageW(
        startup,
        BM_SETCHECK,
        if native::startup_enabled() {
            BST_CHECKED as usize
        } else {
            0
        },
        0,
    );
    control(
        "STATIC",
        "Dossiers indexés (un dossier par ligne)",
        0,
        22,
        253,
        580,
        25,
        0,
    );
    let roots = control(
        "EDIT",
        &s.settings.file_roots.join("\r\n"),
        ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | WS_VSCROLL | WS_TABSTOP,
        22,
        283,
        580,
        98,
        206,
    );
    control(
        "STATIC",
        "Recherche web · {query} sera remplacé",
        0,
        22,
        398,
        580,
        24,
        0,
    );
    let web = control(
        "EDIT",
        &s.settings.web_search_template,
        ES_AUTOHSCROLL as u32 | WS_TABSTOP,
        22,
        427,
        580,
        28,
        207,
    );
    control(
        "BUTTON",
        "Éditer le JSON",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        22,
        490,
        155,
        34,
        208,
    );
    control(
        "BUTTON",
        "Annuler",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        367,
        490,
        105,
        34,
        209,
    );
    control(
        "BUTTON",
        "Enregistrer",
        BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
        485,
        490,
        117,
        34,
        210,
    );
    dialog.controls = vec![hotkey, theme, accent, clip, startup, roots, web];
    EnableWindow(s.hwnd, 0);
    ShowWindow(hwnd, SW_SHOW);
    SetForegroundWindow(hwnd);
    SetFocus(hotkey);
    let mut msg: MSG = zeroed();
    while !dialog.done {
        if GetMessageW(&mut msg, null_mut(), 0, 0) <= 0 {
            dialog.done = true;
            PostQuitMessage(0);
            break;
        }
        if IsDialogMessageW(hwnd, &msg) == 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    if IsWindow(hwnd) != 0 {
        DestroyWindow(hwnd);
    }
    EnableWindow(s.hwnd, 1);
    dialog.result.take()
}
unsafe extern "system" fn settings_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_NCCREATE {
        let c = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, c.lpCreateParams as isize);
        return 1;
    }
    let Some(d) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsDialog).as_mut() else {
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    match msg {
        WM_COMMAND => match wp & 0xffff {
            208 => {
                let path = config::settings_path();
                if !path.exists() {
                    let _ = config::save(&d.source);
                }
                let _ = native::launch("notepad.exe", &format!("\"{}\"", path.display()));
                0
            }
            209 | 2 => {
                d.done = true;
                0
            }
            210 | 1 => {
                if d.controls.len() < 7 {
                    return 0;
                }
                let mut settings = d.source.clone();
                settings.hotkey = control_text(d.controls[0]);
                settings.theme = if SendMessageW(d.controls[1], CB_GETCURSEL, 0, 0) == 1 {
                    "light"
                } else {
                    "dark"
                }
                .into();
                settings.accent_color = control_text(d.controls[2]);
                if settings.accent_color.len() != 7
                    || !settings.accent_color.starts_with('#')
                    || u32::from_str_radix(&settings.accent_color[1..], 16).is_err()
                {
                    MessageBoxW(
                        hwnd,
                        wide("Utilise une couleur hexadécimale, par exemple #9A8CFF.").as_ptr(),
                        wide("Couleur").as_ptr(),
                        MB_OK | MB_ICONWARNING,
                    );
                    return 0;
                }
                settings.clipboard_enabled =
                    SendMessageW(d.controls[3], BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
                settings.file_roots = control_text(d.controls[5])
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(String::from)
                    .collect();
                settings.web_search_template = control_text(d.controls[6]);
                if !settings.web_search_template.contains("{query}")
                    || !settings.web_search_template.starts_with("https://")
                {
                    MessageBoxW(hwnd,wide("L'adresse de recherche doit commencer par https:// et contenir {query}.").as_ptr(),wide("Recherche web").as_ptr(),MB_OK|MB_ICONWARNING);
                    return 0;
                }
                if let Err(e) = native::set_startup(
                    SendMessageW(d.controls[4], BM_GETCHECK, 0, 0) == BST_CHECKED as isize,
                ) {
                    show_error(&e);
                    return 0;
                }
                settings.normalize();
                d.result = Some(settings);
                d.done = true;
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        },
        WM_CLOSE => {
            d.done = true;
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
unsafe fn control_text(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd).max(0) as usize;
    let mut value = vec![0u16; len + 1];
    GetWindowTextW(hwnd, value.as_mut_ptr(), value.len() as i32);
    String::from_utf16_lossy(&value[..len])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn web_query_encoding() {
        assert_eq!(encode("a&b é"), "a%26b%20%C3%A9");
    }
    #[test]
    fn home_providers() {
        let menu = navigation();
        assert_eq!(menu.len(), 9);
        assert!(menu.iter().all(|i| i.kind == Kind::Navigation));
    }
}
