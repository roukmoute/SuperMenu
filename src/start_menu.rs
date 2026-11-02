//! Dismiss an already open Windows Start menu before showing the launcher.
//! Ordinary activation does not create COM objects or wait for key release.

use crate::com::{self, ComPtr, IAppVisibility};
use std::mem::{size_of, zeroed};
use std::ptr::null;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, HWND};
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU,
    VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SPACE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId, IsWindowVisible,
};

const TIMEOUT: Duration = Duration::from_secs(2);

#[cfg_attr(test, derive(Debug))]
#[derive(PartialEq, Eq)]
pub(crate) enum Step {
    Waiting,
    Show,
    Cancel,
}

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

pub(crate) struct Dismissal {
    target: HWND,
    process: u32,
    started: Instant,
    escape_sent: bool,
    // Rust drops fields in declaration order: release the interface before COM.
    visibility: ComPtr<IAppVisibility>,
    _apartment: Apartment,
}

/// The foreground class check keeps ordinary opens on a short Win32-only path.
pub(crate) fn begin() -> Option<Dismissal> {
    unsafe {
        let target = GetForegroundWindow();
        if !core_window(target) || IsWindowVisible(target) == 0 {
            return None;
        }
        let started = Instant::now();
        let mut process = 0;
        GetWindowThreadProcessId(target, &mut process);
        if process == 0 || !start_process(process) {
            return None;
        }
        // S_FALSE also increments the apartment count and must be balanced.
        if CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) < 0 {
            return None;
        }
        let apartment = Apartment;
        let visibility =
            com::create_instance::<IAppVisibility>(&com::AppVisibility, CLSCTX_INPROC_SERVER)
                .ok()?;
        if !visibility.IsLauncherVisible().ok()? {
            return None;
        }
        // A foreground change after this positive identification cancels in
        // poll(), rather than falling through to an ordinary launcher show.
        Some(Dismissal {
            target,
            process,
            started,
            escape_sent: false,
            visibility,
            _apartment: apartment,
        })
    }
}

impl Dismissal {
    /// Called only while Start dismissal is pending; never blocks on key release.
    pub(crate) fn poll(&mut self) -> Step {
        unsafe {
            if self.started.elapsed() >= TIMEOUT {
                return Step::Cancel;
            }
            let visible = self.visibility.IsLauncherVisible().ok();
            let foreground_matches = if self.escape_sent {
                let foreground = GetForegroundWindow();
                if foreground.is_null() {
                    return Step::Waiting;
                }
                foreground == self.target
            } else {
                self.target_is_foreground()
            };
            match decision(
                self.started.elapsed(),
                self.escape_sent,
                visible,
                foreground_matches,
                chord_held(),
            ) {
                Decision::Waiting => Step::Waiting,
                Decision::Show => Step::Show,
                Decision::Cancel => Step::Cancel,
                Decision::Escape => {
                    // COM may have pumped messages. Check again immediately before
                    // global input; never release/repress the user's modifiers.
                    if self.started.elapsed() >= TIMEOUT || !self.target_is_foreground() {
                        return Step::Cancel;
                    }
                    if chord_held() {
                        return Step::Waiting;
                    }
                    let escape = [escape_input(false), escape_input(true)];
                    if GetForegroundWindow() != self.target {
                        return Step::Cancel;
                    }
                    // Never retry a partial send: Escape may already be queued.
                    self.escape_sent = true;
                    let sent = SendInput(2, escape.as_ptr(), size_of::<INPUT>() as i32);
                    if sent != 2 {
                        if sent == 1 {
                            SendInput(1, escape[1..].as_ptr(), size_of::<INPUT>() as i32);
                        }
                        return Step::Cancel;
                    }
                    Step::Waiting
                }
            }
        }
    }

    unsafe fn target_is_foreground(&self) -> bool {
        if GetForegroundWindow() != self.target
            || IsWindowVisible(self.target) == 0
            || !core_window(self.target)
        {
            return false;
        }
        let mut process = 0;
        GetWindowThreadProcessId(self.target, &mut process);
        process == self.process && GetForegroundWindow() == self.target
    }
}

