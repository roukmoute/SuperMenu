//! A short, non-activating acknowledgement of a launch request, not app readiness.

use crate::i18n::{fmt, tr};
use crate::{brand_icon, config, native::wide};
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
pub const DURATION: Duration = Duration::from_secs(2);
const SUBTITLE: &str = "Ouverture demandée";
const TEXT_FLAGS: u32 = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;

#[derive(Clone, Copy)]
struct Palette {
    background: COLORREF,
    text: COLORREF,
    muted: COLORREF,
    border: COLORREF,
}

impl Palette {
    fn from(settings: &config::Settings) -> Self {
        if settings.theme.trim().eq_ignore_ascii_case("light") {
            Self {
                background: color(247, 247, 251),
                text: color(30, 28, 43),
                muted: color(103, 101, 119),
                border: color(213, 211, 224),
            }
        } else {
            Self {
                background: color(17, 18, 22),
                text: color(241, 240, 247),
                muted: color(148, 147, 165),
                border: color(45, 44, 57),
            }
        }
    }
}

struct OwnedIcon(HICON);

impl OwnedIcon {
    fn copy(icon: HICON) -> Option<Self> {
        if icon.is_null() {
            return None;
        }
        let copy = unsafe { CopyIcon(icon) };
        (!copy.is_null()).then_some(Self(copy))
    }
}

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        unsafe { DestroyIcon(self.0) };
    }
}

struct State {
    title: Vec<u16>,
    palette: Palette,
    dpi: i32,
    fonts: [HFONT; 2],
    icon: Option<OwnedIcon>,
    fallback: brand_icon::Icon,
    deadline: Option<Instant>,
}

impl State {
    fn set_dpi(&mut self, dpi: i32) -> Result<(), String> {
        if self.dpi == dpi && self.fonts.iter().all(|font| !font.is_null()) {
            return Ok(());
        }
        let mut fonts = [null_mut(); 2];
        for (index, (height, weight, family)) in [(15, 600, "Segoe UI"), (12, 400, "Segoe UI")]
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
        if self.dpi != dpi {
            self.fallback = brand_icon::Icon::new(icon_size(dpi));
        }
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
            title: wide(&tr("Lancement…")),
            palette: Palette::from(&config::Settings::default()),
            dpi: 96,
            fonts: [null_mut(); 2],
            icon: None,
            fallback: brand_icon::Icon::new(icon_size(96)),
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
                wide(&tr("Super Menu · Ouverture demandée")).as_ptr(),
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
        icon: Option<HICON>,
        anchor: HWND,
        settings: &config::Settings,
    ) -> Result<(), String> {
        // The cache's borrowed icon may be released after this call. Own a copy
        // before hiding the preceding request or dispatching any window messages.
        let icon = icon.and_then(OwnedIcon::copy);
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
            state.icon = icon;
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
            if !repaint(self.hwnd) {
                self.hide();
                return Err(win_error("Peinture du retour de lancement"));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if SetTimer(self.hwnd, TIMER, timer_interval(remaining), None) == 0 {
                self.hide();
                return Err(win_error("Durée du retour de lancement"));
            }
        }
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        self.state
            .borrow()
            .deadline
            .is_some_and(|deadline| deadline > Instant::now())
    }

    /// Accepts asynchronous cache artwork only while this acknowledgement is
    /// alive. Updating pixels cannot extend its deadline or reopen its window.
    pub fn update_icon(&mut self, icon: HICON) -> bool {
        if !self.is_active() {
            return false;
        }
        let Some(icon) = OwnedIcon::copy(icon) else {
            return false;
        };
        {
            let mut state = self.state.borrow_mut();
            if state
                .deadline
                .is_none_or(|deadline| deadline <= Instant::now())
            {
                return false;
            }
            state.icon = Some(icon);
        }
        unsafe { repaint(self.hwnd) }
    }

