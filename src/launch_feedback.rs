//! A short, non-activating acknowledgement of a launch request, not app readiness.

use crate::{config, native::wide};
use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*},
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

const CLASS: &str = "SuperMenuLaunchFeedback";
const TIMER: usize = 1;
const DURATION: Duration = Duration::from_secs(4);
const SUBTITLE: &str = "Ouverture demandée";
const TEXT_FLAGS: u32 = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;

#[derive(Clone, Copy)]
struct Palette {
    background: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    border: COLORREF,
    accent: COLORREF,
}

impl Palette {
    fn from(settings: &config::Settings) -> Self {
        let value = settings.accent_color.trim().trim_start_matches('#');
        let rgb = u32::from_str_radix(value, 16).unwrap_or(0x9A8CFF);
        let accent = color((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
        if settings.theme.trim().eq_ignore_ascii_case("light") {
            Self {
                background: color(247, 247, 251),
                text: color(30, 28, 43),
                muted: color(103, 101, 119),
                border: color(213, 211, 224),
                accent,
            }
        } else {
            Self {
                background: color(17, 18, 22),
                text: color(241, 240, 247),
                muted: color(148, 147, 165),
                border: color(45, 44, 57),
                accent,
            }
        }
    }
}

struct State {
    title: Vec<u16>,
    palette: Palette,
    dpi: i32,
    fonts: [HFONT; 3],
    deadline: Option<Instant>,
}

impl State {
    fn set_dpi(&mut self, dpi: i32) -> Result<(), String> {
        if self.dpi == dpi && self.fonts.iter().all(|font| !font.is_null()) {
            return Ok(());
        }
        let mut fonts = [null_mut(); 3];
        for (index, (height, weight, family)) in [
            (15, 600, "Segoe UI"),
            (12, 400, "Segoe UI"),
            (24, 400, "Segoe UI Symbol"),
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
                    wide(family).as_ptr(),
                )
            };
            if fonts[index].is_null() {
                let error = win_error("Création de la police du retour de lancement");
                for font in fonts.into_iter().filter(|font| !font.is_null()) {
                    unsafe { DeleteObject(font) };
                }
                return Err(error);
            }
        }
        for font in self.fonts.into_iter().filter(|font| !font.is_null()) {
            unsafe { DeleteObject(font) };
        }
        self.fonts = fonts;
        self.dpi = dpi;
        Ok(())
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for font in self.fonts.into_iter().filter(|font| !font.is_null()) {
            unsafe { DeleteObject(font) };
        }
    }
}

/// Created, shown and dropped on the launcher's message-loop thread.
pub struct LaunchFeedback {
    hwnd: HWND,
    // The window procedure references this stable allocation; never transfer it
    // to a worker or hold a RefCell borrow across synchronous window messages.
    state: Box<RefCell<State>>,
}