#[cfg_attr(test, derive(Debug))]
#[derive(PartialEq, Eq)]
enum Decision {
    Waiting,
    Escape,
    Show,
    Cancel,
}

fn decision(
    elapsed: Duration,
    escape_sent: bool,
    visible: Option<bool>,
    foreground_matches: bool,
    chord_held: bool,
) -> Decision {
    if elapsed >= TIMEOUT || visible.is_none() {
        return Decision::Cancel;
    }
    if escape_sent {
        if visible == Some(false) && !foreground_matches {
            Decision::Show
        } else {
            Decision::Waiting
        }
    } else if visible != Some(true) || !foreground_matches {
        Decision::Cancel
    } else if chord_held {
        Decision::Waiting
    } else {
        Decision::Escape
    }
}

unsafe fn chord_held() -> bool {
    [
        VK_LWIN,
        VK_RWIN,
        VK_CONTROL,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_SHIFT,
        VK_LSHIFT,
        VK_RSHIFT,
        VK_MENU,
        VK_LMENU,
        VK_RMENU,
        VK_SPACE,
    ]
    .iter()
    .any(|key| GetAsyncKeyState(*key as i32) < 0)
}

unsafe fn escape_input(up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE,
                dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                ..zeroed()
            },
        },
    }
}

unsafe fn core_window(window: HWND) -> bool {
    if window.is_null() {
        return false;
    }
    let mut class = [0u16; 64];
    let length = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
    length > 0
        && class[..length as usize]
            .iter()
            .copied()
            .eq("Windows.UI.Core.CoreWindow".encode_utf16())
}

unsafe fn start_process(process: u32) -> bool {
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process);
    if handle.is_null() {
        return false;
    }
    let mut image = [0u16; 1024];
    let mut length = image.len() as u32;
    let queried = QueryFullProcessImageNameW(handle, 0, image.as_mut_ptr(), &mut length);
    CloseHandle(handle);
    if queried == 0 || length as usize > image.len() {
        return false;
    }
    let Ok(image) = String::from_utf16(&image[..length as usize]) else {
        return false;
    };
    let mut directory = [0u16; 1024];
    let length = GetWindowsDirectoryW(directory.as_mut_ptr(), directory.len() as u32) as usize;
    if length == 0 || length >= directory.len() {
        return false;
    }
    let Ok(directory) = String::from_utf16(&directory[..length]) else {
        return false;
    };
    start_image(&image, &directory)
}