    pub fn hide(&mut self) {
        {
            let mut state = self.state.borrow_mut();
            state.deadline = None;
            state.icon = None;
        }
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
            let remaining = if let Ok(mut state) = (*ptr).try_borrow_mut() {
                state.deadline.map(|deadline| {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        state.deadline = None;
                        state.icon = None;
                    }
                    remaining
                })
            } else {
                None
            };
            // KillTimer does not remove queued messages. A stale or early tick
            // must wait only until this request's deadline, not another full period.
            if let Some(remaining) = remaining {
                if remaining.is_zero()
                    || SetTimer(hwnd, TIMER, timer_interval(remaining), None) == 0
                {
                    if let Ok(mut state) = (*ptr).try_borrow_mut() {
                        state.deadline = None;
                        state.icon = None;
                    }
                    KillTimer(hwnd, TIMER);
                    ShowWindow(hwnd, SW_HIDE);
                }
            }
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

unsafe fn repaint(hwnd: HWND) -> bool {
    InvalidateRect(hwnd, null(), 0);
    UpdateWindow(hwnd);
    // WS_EX_TRANSPARENT can defer normal painting. Render the client explicitly
    // after releasing every State borrow, both for show and asynchronous updates.
    let dc = GetDC(hwnd);
    if dc.is_null() {
        return false;
    }
    SendMessageW(hwnd, WM_PRINTCLIENT, dc as usize, PRF_CLIENT as isize);
    ReleaseDC(hwnd, dc);
    ValidateRect(hwnd, null());
    true
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
    let left = scale(18, state.dpi);
    let top = scale(22, state.dpi);
    let size = scale(40, state.dpi);
    SelectObject(dc, pen);
    SelectObject(dc, brush);
    SetBkMode(dc, TRANSPARENT as i32);
    let pixels = icon_size(state.dpi) as i32;
    let icon = state
        .icon
        .as_ref()
        .map_or_else(|| state.fallback.raw(), |icon| icon.0);
    DrawIconEx(
        dc,
        left + (size - pixels) / 2,
        top + (size - pixels) / 2,
        icon,
        pixels,
        pixels,
        0,
        null_mut(),
        DI_NORMAL,
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
        &wide(&tr(SUBTITLE)),
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

// Windows timers use integer milliseconds. Round up so the first tick cannot
// precede the deadline just because converting Duration truncated a fraction.
fn timer_interval(remaining: Duration) -> u32 {
    remaining
        .as_nanos()
        .div_ceil(1_000_000)
        .clamp(1, u32::MAX as u128) as u32
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
    wide(&fmt(
        "Lancement de {title}…",
        &[("title", title.trim().into())],
    ))
}

fn scale(dip: i32, dpi: i32) -> i32 {
    (dip * dpi + 48) / 96
}

pub fn icon_size(dpi: i32) -> u32 {
    ((32i64 * i64::from(dpi) + 48) / 96).clamp(16, 128) as u32
}

fn color(r: u32, g: u32, b: u32) -> COLORREF {
    r | (g << 8) | (b << 16)
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
    use windows_sys::Win32::UI::HiDpi::{
        SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_UNAWARE,
    };

    struct TestBitmap(HBITMAP);

    impl Drop for TestBitmap {
        fn drop(&mut self) {
            unsafe { DeleteObject(self.0) };
        }
    }

    fn test_dib(width: i32, height: i32) -> (TestBitmap, *mut u32) {
        unsafe {
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut pixels = null_mut();
            let bitmap = TestBitmap(CreateDIBSection(
                null_mut(),
                &info,
                DIB_RGB_COLORS,
                &mut pixels,
                null_mut(),
                0,
            ));
            assert!(!bitmap.0.is_null() && !pixels.is_null());
            (bitmap, pixels.cast())
        }
    }

    fn fixture_pixel(x: usize, y: usize, replacement: bool) -> Option<u32> {
        let colors = if replacement {
            [0xFF32_76E8, 0xFFB3_67DE, 0xFF41_C88D]
        } else {
            [0xFFD3_4476, 0xFF12_A8B0, 0xFFFF_B347]
        };
        if (2..10).contains(&x) && (4..12).contains(&y) {
            Some(colors[0])
        } else if (18..30).contains(&x) && (14..28).contains(&y) {
            Some(colors[1])
        } else if (14..17).contains(&x) && (8..24).contains(&y) {
            Some(colors[2])
        } else {
            None
        }
    }

    fn fixture_icon(replacement: bool) -> OwnedIcon {
        let (color, pixels) = test_dib(32, 32);
        let mut mask_bytes = [0xFFu8; 128];
        unsafe {
            let pixels = std::slice::from_raw_parts_mut(pixels, 32 * 32);
            for y in 0..32 {
                for x in 0..32 {
                    pixels[y * 32 + x] = fixture_pixel(x, y, replacement).unwrap_or(0);
                    if fixture_pixel(x, y, replacement).is_some() {
                        mask_bytes[y * 4 + x / 8] &= !(0x80 >> (x % 8));
                    }
                }
            }
            let mask = TestBitmap(CreateBitmap(32, 32, 1, 1, mask_bytes.as_ptr().cast()));
            assert!(!mask.0.is_null());
            let icon = CreateIconIndirect(&ICONINFO {
                fIcon: 1,
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: mask.0,
                hbmColor: color.0,
            });
            assert!(!icon.is_null());
            OwnedIcon(icon)
        }
    }

    struct Capture {
        dc: HDC,
        old: HGDIOBJ,
        bitmap: TestBitmap,
        pixels: *mut u32,
        width: usize,
        height: usize,
    }

    impl Capture {
        fn new(hwnd: HWND) -> Self {
            unsafe {
                let mut bounds: RECT = zeroed();
                assert_ne!(GetClientRect(hwnd, &mut bounds), 0);
                let (bitmap, pixels) = test_dib(bounds.right, bounds.bottom);
                let dc = CreateCompatibleDC(null_mut());
                assert!(!dc.is_null());
                let old = SelectObject(dc, bitmap.0);
                assert!(!old.is_null());
                Self {
                    dc,
                    old,
                    bitmap,
                    pixels,
                    width: bounds.right as usize,
                    height: bounds.bottom as usize,
                }
            }
        }

        fn paint(&self, hwnd: HWND) {
            unsafe {
                SendMessageW(hwnd, WM_PRINTCLIENT, self.dc as usize, PRF_CLIENT as isize);
                assert_ne!(GdiFlush(), 0);
            }
        }

        fn pixels(&self) -> &[u32] {
            unsafe { std::slice::from_raw_parts(self.pixels, self.width * self.height) }
        }

        fn export(&self, name: &str) {
            let Some(directory) =
                std::env::var_os("SUPER_MENU_LAUNCH_TEST_DIR").filter(|value| !value.is_empty())
            else {
                return;
            };
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let mut bytes = Vec::with_capacity(54 + self.pixels().len() * 4);
            bytes.extend_from_slice(b"BM");
            bytes.extend_from_slice(&((54 + self.pixels().len() * 4) as u32).to_le_bytes());
            bytes.extend_from_slice(&[0; 4]);
            bytes.extend_from_slice(&54u32.to_le_bytes());
            bytes.extend_from_slice(&40u32.to_le_bytes());
            bytes.extend_from_slice(&(self.width as i32).to_le_bytes());
            bytes.extend_from_slice(&(-(self.height as i32)).to_le_bytes());
            bytes.extend_from_slice(&1u16.to_le_bytes());
            bytes.extend_from_slice(&32u16.to_le_bytes());
            bytes.extend_from_slice(&[0; 24]);
            for pixel in self.pixels() {
                bytes.extend_from_slice(&pixel.to_le_bytes());
            }
            std::fs::write(directory.join(format!("launch-{name}.bmp")), bytes).unwrap();
        }
    }

    impl Drop for Capture {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.old);
                DeleteDC(self.dc);
            }
        }
    }

