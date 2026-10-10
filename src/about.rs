//! A reusable, modeless About window. Hardware collection starts only when shown.

use crate::i18n::{fmt, tr};
use crate::{native::wide, system_info};
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{SetScrollInfo, SetScrollPos, DRAWITEMSTRUCT, ODS_FOCUS, ODS_SELECTED},
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{GetFocus, GetKeyState, SetFocus, VK_ESCAPE, VK_SHIFT, VK_TAB},
        Shell::ShellExecuteW,
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "SuperMenuAboutWindow";
const CONTENT_CLASS: &str = "SuperMenuAboutContent";
const READY: u32 = WM_APP + 60;
const READY_TIMER: usize = 61;
const GITHUB: usize = 3101;
const CLOSE: usize = 3102;
const REPOSITORY: &str = "https://github.com/roukmoute/SuperMenu";
const STATIC_NOPREFIX: u32 = 0x0080;

pub struct Context {
    pub theme: String,
    pub accent_color: String,
    pub hotkey: String,
    pub applications: usize,
    pub files: usize,
    pub index_items: usize,
    pub app_uptime: Duration,
    pub external_links: bool,
}

#[derive(Clone, Copy)]
struct Palette {
    background: COLORREF,
    panel: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    border: COLORREF,
    accent: COLORREF,
}

