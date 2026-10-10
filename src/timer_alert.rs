//! Persistent, non-activating completion notice owned by the UI thread.

use crate::{config, native::wide};
use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::OnceLock;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{GetCapture, ReleaseCapture, SetCapture, VK_ESCAPE},
        WindowsAndMessaging::*,
    },
};

const CLASS: &str = "SuperMenuTimerAlert";
const TITLE: &str = "Minuteur terminé";
const TEXT_FLAGS: u32 = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;
/// wParam carries the generation returned by `generation()` at the last show.
pub const WM_TIMER_ALERT_DISMISSED: u32 = WM_APP + 25;

#[derive(Clone, Copy)]
struct Palette {
    background: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    border: COLORREF,
    accent: COLORREF,
    on_accent: COLORREF,
}

impl Palette {
    fn from(settings: &config::Settings) -> Self {
        let value = settings.accent_color.trim().trim_start_matches('#');
        let rgb = u32::from_str_radix(value, 16).unwrap_or(0x9A8CFF);
        let accent = color((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
        let brightness = ((rgb >> 16) & 255) * 299 + ((rgb >> 8) & 255) * 587 + (rgb & 255) * 114;
        let on_accent = if brightness >= 150_000 {
            color(17, 18, 22)
        } else {
            color(247, 247, 251)
        };
        if settings.theme.trim().eq_ignore_ascii_case("light") {
            Self {
                background: color(247, 247, 251),
                text: color(30, 28, 43),
                muted: color(103, 101, 119),
                border: color(213, 211, 224),
                accent,
                on_accent,
            }
        } else {
            Self {
                background: color(17, 18, 22),
                text: color(241, 240, 247),
                muted: color(148, 147, 165),
                border: color(45, 44, 57),
                accent,
                on_accent,
            }
        }
    }
}

struct State {
    labels: Vec<u16>,
    palette: Palette,
    dpi: i32,
    fonts: [HFONT; 3],
    anchor: HWND,
    generation: usize,
    active: bool,
    pressed: bool,
}

impl State {
    fn set_dpi(&mut self, dpi: i32) -> Result<(), String> {
        if self.dpi == dpi && self.fonts.iter().all(|font| !font.is_null()) {
            return Ok(());
        }
        let mut fonts = [null_mut(); 3];
        for (index, (height, weight)) in [(15, 600), (12, 400), (12, 600)].into_iter().enumerate() {
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
                let error = win_error("Création de la police du minuteur");
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

/// Create, show and drop this window on the launcher's message-loop thread.
pub struct TimerAlert {
    hwnd: HWND,
    // Stable allocation retained until after the last window procedure call.
    state: Box<RefCell<State>>,
}

impl TimerAlert {
    pub fn new() -> Result<Self, String> {
        register_class()?;
        let mut state = Box::new(RefCell::new(State {
            labels: wide("Minuteur"),
            palette: Palette::from(&config::Settings::default()),
            dpi: 96,
            fonts: [null_mut(); 3],
            anchor: null_mut(),
            generation: 0,
            active: false,
            pressed: false,
        }));
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                wide(CLASS).as_ptr(),
                wide(TITLE).as_ptr(),
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
            return Err(win_error("Création de l’alerte du minuteur"));
        }
        Ok(Self { hwnd, state })
    }

    /// Replace the labels without stacking notices; no timer or sound is used.
    pub fn show(
        &mut self,
        titles: &[String],
        anchor: HWND,
        settings: &config::Settings,
    ) -> Result<(), String> {
        if titles.is_empty() {
            self.hide();
            return Ok(());
        }
        let mut monitor: MONITORINFO = unsafe { zeroed() };
        monitor.cbSize = size_of::<MONITORINFO>() as u32;
        if unsafe {
            GetMonitorInfoW(
                MonitorFromWindow(anchor, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            )
        } == 0
        {
            return Err(win_error("Position de l’alerte du minuteur"));
        }
        let dpi = unsafe { GetDpiForWindow(anchor) }.max(96) as i32;
        let bounds = placement(monitor.rcWork, dpi);
        let labels = grouped_labels(titles);
        {
            let mut state = self.state.borrow_mut();
            state.set_dpi(dpi)?;
            state.labels = wide(&labels);
            state.palette = Palette::from(settings);
            state.anchor = anchor;
            state.generation = state.generation.wrapping_add(1).max(1);
            state.active = true;
            state.pressed = false;
        }
        // Do not hold RefCell borrows across synchronous Win32 messages.
        unsafe {
            SetWindowTextW(self.hwnd, wide(&format!("{TITLE} · {labels}")).as_ptr());
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
                return Err(win_error("Position de l’alerte du minuteur"));
            }
            set_round_region(self.hwnd, bounds, dpi);
            let dark = i32::from(!settings.theme.trim().eq_ignore_ascii_case("light"));
            let round = 2i32;
            DwmSetWindowAttribute(self.hwnd, 20, (&dark as *const i32).cast(), 4);
            DwmSetWindowAttribute(self.hwnd, 33, (&round as *const i32).cast(), 4);
            ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            InvalidateRect(self.hwnd, null(), 0);
            UpdateWindow(self.hwnd);
        }
        Ok(())
    }

    /// Programmatic hiding is not an acknowledgement of the completed timers.
    pub fn hide(&mut self) {
        {
            let mut state = self.state.borrow_mut();
            state.active = false;
            state.pressed = false;
        }
        unsafe {
            if GetCapture() == self.hwnd {
                ReleaseCapture();
            }
            ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd) != 0 }
    }

    pub fn generation(&self) -> usize {
        self.state.borrow().generation
    }
}

impl Drop for TimerAlert {
    fn drop(&mut self) {
        self.hide();
        unsafe { DestroyWindow(self.hwnd) };
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
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                lpszClassName: class_name.as_ptr(),
                ..zeroed()
            };
            if RegisterClassExW(&class) == 0 {
                Err(win_error("Classe de l’alerte du minuteur"))
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
        WM_ERASEBKGND => return 1,
        WM_NCDESTROY => {
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
        WM_LBUTTONDOWN if !ptr.is_null() => {
            let mut bounds: RECT = zeroed();
            GetClientRect(hwnd, &mut bounds);
            let pressed = if let Ok(mut state) = (*ptr).try_borrow_mut() {
                state.pressed = state.active
                    && contains(button_bounds(bounds, state.dpi), client_point(lparam));
                state.pressed
            } else {
                false
            };
            if pressed {
                SetCapture(hwnd);
                InvalidateRect(hwnd, null(), 0);
            }
            return 0;
        }
        WM_LBUTTONUP if !ptr.is_null() => {
            let mut bounds: RECT = zeroed();
            GetClientRect(hwnd, &mut bounds);
            let close = if let Ok(mut state) = (*ptr).try_borrow_mut() {
                let close = state.pressed
                    && contains(button_bounds(bounds, state.dpi), client_point(lparam));
                state.pressed = false;
                close
            } else {
                false
            };
            if GetCapture() == hwnd {
                ReleaseCapture();
            }
            if close {
                dismiss(hwnd, &*ptr);
            } else {
                InvalidateRect(hwnd, null(), 0);
            }
            return 0;
        }
        WM_CAPTURECHANGED if !ptr.is_null() => {
            if let Ok(mut state) = (*ptr).try_borrow_mut() {
                state.pressed = false;
            }
            InvalidateRect(hwnd, null(), 0);
            return 0;
        }
        WM_KEYDOWN if wparam == VK_ESCAPE as usize && !ptr.is_null() => {
            dismiss(hwnd, &*ptr);
            return 0;
        }
        WM_CLOSE if !ptr.is_null() => {
            dismiss(hwnd, &*ptr);
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

unsafe fn dismiss(hwnd: HWND, state: &RefCell<State>) {
    let acknowledgement = if let Ok(mut state) = state.try_borrow_mut() {
        if state.active {
            state.active = false;
            state.pressed = false;
            Some((state.anchor, state.generation))
        } else {
            None
        }
    } else {
        None
    };
    if GetCapture() == hwnd {
        ReleaseCapture();
    }
    ShowWindow(hwnd, SW_HIDE);
    if let Some((anchor, generation)) = acknowledgement {
        if !anchor.is_null() {
            PostMessageW(anchor, WM_TIMER_ALERT_DISMISSED, generation, 0);
        }
    }
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
    let clock_pen = CreatePen(PS_SOLID, scale(2, state.dpi), state.palette.on_accent);
    if !clock_pen.is_null() {
        SelectObject(dc, clock_pen);
        let clock_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
        let inset = scale(9, state.dpi);
        Ellipse(
            dc,
            left + inset,
            top + inset,
            left + size - inset,
            top + size - inset,
        );
        let center_x = left + size / 2;
        let center_y = top + size / 2;
        MoveToEx(dc, center_x, center_y - scale(7, state.dpi), null_mut());
        LineTo(dc, center_x, center_y);
        LineTo(dc, center_x + scale(6, state.dpi), center_y);
        SelectObject(dc, clock_brush);
        SelectObject(dc, GetStockObject(NULL_PEN));
        DeleteObject(clock_pen);
    }
    let button = button_bounds(bounds, state.dpi);
    SetDCBrushColor(
        dc,
        if state.pressed {
            state.palette.border
        } else {
            state.palette.accent
        },
    );
    RoundRect(
        dc,
        button.left,
        button.top,
        button.right,
        button.bottom,
        scale(8, state.dpi),
        scale(8, state.dpi),
    );
    SelectObject(dc, pen);
    SelectObject(dc, brush);
    SetBkMode(dc, TRANSPARENT as i32);
    let text_left = scale(74, state.dpi);
    let text_right = (bounds.right - scale(22, state.dpi)).max(text_left);
    draw_text(
        dc,
        state.fonts[0],
        state.palette.text,
        &wide(TITLE),
        RECT {
            left: text_left,
            top: scale(17, state.dpi),
            right: text_right,
            bottom: scale(43, state.dpi),
        },
        TEXT_FLAGS,
    );
    draw_text(
        dc,
        state.fonts[1],
        state.palette.muted,
        &state.labels,
        RECT {
            left: text_left,
            top: scale(46, state.dpi),
            right: text_right,
            bottom: scale(88, state.dpi),
        },
        DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX,
    );
    draw_text(
        dc,
        state.fonts[2],
        if state.pressed {
            state.palette.text
        } else {
            state.palette.on_accent
        },
        &wide("Fermer"),
        button,
        TEXT_FLAGS | DT_CENTER,
    );
}

unsafe fn draw_text(
    dc: HDC,
    font: HFONT,
    color: COLORREF,
    value: &[u16],
    mut bounds: RECT,
    flags: u32,
) {
    let old = SelectObject(dc, font);
    SetTextColor(dc, color);
    DrawTextW(dc, value.as_ptr(), -1, &mut bounds, flags);
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
        DeleteObject(region);
    }
}

fn placement(work: RECT, dpi: i32) -> RECT {
    let area_width = (work.right - work.left).max(1);
    let area_height = (work.bottom - work.top).max(1);
    let gap = scale(24, dpi).min(area_width / 4).min(area_height / 4);
    let width = scale(480, dpi).min((area_width - gap * 2).max(1));
    let height = scale(148, dpi).min((area_height - gap * 2).max(1));
    let left = work.left + (area_width - width) / 2;
    let top = work.bottom - gap - height;
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

fn button_bounds(client: RECT, dpi: i32) -> RECT {
    let width = (client.right - client.left).max(1);
    let height = (client.bottom - client.top).max(1);
    let horizontal_gap = scale(22, dpi).min(width / 8);
    let vertical_gap = scale(14, dpi).min(height / 8);
    let right = client.right - horizontal_gap;
    let bottom = client.bottom - vertical_gap;
    RECT {
        left: right - scale(106, dpi).min((width - horizontal_gap * 2).max(1)),
        top: bottom - scale(30, dpi).min((height - vertical_gap * 2).max(1)),
        right,
        bottom,
    }
}

fn grouped_labels(titles: &[String]) -> String {
    let mut labels = titles
        .iter()
        .take(8)
        .map(|title| {
            let cleaned: String = title
                .chars()
                .map(|ch| if ch.is_control() { ' ' } else { ch })
                .take(80)
                .collect();
            if cleaned.trim().is_empty() {
                "Minuteur".to_string()
            } else {
                cleaned.trim().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" · ");
    if titles.len() > 8 {
        labels.push_str(&format!(" · +{} minuteurs", titles.len() - 8));
    }
    labels
}

fn client_point(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam as u32 & 0xffff) as i16 as i32,
        y: ((lparam as u32 >> 16) & 0xffff) as i16 as i32,
    }
}

fn contains(bounds: RECT, point: POINT) -> bool {
    point.x >= bounds.left
        && point.x < bounds.right
        && point.y >= bounds.top
        && point.y < bounds.bottom
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
    fn grouped_unicode_labels_are_bounded_and_single_line() {
        assert_eq!(
            grouped_labels(&["Thé 🫖\r\n&&\0Pause".into(), "  ".into()]),
            "Thé 🫖  && Pause · Minuteur"
        );
        let titles = vec!["🦀".repeat(10000); 10];
        let labels = grouped_labels(&titles);
        assert!(labels.ends_with("+2 minuteurs"));
        assert!(labels.chars().count() < 680);
        assert!(String::from_utf16(&wide(&labels)[..wide(&labels).len() - 1]).is_ok());
    }

    #[test]
    fn monitor_placement_and_button_scale_without_crossing_work_area() {
        let work = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1040,
        };
        let bounds = placement(work, 144);
        assert_eq!(bounds.right - bounds.left, 720);
        assert_eq!(bounds.bottom - bounds.top, 222);
        assert_eq!(work.bottom - bounds.bottom, 36);
        assert_eq!(bounds.left - work.left, work.right - bounds.right);
        let client = RECT {
            left: 0,
            top: 0,
            right: 720,
            bottom: 222,
        };
        let button = button_bounds(client, 144);
        assert_eq!(button.right - button.left, 159);
        assert_eq!(button.bottom - button.top, 45);
        assert!(contains(
            button,
            POINT {
                x: button.left,
                y: button.top
            }
        ));
        assert!(!contains(
            button,
            POINT {
                x: button.right,
                y: button.bottom
            }
        ));
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
        let tiny_button = button_bounds(
            RECT {
                left: 0,
                top: 0,
                right: tiny.right - tiny.left,
                bottom: tiny.bottom - tiny.top,
            },
            192,
        );
        assert!(tiny_button.left >= 0 && tiny_button.right <= tiny.right - tiny.left);
        assert!(tiny_button.top >= 0 && tiny_button.bottom <= tiny.bottom - tiny.top);
    }

    #[test]
    fn native_notice_persists_without_activation_and_acknowledges_its_generation() {
        let _ui_test_guard = crate::native::UI_TEST_LOCK.lock().unwrap();
        let anchor = TimerAlert::new().unwrap();
        let mut popup = TimerAlert::new().unwrap();
        let settings = config::Settings::default();
        let foreground = unsafe { GetForegroundWindow() };
        popup
            .show(&["Thé 🫖".into()], anchor.hwnd(), &settings)
            .unwrap();
        let hwnd = popup.hwnd();
        let generation = popup.generation();
        assert!(popup.is_visible());
        assert_ne!(generation, 0);
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            assert_ne!(style & WS_EX_NOACTIVATE, 0);
            assert_ne!(style & WS_EX_TOOLWINDOW, 0);
            assert_eq!(style & WS_EX_TRANSPARENT, 0);
            assert_eq!(GetWindow(hwnd, GW_OWNER), null_mut());
            assert_eq!(GetForegroundWindow(), foreground);
            assert_eq!(
                SendMessageW(hwnd, WM_MOUSEACTIVATE, 0, 0),
                MA_NOACTIVATE as isize
            );
            SendMessageW(hwnd, WM_TIMER, 1, 0);
        }
        assert!(popup.is_visible());
        popup
            .show(&["Thé 🫖".into(), "Pause".into()], anchor.hwnd(), &settings)
            .unwrap();
        assert_ne!(popup.generation(), generation);
        let mut caption = [0u16; 256];
        let length = unsafe { GetWindowTextW(hwnd, caption.as_mut_ptr(), caption.len() as i32) };
        assert_eq!(
            String::from_utf16(&caption[..length as usize]).unwrap(),
            "Minuteur terminé · Thé 🫖 · Pause"
        );
        let mut bounds: RECT = unsafe { zeroed() };
        unsafe { GetClientRect(hwnd, &mut bounds) };
        let button = button_bounds(bounds, popup.state.borrow().dpi);
        let x = (button.left + button.right) / 2;
        let y = (button.top + button.bottom) / 2;
        let point = ((y as u32) << 16 | x as u32) as LPARAM;
        unsafe {
            SendMessageW(hwnd, WM_LBUTTONDOWN, 1, point);
            SendMessageW(hwnd, WM_LBUTTONUP, 0, point);
        }
        assert!(!popup.is_visible());
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);
        let mut message: MSG = unsafe { zeroed() };
        assert_ne!(
            unsafe {
                PeekMessageW(
                    &mut message,
                    anchor.hwnd(),
                    WM_TIMER_ALERT_DISMISSED,
                    WM_TIMER_ALERT_DISMISSED,
                    PM_REMOVE,
                )
            },
            0
        );
        assert_eq!(message.wParam, popup.generation());
        unsafe { SendMessageW(hwnd, WM_CLOSE, 0, 0) };
        assert_eq!(
            unsafe {
                PeekMessageW(
                    &mut message,
                    anchor.hwnd(),
                    WM_TIMER_ALERT_DISMISSED,
                    WM_TIMER_ALERT_DISMISSED,
                    PM_REMOVE,
                )
            },
            0
        );
        popup
            .show(&["Pause".into()], anchor.hwnd(), &settings)
            .unwrap();
        popup.hide();
        assert!(!popup.is_visible());
        assert_eq!(
            unsafe {
                PeekMessageW(
                    &mut message,
                    anchor.hwnd(),
                    WM_TIMER_ALERT_DISMISSED,
                    WM_TIMER_ALERT_DISMISSED,
                    PM_REMOVE,
                )
            },
            0
        );
        popup
            .show(&["Pause".into()], anchor.hwnd(), &settings)
            .unwrap();
        unsafe { SendMessageW(hwnd, WM_KEYDOWN, VK_ESCAPE as usize, 0) };
        assert!(!popup.is_visible());
        assert_ne!(
            unsafe {
                PeekMessageW(
                    &mut message,
                    anchor.hwnd(),
                    WM_TIMER_ALERT_DISMISSED,
                    WM_TIMER_ALERT_DISMISSED,
                    PM_REMOVE,
                )
            },
            0
        );
        assert_eq!(message.wParam, popup.generation());
        drop(popup);
        assert_eq!(unsafe { IsWindow(hwnd) }, 0);
    }
}