    fn assert_live_icon(icon: HICON) {
        unsafe {
            let mut info: ICONINFO = zeroed();
            assert_ne!(GetIconInfo(icon, &mut info), 0);
            let _color = TestBitmap(info.hbmColor);
            let _mask = TestBitmap(info.hbmMask);
        }
    }

    fn assert_released_icon(icon: HICON) {
        unsafe {
            let mut info: ICONINFO = zeroed();
            assert_eq!(GetIconInfo(icon, &mut info), 0, "owned icon released");
        }
    }

    fn verify_icon_pixels(capture: &Capture, background: COLORREF, replacement: Option<bool>) {
        let background =
            ((background & 255) << 16) | (background & 0xFF00) | ((background >> 16) & 255);
        let pixels = capture.pixels();
        // 32px artwork is centered inside the 40px area at (18,22): its
        // origin is (22,26), and transparent padding must remain background.
        for y in 22..62 {
            for x in 18..58 {
                let expected = if (22..54).contains(&x) && (26..58).contains(&y) {
                    let (icon_x, icon_y) = (x - 22, y - 26);
                    match replacement {
                        Some(replacement) => fixture_pixel(icon_x, icon_y, replacement),
                        None => {
                            let occupied = |point| {
                                [(2..10), (12..20), (22..30)]
                                    .iter()
                                    .any(|range| range.contains(&point))
                            };
                            (occupied(icon_x) && occupied(icon_y)).then_some(0xFF8A_62EC)
                        }
                    }
                } else {
                    None
                };
                assert_eq!(
                    pixels[y * capture.width + x] & 0x00FF_FFFF,
                    expected.unwrap_or(background) & 0x00FF_FFFF,
                    "pixel {x},{y}"
                );
            }
        }
    }