fn start_image(image: &str, windows: &str) -> bool {
    let windows = windows.trim_end_matches('\\');
    image
        .get(..windows.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(windows))
        && image.get(windows.len()..).is_some_and(|suffix| {
            [
                "\\SystemApps\\Microsoft.Windows.StartMenuExperienceHost_cw5n1h2txyewy\\StartMenuExperienceHost.exe",
                "\\SystemApps\\ShellExperienceHost_cw5n1h2txyewy\\ShellExperienceHost.exe",
                // Windows can give the Start search surface foreground focus
                // while IAppVisibility still reports the launcher as visible.
                "\\SystemApps\\MicrosoftWindows.Client.CBS_cw5n1h2txyewy\\SearchHost.exe",
            ]
            .iter()
            .any(|expected| suffix.eq_ignore_ascii_case(expected))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "optional read-only measurement with Start closed"]
    fn closed_start_fast_path_probe() {
        let _guard = crate::native::UI_TEST_LOCK.lock().unwrap();
        let iterations = 10_000;
        let started = Instant::now();
        for _ in 0..iterations {
            assert!(
                begin().is_none(),
                "close Start before measuring its negative path"
            );
        }
        println!(
            "Start-closed detection: {:?} per activation, {iterations} samples",
            started.elapsed() / iterations
        );
    }

    /// Explicit real-desktop validation, excluded from the isolated test suites.
    #[test]
    #[ignore = "requires SUPER_MENU_START_PROBE=1 and Start opened manually"]
    fn real_start_dismissal_probe() {
        assert_eq!(std::env::var("SUPER_MENU_START_PROBE").as_deref(), Ok("1"));
        let _guard = crate::native::UI_TEST_LOCK.lock().unwrap();
        let waiting = Instant::now();
        let mut previous = std::ptr::null_mut();
        let mut dismissal = loop {
            unsafe {
                let foreground = GetForegroundWindow();
                if foreground != previous {
                    previous = foreground;
                    let mut class = [0u16; 128];
                    let size = GetClassNameW(foreground, class.as_mut_ptr(), class.len() as i32);
                    let mut process = 0;
                    GetWindowThreadProcessId(foreground, &mut process);
                    let initialized = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) >= 0;
                    let visible = if initialized {
                        let result = com::create_instance::<IAppVisibility>(
                            &com::AppVisibility,
                            CLSCTX_INPROC_SERVER,
                        )
                        .and_then(|visibility| visibility.IsLauncherVisible());
                        CoUninitialize();
                        result.ok()
                    } else {
                        None
                    };
                    println!("Foreground class {:?}, process {process}, trusted Start host {}, Start visible {visible:?}",
                        String::from_utf16_lossy(&class[..size.max(0) as usize]), start_process(process));
                }
            }
            if let Some(dismissal) = begin() {
                break dismissal;
            }
            assert!(
                waiting.elapsed() < Duration::from_secs(45),
                "Start was not detected during the manual probe"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        let started = Instant::now();
        loop {
            match dismissal.poll() {
                Step::Waiting => std::thread::sleep(Duration::from_millis(10)),
                Step::Show => {
                    println!(
                        "Start closed before launcher activation in {:?}",
                        started.elapsed()
                    );
                    break;
                }
                Step::Cancel => panic!("Start dismissal cancelled after {:?}", started.elapsed()),
            }
        }
    }

    #[test]
    fn only_the_windows_start_hosts_are_candidates() {
        let start = "C:\\Windows\\SystemApps\\Microsoft.Windows.StartMenuExperienceHost_cw5n1h2txyewy\\StartMenuExperienceHost.exe";
        let shell =
            "C:\\Windows\\SystemApps\\ShellExperienceHost_cw5n1h2txyewy\\ShellExperienceHost.exe";
        let search =
            "C:\\Windows\\SystemApps\\MicrosoftWindows.Client.CBS_cw5n1h2txyewy\\SearchHost.exe";
        assert!(start_image(start, "C:\\Windows"));
        assert!(start_image(shell, "C:\\Windows\\"));
        assert!(start_image(search, "C:\\Windows"));
        assert!(start_image(&start.to_ascii_uppercase(), "c:\\windows"));
        for image in [
            "C:\\Temp\\StartMenuExperienceHost.exe".to_owned(),
            start.replace("C:\\Windows", "C:\\Windows-other"),
            start.replace("StartMenuExperienceHost.exe", "SearchHost.exe"),
            format!("{start}.bak"),
        ] {
            assert!(!start_image(&image, "C:\\Windows"), "{image}");
        }
        assert!(!start_image(start, "D:\\Windows"));
    }

    #[test]
    fn escape_waits_for_release_and_is_never_repeated() {
        let elapsed = Duration::from_millis(10);
        assert_eq!(
            decision(elapsed, false, Some(true), true, true),
            Decision::Waiting
        );
        assert_eq!(
            decision(elapsed, false, Some(true), true, false),
            Decision::Escape
        );
        assert_eq!(
            decision(elapsed, true, Some(true), true, false),
            Decision::Waiting
        );
        assert_eq!(
            decision(elapsed, true, Some(false), true, false),
            Decision::Waiting
        );
        assert_eq!(
            decision(elapsed, true, Some(true), false, false),
            Decision::Waiting
        );
        assert_eq!(
            decision(elapsed, true, Some(false), false, false),
            Decision::Show
        );
    }

    #[test]
    fn a_foreground_change_or_uncertain_start_state_cancels_before_escape() {
        for visible in [Some(false), None] {
            assert_eq!(
                decision(Duration::ZERO, false, visible, true, false),
                Decision::Cancel
            );
        }
        assert_eq!(
            decision(Duration::ZERO, false, Some(true), false, false),
            Decision::Cancel
        );
        for sent in [false, true] {
            assert_eq!(
                decision(TIMEOUT, sent, Some(false), false, false),
                Decision::Cancel
            );
            assert_eq!(
                decision(Duration::ZERO, sent, None, false, false),
                Decision::Cancel
            );
        }
    }
}