impl Palette {
    fn from(context: &Context) -> Self {
        let accent = u32::from_str_radix(context.accent_color.trim().trim_start_matches('#'), 16)
            .unwrap_or(0x9a8cff);
        let accent = color((accent >> 16) & 255, (accent >> 8) & 255, accent & 255);
        if context.theme.eq_ignore_ascii_case("light") {
            Self {
                background: color(247, 247, 251),
                panel: color(236, 236, 244),
                text: color(30, 28, 43),
                muted: color(103, 101, 119),
                border: color(213, 211, 224),
                accent,
            }
        } else {
            Self {
                background: color(17, 18, 22),
                panel: color(25, 26, 33),
                text: color(241, 240, 247),
                muted: color(148, 147, 165),
                border: color(45, 44, 57),
                accent,
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Style {
    font: usize,
    muted: bool,
    accent: bool,
    panel: bool,
}

struct RowControls {
    label: HWND,
    value: HWND,
    label_text: String,
    text: String,
}

struct Panel {
    heading: HWND,
    loading: HWND,
    rows: Vec<RowControls>,
    bounds: RECT,
}

struct Placement {
    window: HWND,
    bounds: RECT,
}

struct WakeTarget {
    window: Mutex<Option<usize>>,
    alive: AtomicBool,
}

impl WakeTarget {
    fn wake(&self) {
        let target = self
            .window
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(target) = *target {
            unsafe { PostMessageW(target as HWND, READY, 0, 0) };
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

struct State {
    hwnd: HWND,
    content: HWND,
    parent: HWND,
    context: Context,
    palette: Palette,
    fonts: [HFONT; 3],
    brushes: [HBRUSH; 2],
    dpi: i32,
    compact_font: bool,
    left: Vec<HWND>,
    buttons: [HWND; 2],
    panels: Vec<Panel>,
    styles: HashMap<usize, Style>,
    placements: Vec<Placement>,
    logo: RECT,
    scroll: i32,
    scroll_max: i32,
    loaded: bool,
    collecting: bool,
    response: Option<mpsc::Receiver<system_info::Snapshot>>,
    wake: Arc<WakeTarget>,
}

impl State {
    fn rebuild_resources(&mut self, dpi: i32, compact: bool) -> Result<(), String> {
        let mut fonts = [null_mut(); 3];
        for (index, (height, weight)) in [
            (if compact { 12 } else { 13 }, 400),
            (if compact { 14 } else { 16 }, 600),
            (11, 400),
        ]
        .into_iter()
        .enumerate()
        {
            fonts[index] = unsafe {
                CreateFontW(
                    -scale(height, dpi),
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
                    wide("Segoe UI").as_ptr(),
                )
            };
            if fonts[index].is_null() {
                for font in fonts.into_iter().filter(|font| !font.is_null()) {
                    unsafe { DeleteObject(font) };
                }
                return Err(win_error("Création des polices À propos"));
            }
        }
        let brushes = unsafe {
            [
                CreateSolidBrush(self.palette.background),
                CreateSolidBrush(self.palette.panel),
            ]
        };
        if brushes.iter().any(|brush| brush.is_null()) {
            for font in fonts {
                unsafe { DeleteObject(font) };
            }
            for brush in brushes.into_iter().filter(|brush| !brush.is_null()) {
                unsafe { DeleteObject(brush) };
            }
            return Err(win_error("Création du thème À propos"));
        }
        self.dpi = dpi;
        self.compact_font = compact;
        // Replace control fonts before releasing their previous GDI objects.
        for (&window, style) in &self.styles {
            unsafe { SendMessageW(window as HWND, WM_SETFONT, fonts[style.font] as usize, 0) };
        }
        for button in self.buttons.into_iter().filter(|button| !button.is_null()) {
            unsafe { SendMessageW(button, WM_SETFONT, fonts[0] as usize, 0) };
        }
        for font in self.fonts.into_iter().filter(|font| !font.is_null()) {
            unsafe { DeleteObject(font) };
        }
        for brush in self.brushes.into_iter().filter(|brush| !brush.is_null()) {
            unsafe { DeleteObject(brush) };
        }
        self.fonts = fonts;
        self.brushes = brushes;
        Ok(())
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.wake.stop();
        for font in self.fonts.into_iter().filter(|font| !font.is_null()) {
            unsafe { DeleteObject(font) };
        }
        for brush in self.brushes.into_iter().filter(|brush| !brush.is_null()) {
            unsafe { DeleteObject(brush) };
        }
    }
}

/// All window and GDI operations stay on the launcher's UI thread.
pub struct AboutWindow {
    hwnd: HWND,
    state: Box<RefCell<State>>,
}

impl AboutWindow {
    pub fn new(parent: HWND, context: Context) -> Result<Self, String> {
        register_classes()?;
        let dpi = unsafe { GetDpiForWindow(parent) }.max(96) as i32;
        let palette = Palette::from(&context);
        let wake = Arc::new(WakeTarget {
            window: Mutex::new(None),
            alive: AtomicBool::new(true),
        });
        let state = Box::new(RefCell::new(State {
            hwnd: null_mut(),
            content: null_mut(),
            parent,
            context,
            palette,
            fonts: [null_mut(); 3],
            brushes: [null_mut(); 2],
            dpi,
            compact_font: false,
            left: Vec::new(),
            buttons: [null_mut(); 2],
            panels: Vec::new(),
            styles: HashMap::new(),
            placements: Vec::new(),
            logo: unsafe { zeroed() },
            scroll: 0,
            scroll_max: 0,
            loaded: false,
            collecting: false,
            response: None,
            wake,
        }));
        let pointer = &*state as *const RefCell<State>;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_CONTROLPARENT,
                wide(CLASS).as_ptr(),
                wide(&tr("Super Menu · À propos")).as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                scale(1000, dpi),
                scale(720, dpi),
                // The launcher's TOPMOST owner would also make this window topmost.
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                pointer.cast(),
            )
        };
        if hwnd.is_null() {
            return Err(win_error("Création de la fenêtre À propos"));
        }
        let window = Self { hwnd, state };
        unsafe {
            let parent_icon = GetClassLongPtrW(parent, GCLP_HICON);
            if parent_icon != 0 {
                // Borrowed from the main window, whose icon outlives About.
                SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, parent_icon as isize);
                SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, parent_icon as isize);
            }
        }
        {
            let mut state = window.state.borrow_mut();
            state.hwnd = hwnd;
            *state
                .wake
                .window
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(hwnd as usize);
            state.rebuild_resources(dpi, false)?;
            unsafe {
                state.content = CreateWindowExW(
                    WS_EX_CONTROLPARENT,
                    wide(CONTENT_CLASS).as_ptr(),
                    null(),
                    WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_VSCROLL,
                    0,
                    0,
                    1,
                    1,
                    hwnd,
                    null_mut(),
                    GetModuleHandleW(null()),
                    pointer.cast(),
                );
                if state.content.is_null() {
                    return Err(win_error("Création du contenu À propos"));
                }
                create_controls(&mut state)?;
                update_context(&state);
                layout(&mut state);
            }
        }
        Ok(window)
    }

    pub fn show(&mut self, context: Context) {
        {
            let mut state = self.state.borrow_mut();
            state.context = context;
            state.palette = Palette::from(&state.context);
            let dpi = unsafe { GetDpiForWindow(state.parent) }.max(96) as i32;
            let compact = state.compact_font;
            let _ = state.rebuild_resources(dpi, compact);
            unsafe { update_context(&state) };
        }
        let (parent, dpi) = {
            let state = self.state.borrow();
            (state.parent, state.dpi)
        };
        let bounds = unsafe { monitor_placement(parent, dpi) };
        unsafe {
            let dark = i32::from(
                !self
                    .state
                    .borrow()
                    .context
                    .theme
                    .eq_ignore_ascii_case("light"),
            );
            DwmSetWindowAttribute(self.hwnd, 20, (&dark as *const i32).cast(), 4);
            let round = 2i32;
            DwmSetWindowAttribute(self.hwnd, 33, (&round as *const i32).cast(), 4);
            SetWindowPos(
                self.hwnd,
                HWND_NOTOPMOST,
                bounds.left,
                bounds.top,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                SWP_NOACTIVATE,
            );
        }
        {
            let mut state = self.state.borrow_mut();
            state.scroll = 0;
            unsafe { layout(&mut state) };
        }
        unsafe {
            ShowWindow(self.hwnd, SW_SHOW);
            // STARTUPINFO may override a first ShowWindow with SW_HIDE. This
            // window was explicitly requested, so ensure it is actually visible.
            SetWindowPos(
                self.hwnd,
                HWND_NOTOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            SetForegroundWindow(self.hwnd);
            SetFocus(self.state.borrow().buttons[1]);
            InvalidateRect(self.hwnd, null(), 1);
            InvalidateRect(self.state.borrow().content, null(), 1);
        }
        start_collection(&self.state);
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn is_loaded(&self) -> bool {
        self.state.borrow().loaded
    }

    pub fn is_collecting(&self) -> bool {
        self.state.borrow().collecting
    }

    /// Call before TranslateMessage/DispatchMessage in the main message loop.
    pub fn handle_message(&self, message: &MSG) -> bool {
        unsafe { IsWindowVisible(self.hwnd) != 0 && IsDialogMessageW(self.hwnd, message) != 0 }
    }
}

impl Drop for AboutWindow {
    fn drop(&mut self) {
        self.state.borrow().wake.stop();
        unsafe { DestroyWindow(self.hwnd) };
    }
}

fn register_classes() -> Result<(), String> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| unsafe {
            for name in [CLASS, CONTENT_CLASS] {
                let name = wide(name);
                let class = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(window_proc),
                    hInstance: GetModuleHandleW(null()),
                    hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                    lpszClassName: name.as_ptr(),
                    ..zeroed()
                };
                if RegisterClassExW(&class) == 0 {
                    return Err(win_error("Classe de la fenêtre À propos"));
                }
            }
            Ok(())
        })
        .clone()
}

unsafe fn create_static(state: &mut State, text: &str, style: Style) -> Result<HWND, String> {
    let window = CreateWindowExW(
        0,
        wide("STATIC").as_ptr(),
        wide(&clean(text, 512)).as_ptr(),
        WS_CHILD | WS_VISIBLE | STATIC_NOPREFIX,
        0,
        0,
        1,
        1,
        state.content,
        null_mut(),
        GetModuleHandleW(null()),
        null(),
    );
    if window.is_null() {
        return Err(win_error("Texte de la fenêtre À propos"));
    }
    SendMessageW(window, WM_SETFONT, state.fonts[style.font] as usize, 0);
    state.styles.insert(window as usize, style);
    Ok(window)
}

unsafe fn create_controls(state: &mut State) -> Result<(), String> {
    for (text, font, muted) in [
        ("À portée de clavier", 1, false),
        (
            "Un lanceur Windows rapide pour retrouver vos applications, fichiers et outils en quelques touches.",
            0,
            true,
        ),
        ("", 0, false),
        ("", 0, true),
        ("", 0, false),
        ("", 0, false),
        ("", 0, true),
        ("", 0, true),
        ("Natif · Rust · Windows uniquement", 2, true),
    ] {
        let label = create_static(
            state,
            &tr(text),
            Style {
                font,
                muted,
                accent: false,
                panel: false,
            },
        )?;
        state.left.push(label);
    }
    for name in ["Matériel", "Windows", "Activité"] {
        let heading = create_static(
            state,
            &tr(name),
            Style {
                font: 1,
                muted: false,
                accent: true,
                panel: true,
            },
        )?;
        let loading = create_static(
            state,
            &tr("Chargement…"),
            Style {
                font: 0,
                muted: true,
                accent: false,
                panel: true,
            },
        )?;
        state.panels.push(Panel {
            heading,
            loading,
            rows: Vec::new(),
            bounds: zeroed(),
        });
    }
    for (index, (text, id)) in [("GitHub", GITHUB), ("Fermer", CLOSE)]
        .into_iter()
        .enumerate()
    {
        let button = CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide(&tr(text)).as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
            0,
            0,
            1,
            1,
            state.hwnd,
            id as _,
            GetModuleHandleW(null()),
            null(),
        );
        if button.is_null() {
            return Err(win_error("Bouton de la fenêtre À propos"));
        }
        SendMessageW(button, WM_SETFONT, state.fonts[0] as usize, 0);
        state.buttons[index] = button;
    }
    Ok(())
}

unsafe fn update_context(state: &State) {
    let context = &state.context;
    SetWindowTextW(state.hwnd, wide(&tr("Super Menu · À propos")).as_ptr());
    for (index, source) in [
        (0, "À portée de clavier"),
        (1, "Un lanceur Windows rapide pour retrouver vos applications, fichiers et outils en quelques touches."),
        (8, "Natif · Rust · Windows uniquement"),
    ] {
        SetWindowTextW(state.left[index], wide(&tr(source)).as_ptr());
    }
    for (panel, source) in state.panels.iter().zip(["Matériel", "Windows", "Activité"]) {
        SetWindowTextW(panel.heading, wide(&tr(source)).as_ptr());
    }
    for (button, source) in state.buttons.iter().zip(["GitHub", "Fermer"]) {
        SetWindowTextW(*button, wide(&tr(source)).as_ptr());
    }
    for (index, value) in [
        (
            2,
            fmt(
                "Version {version}",
                &[("version", env!("CARGO_PKG_VERSION").into())],
            ),
        ),
        (
            3,
            fmt(
                "Raccourci : {hotkey}",
                &[("hotkey", clean(&context.hotkey, 80))],
            ),
        ),
        (
            4,
            fmt(
                "{count} applications",
                &[("count", context.applications.to_string())],
            ),
        ),
        (
            5,
            fmt(
                "{count} fichiers et dossiers",
                &[("count", context.files.to_string())],
            ),
        ),
        (
            6,
            fmt(
                "{count} entrées dans l’index local",
                &[("count", context.index_items.to_string())],
            ),
        ),
        (
            7,
            fmt(
                "Super Menu ouvert depuis {duration}",
                &[("duration", elapsed(context.app_uptime))],
            ),
        ),
    ] {
        SetWindowTextW(state.left[index], wide(&value).as_ptr());
    }
}

fn start_collection(cell: &RefCell<State>) {
    let wake = {
        let mut state = cell.borrow_mut();
        if state.collecting {
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        state.response = Some(receiver);
        state.collecting = true;
        let wake = Arc::clone(&state.wake);
        let worker_wake = Arc::clone(&wake);
        let result = std::thread::Builder::new()
            .name("supermenu-about".into())
            .spawn(move || {
                if !worker_wake.alive.load(Ordering::Acquire) {
                    return;
                }
                let mut snapshot =
                    std::panic::catch_unwind(system_info::collect).unwrap_or_else(|_| {
                        system_info::Snapshot {
                            hardware: Vec::new(),
                            windows: Vec::new(),
                            activity: Vec::new(),
                        }
                    });
                let executable_size = std::env::current_exe()
                    .ok()
                    .and_then(|path| std::fs::metadata(path).ok())
                    .map(|metadata| {
                        let amount = format!("{:.2}", metadata.len() as f64 / 1_048_576.0);
                        let (whole, fraction) = amount.split_once('.').unwrap_or((&amount, "00"));
                        fmt(
                            "{whole},{fraction} Mio ({bytes} octets)",
                            &[
                                ("whole", whole.into()),
                                ("fraction", fraction.into()),
                                ("bytes", metadata.len().to_string()),
                            ],
                        )
                    })
                    .unwrap_or_else(|| tr("Indisponible"));
                snapshot.activity.push(system_info::Row {
                    label: tr("Exécutable Super Menu"),
                    value: executable_size,
                });
                if worker_wake.alive.load(Ordering::Acquire) && sender.try_send(snapshot).is_ok() {
                    worker_wake.wake();
                }
            });
        if result.is_err() {
            state.collecting = false;
            state.response = None;
            unsafe {
                for panel in &state.panels {
                    SetWindowTextW(panel.loading, wide(&tr("Indisponible")).as_ptr());
                }
            }
        }
        wake
    };
    // Keep the same synchronized target across all reopenings and workers.
    drop(wake);
}

unsafe fn accept_snapshot(state: &mut State) {
    let snapshot = match state
        .response
        .as_ref()
        .and_then(|receiver| receiver.try_recv().ok())
    {
        Some(snapshot) => snapshot,
        None => return,
    };
    state.collecting = false;
    state.response = None;
    for (index, rows) in [snapshot.hardware, snapshot.windows, snapshot.activity]
        .into_iter()
        .enumerate()
    {
        for row in state.panels[index].rows.drain(..) {
            state.styles.remove(&(row.label as usize));
            state.styles.remove(&(row.value as usize));
            DestroyWindow(row.label);
            DestroyWindow(row.value);
        }
        let mut controls = Vec::new();
        for row in rows.into_iter().take(16) {
            let label_text = clean(&row.label, 512);
            let label = create_static(
                state,
                &label_text,
                Style {
                    font: 0,
                    muted: true,
                    accent: false,
                    panel: true,
                },
            );
            let text = clean(&row.value, 512);
            let text = if text.trim().is_empty() {
                tr("Indisponible")
            } else {
                text
            };
            let value = create_static(
                state,
                &text,
                Style {
                    font: 0,
                    muted: false,
                    accent: false,
                    panel: true,
                },
            );
            match (label, value) {
                (Ok(label), Ok(value)) => controls.push(RowControls {
                    label,
                    value,
                    label_text,
                    text,
                }),
                (label, value) => {
                    for control in [label, value].into_iter().flatten() {
                        state.styles.remove(&(control as usize));
                        DestroyWindow(control);
                    }
                }
            }
        }
        let panel = &mut state.panels[index];
        panel.rows = controls;
        SetWindowTextW(panel.loading, wide(&tr("Indisponible")).as_ptr());
        ShowWindow(
            panel.loading,
            if panel.rows.is_empty() {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
    }
    state.loaded = true;
    layout(state);
    InvalidateRect(state.content, null(), 1);
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<State>;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let cell = &*pointer;
    if message == WM_NCDESTROY {
        if let Ok(state) = cell.try_borrow() {
            if hwnd == state.hwnd {
                state.wake.stop();
            }
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    if message == WM_CLOSE || (message == WM_KEYDOWN && wparam == VK_ESCAPE as usize) {
        let main = cell.try_borrow().map_or(hwnd, |state| state.hwnd);
        ShowWindow(main, SW_HIDE);
        return 0;
    }
    if message == WM_COMMAND && (wparam >> 16) & 0xffff == BN_CLICKED as usize {
        match wparam & 0xffff {
            CLOSE | 2 => {
                ShowWindow(hwnd, SW_HIDE);
                return 0;
            }
            GITHUB => {
                if cell
                    .try_borrow()
                    .is_ok_and(|state| state.context.external_links)
                {
                    ShellExecuteW(
                        hwnd,
                        wide("open").as_ptr(),
                        wide(REPOSITORY).as_ptr(),
                        null(),
                        null(),
                        SW_SHOWNORMAL,
                    );
                }
                return 0;
            }
            _ => {}
        }
    }
    if message == DM_GETDEFID {
        return ((DC_HASDEFID as usize) << 16 | CLOSE) as isize;
    }
    if message == WM_KEYDOWN && wparam == VK_TAB as usize {
        if let Ok(state) = cell.try_borrow() {
            let focused = GetFocus();
            let backwards = GetKeyState(VK_SHIFT as i32) < 0;
            let next = if focused == state.buttons[0] && !backwards {
                state.buttons[1]
            } else if focused == state.buttons[1] && backwards {
                state.buttons[0]
            } else if backwards {
                state.buttons[1]
            } else {
                state.buttons[0]
            };
            SetFocus(next);
        }
        return 0;
    }
    if message == READY || (message == WM_TIMER && wparam == READY_TIMER) {
        if let Ok(mut state) = cell.try_borrow_mut() {
            KillTimer(hwnd, READY_TIMER);
            accept_snapshot(&mut state);
        } else {
            SetTimer(hwnd, READY_TIMER, 50, None);
        }
        return 0;
    }
    if message == WM_DPICHANGED {
        if let Ok(mut state) = cell.try_borrow_mut() {
            let compact = state.compact_font;
            let _ = state.rebuild_resources((wparam & 0xffff) as i32, compact);
        }
        let suggested = &*(lparam as *const RECT);
        SetWindowPos(
            hwnd,
            null_mut(),
            suggested.left,
            suggested.top,
            suggested.right - suggested.left,
            suggested.bottom - suggested.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        return 0;
    }
    if message == WM_GETMINMAXINFO {
        if let Ok(state) = cell.try_borrow() {
            let limits = &mut *(lparam as *mut MINMAXINFO);
            limits.ptMinTrackSize.x = scale(420, state.dpi);
            limits.ptMinTrackSize.y = scale(320, state.dpi);
        }
        return 0;
    }
    if message == WM_SIZE {
        if let Ok(mut state) = cell.try_borrow_mut() {
            if hwnd == state.hwnd && !state.content.is_null() {
                layout(&mut state);
            }
        }
        return 0;
    }
    if message == WM_VSCROLL || message == WM_MOUSEWHEEL {
        if let Ok(mut state) = cell.try_borrow_mut() {
            let mut scroll = state.scroll;
            if message == WM_MOUSEWHEEL {
                let delta = ((wparam >> 16) as u16) as i16 as i32;
                scroll -= delta * scale(54, state.dpi) / 120;
            } else {
                match (wparam & 0xffff) as i32 {
                    SB_LINEUP => scroll -= scale(24, state.dpi),
                    SB_LINEDOWN => scroll += scale(24, state.dpi),
                    SB_PAGEUP => scroll -= viewport_height(&state),
                    SB_PAGEDOWN => scroll += viewport_height(&state),
                    SB_TOP => scroll = 0,
                    SB_BOTTOM => scroll = state.scroll_max,
                    SB_THUMBTRACK | SB_THUMBPOSITION => {
                        let mut info: SCROLLINFO = zeroed();
                        info.cbSize = size_of::<SCROLLINFO>() as u32;
                        info.fMask = SIF_TRACKPOS;
                        GetScrollInfo(state.content, SB_VERT, &mut info);
                        scroll = info.nTrackPos;
                    }
                    _ => {}
                }
            }
            state.scroll = scroll.clamp(0, state.scroll_max);
            position_content(&state);
            InvalidateRect(state.content, null(), 1);
        }
        return 0;
    }
    if message == WM_CTLCOLORSTATIC {
        if let Ok(state) = cell.try_borrow() {
            let style = state.styles.get(&(lparam as usize));
            let background = style.is_some_and(|style| style.panel);
            let color = match style {
                Some(style) if style.accent => state.palette.accent,
                Some(style) if style.muted => state.palette.muted,
                _ => state.palette.text,
            };
            SetTextColor(wparam as HDC, color);
            SetBkColor(
                wparam as HDC,
                if background {
                    state.palette.panel
                } else {
                    state.palette.background
                },
            );
            return state.brushes[usize::from(background)] as isize;
        }
    }
    if message == WM_DRAWITEM {
        if let Ok(state) = cell.try_borrow() {
            draw_button(&state, &*(lparam as *const DRAWITEMSTRUCT));
        }
        return 1;
    }
    if message == WM_ERASEBKGND {
        return 1;
    }
    if message == WM_PAINT {
        let mut ps: PAINTSTRUCT = zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        if let Ok(state) = cell.try_borrow() {
            draw(hwnd, dc, &state);
        }
        EndPaint(hwnd, &ps);
        return 0;
    }
    if message == WM_PRINTCLIENT {
        if let Ok(state) = cell.try_borrow() {
            draw(hwnd, wparam as HDC, &state);
        }
        return 0;
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

unsafe fn layout(state: &mut State) {
    let mut client: RECT = zeroed();
    GetClientRect(state.hwnd, &mut client);
    let compact = client.right * 96 / state.dpi < 900 || client.bottom * 96 / state.dpi < 610;
    if compact != state.compact_font {
        let _ = state.rebuild_resources(state.dpi, compact);
    }
    let footer = scale(64, state.dpi);
    let height = (client.bottom - footer).max(1);
    SetWindowPos(
        state.content,
        null_mut(),
        0,
        0,
        client.right.max(1),
        height,
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
    let width = (client.right - scale(20, state.dpi)).max(1);
    let margin = scale(if compact { 20 } else { 28 }, state.dpi);
    let gap = scale(24, state.dpi);
    let single_column = width * 96 / state.dpi < 660;
    let left_width = if single_column {
        (width - margin * 2).max(1)
    } else {
        ((width - margin * 2 - gap) * 34 / 100).max(1)
    };
    state.placements.clear();
    let logo_width = left_width.min(scale(290, state.dpi));
    let cell = (logo_width / 29).max(1);
    let logo_height = cell * 16;
    state.logo = RECT {
        left: margin,
        top: margin,
        right: margin + cell * 29,
        bottom: margin + logo_height,
    };
    let mut left_y = margin + logo_height + scale(24, state.dpi);
    for (index, control) in state.left.iter().enumerate() {
        let row_height = match index {
            0 => scale(28, state.dpi),
            1 => scale(if single_column { 42 } else { 70 }, state.dpi),
            2 => scale(34, state.dpi),
            7 => scale(40, state.dpi),
            8 => scale(28, state.dpi),
            _ => scale(28, state.dpi),
        };
        state.placements.push(Placement {
            window: *control,
            bounds: RECT {
                left: margin,
                top: left_y,
                right: margin + left_width,
                bottom: left_y + row_height,
            },
        });
        left_y += row_height;
    }
    let panel_left = if single_column {
        margin
    } else {
        margin + left_width + gap
    };
    let panel_width = (width - margin - panel_left).max(1);
    let padding = scale(16, state.dpi);
    let label_width = scale(if compact { 108 } else { 130 }, state.dpi).min(panel_width / 3);
    let value_left = panel_left + padding + label_width + scale(12, state.dpi);
    let value_width = (panel_left + panel_width - padding - value_left).max(1);
    let mut panel_y = if single_column {
        left_y + scale(16, state.dpi)
    } else {
        margin
    };
    let dc = GetDC(state.content);
    let old_font = SelectObject(dc, state.fonts[0]);
    for panel in &mut state.panels {
        let top = panel_y;
        state.placements.push(Placement {
            window: panel.heading,
            bounds: RECT {
                left: panel_left + padding,
                top: top + padding,
                right: panel_left + panel_width - padding,
                bottom: top + padding + scale(26, state.dpi),
            },
        });
        panel_y += padding + scale(30, state.dpi);
        if panel.rows.is_empty() {
            state.placements.push(Placement {
                window: panel.loading,
                bounds: RECT {
                    left: panel_left + padding,
                    top: panel_y,
                    right: panel_left + panel_width - padding,
                    bottom: panel_y + scale(28, state.dpi),
                },
            });
            panel_y += scale(28, state.dpi);
        }
        for row in &panel.rows {
            let mut measured = RECT {
                left: 0,
                top: 0,
                right: value_width,
                bottom: 0,
            };
            DrawTextW(
                dc,
                wide(&row.text).as_ptr(),
                -1,
                &mut measured,
                DT_WORDBREAK | DT_CALCRECT | DT_NOPREFIX,
            );
            let mut measured_label = RECT {
                left: 0,
                top: 0,
                right: label_width,
                bottom: 0,
            };
            DrawTextW(
                dc,
                wide(&row.label_text).as_ptr(),
                -1,
                &mut measured_label,
                DT_WORDBREAK | DT_CALCRECT | DT_NOPREFIX,
            );
            let row_height = (measured.bottom.max(measured_label.bottom) + scale(7, state.dpi))
                .clamp(scale(24, state.dpi), scale(78, state.dpi));
            state.placements.push(Placement {
                window: row.label,
                bounds: RECT {
                    left: panel_left + padding,
                    top: panel_y,
                    right: panel_left + padding + label_width,
                    bottom: panel_y + row_height,
                },
            });
            state.placements.push(Placement {
                window: row.value,
                bounds: RECT {
                    left: value_left,
                    top: panel_y,
                    right: value_left + value_width,
                    bottom: panel_y + row_height,
                },
            });
            panel_y += row_height;
        }
        panel_y += scale(8, state.dpi);
        panel.bounds = RECT {
            left: panel_left,
            top,
            right: panel_left + panel_width,
            bottom: panel_y,
        };
        panel_y += scale(14, state.dpi);
    }
    SelectObject(dc, old_font);
    ReleaseDC(state.content, dc);
    let total = left_y.max(panel_y) + margin;
    state.scroll_max = (total - height).max(0);
    state.scroll = state.scroll.clamp(0, state.scroll_max);
    let mut info: SCROLLINFO = zeroed();
    info.cbSize = size_of::<SCROLLINFO>() as u32;
    info.fMask = SIF_RANGE | SIF_PAGE | SIF_POS;
    info.nMin = 0;
    info.nMax = total.saturating_sub(1);
    info.nPage = height as u32;
    info.nPos = state.scroll;
    SetScrollInfo(state.content, SB_VERT, &info, 1);
    position_content(state);
    let button_width = scale(108, state.dpi);
    let button_height = scale(34, state.dpi);
    let button_y = height + (footer - button_height) / 2;
    for (index, button) in state.buttons.iter().enumerate() {
        SetWindowPos(
            *button,
            null_mut(),
            client.right
                - margin
                - button_width * (2 - index as i32)
                - scale(12, state.dpi) * (1 - index as i32),
            button_y,
            button_width,
            button_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        InvalidateRect(*button, null(), 1);
    }
    InvalidateRect(state.hwnd, null(), 1);
    InvalidateRect(state.content, null(), 1);
}

unsafe fn position_content(state: &State) {
    for placement in &state.placements {
        let bounds = placement.bounds;
        SetWindowPos(
            placement.window,
            null_mut(),
            bounds.left,
            bounds.top - state.scroll,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        // The parent excludes child controls when painting. Repaint the whole
        // text control so resizing cannot preserve its previous line wrapping.
        InvalidateRect(placement.window, null(), 1);
    }
    SetScrollPos(state.content, SB_VERT, state.scroll, 1);
}

unsafe fn viewport_height(state: &State) -> i32 {
    let mut bounds: RECT = zeroed();
    GetClientRect(state.content, &mut bounds);
    bounds.bottom.max(1)
}

unsafe fn draw(hwnd: HWND, dc: HDC, state: &State) {
    let mut bounds: RECT = zeroed();
    GetClientRect(hwnd, &mut bounds);
    FillRect(dc, &bounds, state.brushes[0]);
    if hwnd == state.content {
        for panel in &state.panels {
            let mut bounds = panel.bounds;
            bounds.top -= state.scroll;
            bounds.bottom -= state.scroll;
            rounded(dc, bounds, state.palette.panel, scale(12, state.dpi));
        }
        pixel_logo(dc, state.logo, state.scroll, state.palette.accent);
    } else {
        let footer_y = bounds.bottom - scale(64, state.dpi);
        let line = RECT {
            left: 0,
            top: footer_y,
            right: bounds.right,
            bottom: footer_y + 1,
        };
        let old = SelectObject(dc, GetStockObject(DC_BRUSH));
        SetDCBrushColor(dc, state.palette.border);
        FillRect(dc, &line, GetStockObject(DC_BRUSH));
        SelectObject(dc, old);
        let font = SelectObject(dc, state.fonts[2]);
        SetTextColor(dc, state.palette.muted);
        SetBkMode(dc, TRANSPARENT as i32);
        let mut text = RECT {
            left: scale(24, state.dpi),
            top: footer_y,
            right: (bounds.right - scale(260, state.dpi)).max(0),
            bottom: bounds.bottom,
        };
        DrawTextW(
            dc,
            wide(&tr("Informations locales · Échap fermer")).as_ptr(),
            -1,
            &mut text,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        SelectObject(dc, font);
    }
}

unsafe fn draw_button(state: &State, button: &DRAWITEMSTRUCT) {
    let mut bounds = button.rcItem;
    FillRect(button.hDC, &bounds, state.brushes[0]);
    let pressed = button.itemState & ODS_SELECTED != 0;
    rounded(
        button.hDC,
        bounds,
        if pressed {
            state.palette.border
        } else {
            state.palette.panel
        },
        scale(8, state.dpi),
    );
    let font = SelectObject(button.hDC, state.fonts[0]);
    SetBkMode(button.hDC, TRANSPARENT as i32);
    SetTextColor(
        button.hDC,
        if button.CtlID as usize == GITHUB {
            state.palette.accent
        } else {
            state.palette.text
        },
    );
    DrawTextW(
        button.hDC,
        wide(&tr(if button.CtlID as usize == GITHUB {
            "GitHub"
        } else {
            "Fermer"
        }))
        .as_ptr(),
        -1,
        &mut bounds,
        DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
    SelectObject(button.hDC, font);
    if button.itemState & ODS_FOCUS != 0 {
        bounds.left += scale(4, state.dpi);
        bounds.top += scale(4, state.dpi);
        bounds.right -= scale(4, state.dpi);
        bounds.bottom -= scale(4, state.dpi);
        DrawFocusRect(button.hDC, &bounds);
    }
}

unsafe fn rounded(dc: HDC, bounds: RECT, fill: COLORREF, radius: i32) {
    let brush = SelectObject(dc, GetStockObject(DC_BRUSH));
    let pen = SelectObject(dc, GetStockObject(NULL_PEN));
    SetDCBrushColor(dc, fill);
    RoundRect(
        dc,
        bounds.left,
        bounds.top,
        bounds.right,
        bounds.bottom,
        radius,
        radius,
    );
    SelectObject(dc, pen);
    SelectObject(dc, brush);
}

// Seven-row pixel letters are geometry, so the logo stays sharp at any DPI.
unsafe fn pixel_logo(dc: HDC, bounds: RECT, scroll: i32, accent: COLORREF) {
    let cell = ((bounds.right - bounds.left) / 29).max(1);
    let brush = SelectObject(dc, GetStockObject(DC_BRUSH));
    SetDCBrushColor(dc, accent);
    for (line, word) in ["SUPER", "MENU"].into_iter().enumerate() {
        for (letter, character) in word.chars().enumerate() {
            let rows = letter_pixels(character);
            for (row, bits) in rows.into_iter().enumerate() {
                for column in 0..5 {
                    if bits & (1 << (4 - column)) != 0 {
                        let x = bounds.left + (letter as i32 * 6 + column) * cell;
                        let y = bounds.top + (line as i32 * 9 + row as i32) * cell - scroll;
                        PatBlt(dc, x, y, cell, cell, PATCOPY);
                    }
                }
            }
        }
    }
    SelectObject(dc, brush);
}

fn letter_pixels(character: char) -> [u8; 7] {
    match character {
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 25, 21, 19, 19, 17],
        _ => [0; 7],
    }
}

unsafe fn monitor_placement(parent: HWND, dpi: i32) -> RECT {
    let mut monitor: MONITORINFO = zeroed();
    monitor.cbSize = size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(
        MonitorFromWindow(parent, MONITOR_DEFAULTTONEAREST),
        &mut monitor,
    ) == 0
    {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            (&mut monitor.rcWork as *mut RECT).cast(),
            0,
        );
    }
    placement(monitor.rcWork, dpi)
}

fn placement(work: RECT, dpi: i32) -> RECT {
    let available_width = (work.right - work.left).max(1);
    let available_height = (work.bottom - work.top).max(1);
    let gap = scale(16, dpi)
        .min(available_width / 8)
        .min(available_height / 8);
    let width = scale(1000, dpi).min((available_width - gap * 2).max(1));
    let height = scale(720, dpi).min((available_height - gap * 2).max(1));
    let left = work.left + (available_width - width) / 2;
    let top = work.top + (available_height - height) / 2;
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

fn elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 86_400 {
        fmt(
            "{days} j {hours} h",
            &[
                ("days", (seconds / 86_400).to_string()),
                ("hours", (seconds / 3600 % 24).to_string()),
            ],
        )
    } else if seconds >= 3600 {
        fmt(
            "{hours} h {minutes} min",
            &[
                ("hours", (seconds / 3600).to_string()),
                ("minutes", (seconds / 60 % 60).to_string()),
            ],
        )
    } else if seconds >= 60 {
        fmt("{minutes} min", &[("minutes", (seconds / 60).to_string())])
    } else {
        fmt("{seconds} s", &[("seconds", seconds.to_string())])
    }
}

fn clean(value: &str, limit: usize) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(limit)
        .collect()
}

fn scale(dip: i32, dpi: i32) -> i32 {
    (dip * dpi + 48) / 96
}

fn color(red: u32, green: u32, blue: u32) -> COLORREF {
    red | (green << 8) | (blue << 16)
}

fn win_error(context: &str) -> String {
    let error = std::io::Error::last_os_error().to_string();
    fmt(
        "{context} : {error}",
        &[("context", tr(context)), ("error", error)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_placement_fits_small_and_scaled_monitor_work_areas() {
        for (width, height, dpi) in [(800, 560, 96), (800, 560, 144), (1920, 1040, 192)] {
            let work = RECT {
                left: -width,
                top: 40,
                right: 0,
                bottom: 40 + height,
            };
            let bounds = placement(work, dpi);
            assert!(bounds.left >= work.left && bounds.top >= work.top);
            assert!(bounds.right <= work.right && bounds.bottom <= work.bottom);
            assert!(bounds.right > bounds.left && bounds.bottom > bounds.top);
            assert!((bounds.left - work.left - (work.right - bounds.right)).abs() <= 1);
        }
    }

    #[test]
    fn display_text_is_bounded_unicode_and_durations_are_real() {
        assert_eq!(clean("GPU\0\r\n🦀", 9), "GPU   🦀");
        assert_eq!(clean(&"🦀".repeat(1000), 512).chars().count(), 512);
        assert_eq!(elapsed(Duration::from_secs(0)), "0 s");
        assert_eq!(elapsed(Duration::from_secs(3661)), "1 h 1 min");
        assert_eq!(elapsed(Duration::from_secs(90000)), "1 j 1 h");
    }
}