    #[test]
    fn application_icon_is_copied_centered_transparent_and_never_extends_feedback() {
        let _ui_test_guard = crate::native::UI_TEST_LOCK.lock().unwrap();
        struct DpiContext(DPI_AWARENESS_CONTEXT);
        impl Drop for DpiContext {
            fn drop(&mut self) {
                unsafe { SetThreadDpiAwarenessContext(self.0) };
            }
        }
        let previous = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_UNAWARE) };
        assert!(!previous.is_null());
        let _dpi_context = DpiContext(previous);
        let anchor = LaunchFeedback::new().unwrap();
        let mut popup = LaunchFeedback::new().unwrap();
        let foreground = unsafe { GetForegroundWindow() };
        for theme in ["dark", "light"] {
            let settings = config::Settings {
                theme: theme.into(),
                ..config::Settings::default()
            };
            let source = fixture_icon(false);
            let source_raw = source.0;
            popup
                .show(
                    "Icône de l’application",
                    Some(source_raw),
                    anchor.hwnd(),
                    &settings,
                )
                .unwrap();
            assert_eq!(popup.state.borrow().dpi, 96);
            assert!(popup.is_active());
            let copied = popup.state.borrow().icon.as_ref().unwrap().0;
            assert_ne!(source_raw, copied);
            drop(source);
            assert_released_icon(source_raw);
            assert_live_icon(copied);
            let capture = Capture::new(popup.hwnd());
            assert_eq!((capture.width, capture.height), (480, 84));
            capture.paint(popup.hwnd());
            verify_icon_pixels(&capture, Palette::from(&settings).background, Some(false));
            capture.export(&format!("{theme}-application"));
            let deadline = popup.state.borrow().deadline;
            let title = popup.state.borrow().title.clone();
            assert!(!popup.update_icon(null_mut()));
            assert_eq!(popup.state.borrow().icon.as_ref().unwrap().0, copied);
            let replacement = fixture_icon(true);
            let replacement_source = replacement.0;
            assert!(popup.update_icon(replacement_source));
            let replacement_copy = popup.state.borrow().icon.as_ref().unwrap().0;
            assert_ne!(replacement_copy, replacement_source);
            drop(replacement);
            assert_released_icon(replacement_source);
            assert_released_icon(copied);
            assert_live_icon(replacement_copy);
            assert_eq!(popup.state.borrow().deadline, deadline);
            assert_eq!(popup.state.borrow().title, title);
            assert_eq!(unsafe { GetForegroundWindow() }, foreground);
            assert_ne!(unsafe { IsWindowVisible(popup.hwnd()) }, 0);
            capture.paint(popup.hwnd());
            verify_icon_pixels(&capture, Palette::from(&settings).background, Some(true));
            capture.export(&format!("{theme}-replacement"));
            unsafe { SendMessageW(popup.hwnd(), WM_TIMER, TIMER, 0) };
            assert_eq!(popup.state.borrow().deadline, deadline);
            let expired = Instant::now() - Duration::from_millis(1);
            popup.state.borrow_mut().deadline = Some(expired);
            let late = fixture_icon(false);
            assert!(!popup.is_active());
            assert!(!popup.update_icon(late.0));
            assert_eq!(popup.state.borrow().deadline, Some(expired));
            assert_eq!(
                popup.state.borrow().icon.as_ref().unwrap().0,
                replacement_copy
            );
            unsafe { SendMessageW(popup.hwnd(), WM_TIMER, TIMER, 0) };
            assert_eq!(unsafe { IsWindowVisible(popup.hwnd()) }, 0);
            assert!(!popup.is_active());
            assert!(popup.state.borrow().icon.is_none());
            assert_released_icon(replacement_copy);
            assert!(!popup.update_icon(late.0));
            assert_eq!(unsafe { IsWindowVisible(popup.hwnd()) }, 0);
            assert_eq!(popup.state.borrow().deadline, None);
            assert_eq!(popup.state.borrow().title, title);
            popup
                .show("Icône par défaut", None, anchor.hwnd(), &settings)
                .unwrap();
            capture.paint(popup.hwnd());
            verify_icon_pixels(&capture, Palette::from(&settings).background, None);
            capture.export(&format!("{theme}-fallback"));
            popup.hide();
            let capture_bitmap = capture.bitmap.0;
            drop(capture);
            unsafe {
                let mut details: BITMAP = zeroed();
                assert_eq!(
                    GetObjectW(
                        capture_bitmap,
                        size_of::<BITMAP>() as i32,
                        (&mut details as *mut BITMAP).cast()
                    ),
                    0
                );
            }
        }
        let hwnd = popup.hwnd();
        let fallback = popup.state.borrow().fallback.raw();
        drop(popup);
        assert_eq!(unsafe { IsWindow(hwnd) }, 0);
        assert_released_icon(fallback);
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);
    }

    #[test]
    fn native_timer_interval_rounds_up_to_the_deadline() {
        assert_eq!(timer_interval(Duration::ZERO), 1);
        assert_eq!(timer_interval(Duration::from_nanos(1)), 1);
        assert_eq!(timer_interval(Duration::from_millis(1)), 1);
        assert_eq!(timer_interval(Duration::from_nanos(1_000_001)), 2);
        assert_eq!(timer_interval(Duration::from_micros(1_999_001)), 2000);
        assert_eq!(timer_interval(DURATION), 2000);
    }
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
        let light = Palette::from(&settings);
        assert_ne!(dark.background, light.background);
        assert_ne!(dark.text, light.text);
        for (dpi, expected) in [
            (i32::MIN, 16),
            (48, 16),
            (96, 32),
            (144, 48),
            (192, 64),
            (384, 128),
            (i32::MAX, 128),
        ] {
            assert_eq!(icon_size(dpi), expected);
        }
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
        let _ui_test_guard = crate::native::UI_TEST_LOCK.lock().unwrap();
        let anchor = LaunchFeedback::new().unwrap();
        let mut popup = LaunchFeedback::new().unwrap();
        let settings = config::Settings::default();
        let foreground = unsafe { GetForegroundWindow() };
        popup
            .show("Éditeur 🦀", None, anchor.hwnd(), &settings)
            .unwrap();
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
            .show("Deuxième lancement", None, anchor.hwnd(), &settings)
            .unwrap();
        let replacement_deadline = popup.state.borrow().deadline;
        unsafe {
            // Simulate an already queued timer from the preceding request.
            SendMessageW(hwnd, WM_TIMER, TIMER, 0);
            assert_ne!(IsWindowVisible(hwnd), 0);
        }
        assert_eq!(popup.state.borrow().deadline, replacement_deadline);
        let mut caption = [0u16; 256];
        let length = unsafe { GetWindowTextW(hwnd, caption.as_mut_ptr(), caption.len() as i32) };
        assert_eq!(
            String::from_utf16(&caption[..length as usize]).unwrap(),
            "Lancement de Deuxième lancement…"
        );
        // The requested two-second feedback must expire before a longer delay.
        let limit = Instant::now() + Duration::from_secs(3);
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