impl LaunchFeedback {
    pub fn new() -> Result<Self, String> {
        register_class()?;
        let mut state = Box::new(RefCell::new(State {
            title: wide("Lancement…"),
            palette: Palette::from(&config::Settings::default()),
            dpi: 96,
            fonts: [null_mut(); 3],
            deadline: None,
        }));
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT,
                wide(CLASS).as_ptr(),
                wide("Super Menu · Ouverture demandée").as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                (&mut *state as *mut RefCell<State>).cast(),
            )
        };
        if hwnd.is_null() {
            return Err(win_error("Création du retour de lancement"));
        }
        let feedback = Self { hwnd, state };
        if unsafe { SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA) } == 0 {
            return Err(win_error("Affichage du retour de lancement"));
        }
        Ok(feedback)
    }

    /// Paints immediately; the caller can then dispatch Shell work to a worker.
    pub fn show(
        &mut self,
        title: &str,
        anchor: HWND,
        settings: &config::Settings,
    ) -> Result<(), String> {
        self.hide();
        let deadline = Instant::now() + DURATION;
        let mut monitor: MONITORINFO = unsafe { zeroed() };
        monitor.cbSize = size_of::<MONITORINFO>() as u32;
        if unsafe {
            GetMonitorInfoW(
                MonitorFromWindow(anchor, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            )
        } == 0
        {
            return Err(win_error("Position du retour de lancement"));
        }
        let dpi = unsafe { GetDpiForWindow(anchor) }.max(96) as i32;
        let bounds = placement(monitor.rcWork, dpi);
        {
            let mut state = self.state.borrow_mut();
            state.set_dpi(dpi)?;
            state.title = title_text(title);
            state.palette = Palette::from(settings);
            state.deadline = Some(deadline);
        }
        // All synchronous Win32 calls run with the state borrow released.
        unsafe {
            let title = self.state.borrow().title.clone();
            SetWindowTextW(self.hwnd, title.as_ptr());
            if SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                bounds.left,
                bounds.top,
                bounds.right - bounds.left,
                bounds.bottom - bounds.top,
                SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            ) == 0
            {
                self.hide();
                return Err(win_error("Position du retour de lancement"));
            }
            set_round_region(self.hwnd, bounds, dpi);
            let dark = i32::from(!settings.theme.eq_ignore_ascii_case("light"));
            let round = 2i32;
            DwmSetWindowAttribute(self.hwnd, 20, (&dark as *const i32).cast(), 4);
            DwmSetWindowAttribute(self.hwnd, 33, (&round as *const i32).cast(), 4);
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            InvalidateRect(self.hwnd, null(), 0);
            UpdateWindow(self.hwnd);
            // WS_EX_TRANSPARENT defers ordinary WM_PAINT until windows below
            // have painted. Render this owned client DC synchronously instead,
            // so the acknowledgement is ready before Shell work is dispatched.
            let dc = GetDC(self.hwnd);
            if dc.is_null() {
                self.hide();
                return Err(win_error("Peinture du retour de lancement"));
            }
            SendMessageW(self.hwnd, WM_PRINTCLIENT, dc as usize, PRF_CLIENT as isize);
            ReleaseDC(self.hwnd, dc);
            ValidateRect(self.hwnd, null());
            let remaining = deadline.saturating_duration_since(Instant::now());
            if SetTimer(self.hwnd, TIMER, remaining.as_millis().max(1) as u32, None) == 0 {
                self.hide();
                return Err(win_error("Durée du retour de lancement"));
            }
        }
        Ok(())
    }

    pub fn hide(&mut self) {
        self.state.borrow_mut().deadline = None;
        unsafe {
            KillTimer(self.hwnd, TIMER);
            ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for LaunchFeedback {
    fn drop(&mut self) {
        self.hide();
        unsafe { DestroyWindow(self.hwnd) };
        // State and its fonts outlive WM_NCDESTROY and are dropped afterwards.
    }
}

fn register_class() -> Result<(), String> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| unsafe {
            let class_name = wide(CLASS);
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                hInstance: GetModuleHandleW(null()),
                lpszClassName: class_name.as_ptr(),
                ..zeroed()
            };
            if RegisterClassExW(&class) == 0 {
                Err(win_error("Classe du retour de lancement"))
            } else {
                Ok(())
            }
        })
        .clone()
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
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<State>;
    match message {
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as isize,
        WM_NCHITTEST => return HTTRANSPARENT as isize,
        WM_ERASEBKGND => return 1,
        WM_NCDESTROY => {
            KillTimer(hwnd, TIMER);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
        WM_PAINT if !ptr.is_null() => {
            let mut paint: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut paint);
            if let Ok(state) = (*ptr).try_borrow() {
                draw(hwnd, dc, &state);
            }
            EndPaint(hwnd, &paint);
            return 0;
        }
        WM_PRINTCLIENT if !ptr.is_null() => {
            if let Ok(state) = (*ptr).try_borrow() {
                draw(hwnd, wparam as HDC, &state);
            }
            return 0;
        }
        WM_TIMER if wparam == TIMER && !ptr.is_null() => {
            let expired = if let Ok(mut state) = (*ptr).try_borrow_mut() {
                let expired = state
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline);
                if expired {
                    state.deadline = None;
                }
                expired
            } else {
                false
            };
            // KillTimer does not remove queued messages. An earlier launch's
            // WM_TIMER must not dismiss a replacement before its own deadline.
            if expired {
                KillTimer(hwnd, TIMER);
                ShowWindow(hwnd, SW_HIDE);
            }
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

unsafe fn draw(hwnd: HWND, dc: HDC, state: &State) {
    let mut bounds: RECT = zeroed();
    GetClientRect(hwnd, &mut bounds);
    let brush = SelectObject(dc, GetStockObject(DC_BRUSH));
    let pen = SelectObject(dc, GetStockObject(NULL_PEN));
    SetDCBrushColor(dc, state.palette.border);
    PatBlt(dc, 0, 0, bounds.right, bounds.bottom, PATCOPY);
    SetDCBrushColor(dc, state.palette.background);
    let radius = scale(16, state.dpi);
    RoundRect(
        dc,
        1,
        1,
        bounds.right - 1,
        bounds.bottom - 1,
        radius,
        radius,
    );
    SetDCBrushColor(dc, state.palette.accent);
    let left = scale(18, state.dpi);
    let top = scale(22, state.dpi);
    let size = scale(40, state.dpi);
    Ellipse(dc, left, top, left + size, top + size);
    SelectObject(dc, pen);
    SelectObject(dc, brush);
    SetBkMode(dc, TRANSPARENT as i32);
    let icon = wide("↗");
    draw_text(
        dc,
        state.fonts[2],
        state.palette.background,
        &icon,
        RECT {
            left,
            top,
            right: left + size,
            bottom: top + size,
        },
        DT_CENTER,
    );
    let left = scale(74, state.dpi);
    let right = (bounds.right - scale(22, state.dpi)).max(left);
    draw_text(
        dc,
        state.fonts[0],
        state.palette.text,
        &state.title,
        RECT {
            left,
            top: scale(17, state.dpi),
            right,
            bottom: scale(43, state.dpi),
        },
        DT_LEFT,
    );
    draw_text(
        dc,
        state.fonts[1],
        state.palette.muted,
        &wide(SUBTITLE),
        RECT {
            left,
            top: scale(45, state.dpi),
            right,
            bottom: scale(67, state.dpi),
        },
        DT_LEFT,
    );
}

unsafe fn draw_text(
    dc: HDC,
    font: HFONT,
    color: COLORREF,
    value: &[u16],
    mut bounds: RECT,
    alignment: u32,
) {
    let old = SelectObject(dc, font);
    SetTextColor(dc, color);
    DrawTextW(dc, value.as_ptr(), -1, &mut bounds, TEXT_FLAGS | alignment);
    SelectObject(dc, old);
}

unsafe fn set_round_region(hwnd: HWND, bounds: RECT, dpi: i32) {
    let radius = scale(16, dpi);
    let region = CreateRoundRectRgn(
        0,
        0,
        bounds.right - bounds.left + 1,
        bounds.bottom - bounds.top + 1,
        radius,
        radius,
    );
    if !region.is_null() && SetWindowRgn(hwnd, region, 0) == 0 {
        // Windows owns the region only after a successful SetWindowRgn call.
        DeleteObject(region);
    }
}

fn placement(work: RECT, dpi: i32) -> RECT {
    let area_width = (work.right - work.left).max(1);
    let area_height = (work.bottom - work.top).max(1);
    let gap = scale(24, dpi).min(area_width / 4).min(area_height / 4);
    let width = scale(480, dpi).min((area_width - gap * 2).max(1));
    let height = scale(84, dpi).min((area_height - gap * 2).max(1));
    let left = work.left + (area_width - width) / 2;
    let top = work.bottom - gap - height;
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

fn title_text(title: &str) -> Vec<u16> {
    // Bound the allocation, keep Unicode scalar boundaries, and prevent paths
    // or custom command names from introducing control characters or new lines.
    let title: String = title
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(160)
        .collect();
    wide(&format!("Lancement de {}…", title.trim()))
}

fn scale(dip: i32, dpi: i32) -> i32 {
    (dip * dpi + 48) / 96
}

fn color(r: u32, g: u32, b: u32) -> COLORREF {
    r | (g << 8) | (b << 16)
}

fn win_error(context: &str) -> String {
    format!("{context} : {}", std::io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_title_and_long_names_are_safe_single_line_utf16() {
        let value = title_text("Éditeur 🦀\r\n&&\0Document");
        let decoded = String::from_utf16(&value[..value.len() - 1]).unwrap();
        assert_eq!(decoded, "Lancement de Éditeur 🦀  && Document…");
        assert_eq!(value.last(), Some(&0));
        let long = title_text(&"🦀".repeat(10_000));
        assert!(long.len() < 350);
        assert!(String::from_utf16(&long[..long.len() - 1]).is_ok());
        assert_ne!(TEXT_FLAGS & DT_END_ELLIPSIS, 0);
        assert_ne!(TEXT_FLAGS & DT_NOPREFIX, 0);
    }

    #[test]
    fn theme_and_monitor_placement_preserve_contrast_and_dpi() {
        let mut settings = config::Settings::default();
        let dark = Palette::from(&settings);
        settings.theme = "light".into();
        settings.accent_color = "#123456".into();
        let light = Palette::from(&settings);
        assert_ne!(dark.background, light.background);
        assert_ne!(dark.text, light.text);
        assert_eq!(light.accent, color(0x12, 0x34, 0x56));
        let work = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1040,
        };
        let bounds = placement(work, 144);
        assert_eq!(bounds.right - bounds.left, 720);
        assert_eq!(bounds.bottom - bounds.top, 126);
        assert_eq!(work.bottom - bounds.bottom, 36);
        assert_eq!(bounds.left - work.left, work.right - bounds.right);
        let tiny = placement(
            RECT {
                left: 0,
                top: 0,
                right: 100,
                bottom: 40,
            },
            192,
        );
        assert!(tiny.left >= 0 && tiny.right <= 100);
        assert!(tiny.top >= 0 && tiny.bottom <= 40);
    }

    #[test]
    fn own_popup_paints_without_activation_replaces_and_hides_on_native_timer() {
        let anchor = LaunchFeedback::new().unwrap();
        let mut popup = LaunchFeedback::new().unwrap();
        let settings = config::Settings::default();
        let foreground = unsafe { GetForegroundWindow() };
        popup.show("Éditeur 🦀", anchor.hwnd(), &settings).unwrap();
        let hwnd = popup.hwnd();
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            assert_ne!(style & WS_EX_NOACTIVATE, 0);
            assert_ne!(style & WS_EX_TOOLWINDOW, 0);
            assert_ne!(style & WS_EX_TRANSPARENT, 0);
            assert_ne!(style & WS_EX_LAYERED, 0);
            assert_eq!(style & WS_EX_APPWINDOW, 0);
            assert_eq!(GetWindow(hwnd, GW_OWNER), null_mut());
            assert_ne!(IsWindowVisible(hwnd), 0);
            assert_eq!(GetForegroundWindow(), foreground);
            assert_eq!(
                SendMessageW(hwnd, WM_MOUSEACTIVATE, 0, 0),
                MA_NOACTIVATE as isize
            );
            assert_eq!(
                SendMessageW(hwnd, WM_NCHITTEST, 0, 0),
                HTTRANSPARENT as isize
            );
            // A layered window's GetDC can expose a separate redirection
            // surface. Read one known interior pixel after DWM composition.
            windows_sys::Win32::Graphics::Dwm::DwmFlush();
            let mut bounds: RECT = zeroed();
            GetWindowRect(hwnd, &mut bounds);
            let dc = GetDC(null_mut());
            assert!(!dc.is_null());
            assert_eq!(
                GetPixel(dc, bounds.left + 5, bounds.top + 40),
                Palette::from(&settings).background
            );
            ReleaseDC(null_mut(), dc);
        }
        popup
            .show("Deuxième lancement", anchor.hwnd(), &settings)
            .unwrap();
        unsafe {
            // Simulate an already queued timer from the preceding request.
            SendMessageW(hwnd, WM_TIMER, TIMER, 0);
            assert_ne!(IsWindowVisible(hwnd), 0);
        }
        let mut caption = [0u16; 256];
        let length = unsafe { GetWindowTextW(hwnd, caption.as_mut_ptr(), caption.len() as i32) };
        assert_eq!(
            String::from_utf16(&caption[..length as usize]).unwrap(),
            "Lancement de Deuxième lancement…"
        );
        let limit = Instant::now() + Duration::from_secs(5);
        while unsafe { IsWindowVisible(hwnd) } != 0 && Instant::now() < limit {
            let mut message: MSG = unsafe { zeroed() };
            unsafe {
                while PeekMessageW(&mut message, hwnd, 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(unsafe { IsWindowVisible(hwnd) }, 0);
        assert!(popup.state.borrow().deadline.is_none());
        popup.hide();
        unsafe { SendMessageW(hwnd, WM_TIMER, TIMER, 0) };
        assert_eq!(unsafe { IsWindowVisible(hwnd) }, 0);
        drop(popup);
        assert_eq!(unsafe { IsWindow(hwnd) }, 0);
    }
}
