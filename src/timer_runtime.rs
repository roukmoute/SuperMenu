//! UI-thread countdown service that remains available in the menu's modal loops.

use crate::{config, timer, timer_alert};
use std::cell::RefCell;
use std::ptr::null_mut;
use std::rc::Rc;
use std::time::Instant;
use windows_sys::Win32::{
    Foundation::HWND,
    System::Diagnostics::Debug::MessageBeep,
    UI::WindowsAndMessaging::{
        KillTimer, MessageBoxW, SetTimer, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_TOPMOST,
        WM_POWERBROADCAST, WM_TIMER,
    },
};

const TIMER_TICK: usize = 43;
// Only needed if a nested Win32 loop reenters this service itself.
const RETRY_TICK: usize = 44;

struct ActiveRuntime {
    hwnd: HWND,
    runtime: Rc<RefCell<TimerRuntime>>,
}

#[derive(Default)]
struct DeferredEvents {
    tick: bool,
    acknowledgements: Vec<usize>,
}

impl DeferredEvents {
    fn add(&mut self, message: u32, parameter: usize) {
        if message == timer_alert::WM_TIMER_ALERT_DISMISSED {
            if !self.acknowledgements.contains(&parameter) {
                self.acknowledgements.push(parameter);
            }
        } else {
            self.tick = true;
        }
    }
}

thread_local! {
    static ACTIVE_RUNTIME: RefCell<Option<ActiveRuntime>> = const { RefCell::new(None) };
    static DEFERRED: RefCell<DeferredEvents> = const { RefCell::new(DeferredEvents {
        tick: false,
        acknowledgements: Vec::new(),
    }) };
}

pub struct TimerRuntime {
    pub book: timer::TimerBook,
    pub settings: config::Settings,
    pub hwnd: HWND,
    pub smoke: bool,
    alert: Option<timer_alert::TimerAlert>,
    alert_ids: Vec<timer::TimerId>,
    alert_generation: usize,
    scheduling_failed: bool,
}

impl TimerRuntime {
    pub fn new(settings: config::Settings, smoke: bool) -> Self {
        Self {
            book: timer::TimerBook::new(),
            settings,
            hwnd: null_mut(),
            smoke,
            alert: timer_alert::TimerAlert::new().ok(),
            alert_ids: Vec::new(),
            alert_generation: 0,
            scheduling_failed: false,
        }
    }

    pub fn tick(&mut self) {
        let alerts = self.book.update(Instant::now());
        if !alerts.is_empty() {
            if !self.smoke {
                unsafe { MessageBeep(MB_ICONINFORMATION) };
            }
            self.refresh_alert();
        }
        self.synchronize();
    }

    pub fn synchronize(&mut self) {
        if self.hwnd.is_null() {
            return;
        }
        if let Some(deadline) = self.book.next_deadline() {
            let milliseconds = deadline
                .saturating_duration_since(Instant::now())
                .as_nanos()
                .div_ceil(1_000_000)
                .clamp(1, 1000) as u32;
            if unsafe { SetTimer(self.hwnd, TIMER_TICK, milliseconds, None) } == 0 {
                if !self.scheduling_failed {
                    self.scheduling_failed = true;
                    // Do not leave an apparently active countdown without a wake-up.
                    let now = Instant::now();
                    let running: Vec<_> = self
                        .book
                        .snapshots(now)
                        .into_iter()
                        .filter(|snapshot| snapshot.status == timer::TimerStatus::Running)
                        .map(|snapshot| snapshot.id)
                        .collect();
                    for id in running {
                        let _ = self.book.pause(id, now);
                    }
                    unsafe { KillTimer(self.hwnd, TIMER_TICK) };
                    unsafe {
                        MessageBoxW(
                            self.hwnd,
                            crate::native::wide("Minuteurs mis en pause : Windows ne peut pas planifier leur alerte. Reprends-les depuis la liste des minuteurs pour réessayer.").as_ptr(),
                            crate::native::wide("Super Menu · Minuteur").as_ptr(),
                            MB_OK | MB_ICONERROR,
                        );
                    }
                }
            } else {
                self.scheduling_failed = false;
            }
        } else {
            unsafe { KillTimer(self.hwnd, TIMER_TICK) };
            self.scheduling_failed = false;
        }
    }

    pub fn refresh_alert(&mut self) {
        let finished: Vec<_> = self
            .book
            .snapshots(Instant::now())
            .into_iter()
            .filter(|snapshot| snapshot.status == timer::TimerStatus::Finished)
            .collect();
        let ids: Vec<_> = finished.iter().map(|snapshot| snapshot.id).collect();
        if ids == self.alert_ids {
            return;
        }
        self.alert_ids = ids;
        if finished.is_empty() {
            if let Some(alert) = &mut self.alert {
                alert.hide();
            }
            self.alert_generation = 0;
            return;
        }
        let labels: Vec<_> = finished
            .into_iter()
            .map(|snapshot| snapshot.label)
            .collect();
        if let Some(alert) = &mut self.alert {
            if alert.show(&labels, self.hwnd, &self.settings).is_ok() {
                self.alert_generation = alert.generation();
                return;
            }
        }
        // Preserve a visible report even if the passive panel cannot be created.
        unsafe {
            MessageBoxW(
                self.hwnd,
                crate::native::wide(&labels.join("\n")).as_ptr(),
                crate::native::wide("Minuteur terminé").as_ptr(),
                MB_OK | MB_TOPMOST,
            );
        }
        let now = Instant::now();
        for id in self.alert_ids.drain(..) {
            let _ = self.book.acknowledge(id, now);
        }
        self.alert_generation = 0;
        if let Some(alert) = &mut self.alert {
            alert.hide();
        }
    }

    pub fn acknowledge_alert(&mut self, generation: usize) {
        if generation == 0 || generation != self.alert_generation {
            return;
        }
        let now = Instant::now();
        for id in self.alert_ids.drain(..) {
            let _ = self.book.acknowledge(id, now);
        }
        self.alert_generation = 0;
        if let Some(alert) = &mut self.alert {
            alert.hide();
        }
        self.synchronize();
    }

    pub fn shutdown(&mut self) {
        if !self.hwnd.is_null() {
            unsafe {
                KillTimer(self.hwnd, TIMER_TICK);
                KillTimer(self.hwnd, RETRY_TICK);
            }
        }
        self.alert.take();
        self.alert_ids.clear();
        self.alert_generation = 0;
        self.book = timer::TimerBook::new();
        self.hwnd = null_mut();
    }

    pub fn alert_visible(&self) -> bool {
        self.alert
            .as_ref()
            .is_some_and(timer_alert::TimerAlert::is_visible)
    }

    pub fn alert_hwnd(&self) -> HWND {
        self.alert
            .as_ref()
            .map_or(null_mut(), timer_alert::TimerAlert::hwnd)
    }
}

pub fn bind(runtime: &Rc<RefCell<TimerRuntime>>, hwnd: HWND) {
    unbind();
    runtime.borrow_mut().hwnd = hwnd;
    ACTIVE_RUNTIME.with(|active| {
        *active.borrow_mut() = Some(ActiveRuntime {
            hwnd,
            runtime: Rc::clone(runtime),
        });
    });
}

pub fn unbind() {
    let previous = ACTIVE_RUNTIME.with(|active| active.borrow_mut().take());
    if let Some(previous) = previous {
        unsafe {
            KillTimer(previous.hwnd, TIMER_TICK);
            KillTimer(previous.hwnd, RETRY_TICK);
        }
    }
    DEFERRED.with(|deferred| *deferred.borrow_mut() = DeferredEvents::default());
}

/// Call before borrowing the menu State; true means the callback was consumed.
pub fn handle_message(hwnd: HWND, message: u32, parameter: usize) -> bool {
    let recognized = message == timer_alert::WM_TIMER_ALERT_DISMISSED
        || message == WM_TIMER && matches!(parameter, TIMER_TICK | RETRY_TICK)
        || message == WM_POWERBROADCAST && matches!(parameter, 7 | 18);
    if !recognized {
        return false;
    }
    let runtime = ACTIVE_RUNTIME.with(|active| {
        active
            .borrow()
            .as_ref()
            .filter(|active| active.hwnd == hwnd)
            .map(|active| Rc::clone(&active.runtime))
    });
    let Some(runtime) = runtime else {
        return false;
    };
    let Ok(mut runtime) = runtime.try_borrow_mut() else {
        if message != WM_TIMER || parameter != RETRY_TICK {
            DEFERRED.with(|deferred| deferred.borrow_mut().add(message, parameter));
        }
        // A periodic retry cannot flood a nested modal loop like PostMessage can.
        unsafe { SetTimer(hwnd, RETRY_TICK, 50, None) };
        return true;
    };
    unsafe { KillTimer(hwnd, RETRY_TICK) };
    let mut deferred = DEFERRED.with(|deferred| std::mem::take(&mut *deferred.borrow_mut()));
    if message != WM_TIMER || parameter != RETRY_TICK {
        deferred.add(message, parameter);
    }
    // A dismissal applies to the displayed generation before a new expiry is shown.
    for generation in deferred.acknowledgements {
        runtime.acknowledge_alert(generation);
    }
    if deferred.tick {
        runtime.tick();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_callbacks_coalesce_ticks_and_preserve_distinct_acknowledgements() {
        let mut events = DeferredEvents::default();
        events.add(WM_TIMER, TIMER_TICK);
        events.add(WM_POWERBROADCAST, 18);
        events.add(timer_alert::WM_TIMER_ALERT_DISMISSED, 7);
        events.add(timer_alert::WM_TIMER_ALERT_DISMISSED, 7);
        events.add(timer_alert::WM_TIMER_ALERT_DISMISSED, 8);
        assert!(events.tick);
        assert_eq!(events.acknowledgements, [7, 8]);
    }

    #[test]
    fn runtime_remains_available_while_menu_is_borrowed_and_defers_reentrant_dismissal() {
        use std::time::Duration;
        use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;

        let _ui_test_guard = crate::native::UI_TEST_LOCK.lock().unwrap();
        let anchor = timer_alert::TimerAlert::new().unwrap();
        let runtime = Rc::new(RefCell::new(TimerRuntime::new(
            config::Settings::default(),
            true,
        )));
        bind(&runtime, anchor.hwnd());
        let now = Instant::now();
        let id = runtime
            .borrow_mut()
            .book
            .start(timer::parse("1s Thé").unwrap(), now)
            .unwrap();
        // Force the pending transition without a real-time deadline or sleeping.
        assert_eq!(
            runtime
                .borrow_mut()
                .book
                .pause(id, now + Duration::from_secs(2)),
            Err(timer::TimerError::NotRunning)
        );
        let menu = RefCell::new(());
        let _menu_borrow = menu.borrow_mut();
        assert!(handle_message(anchor.hwnd(), WM_TIMER, TIMER_TICK));
        assert!(runtime.borrow().alert_visible());
        assert_eq!(runtime.borrow().book.snapshots(now).len(), 1);
        assert!(!handle_message(anchor.hwnd(), WM_TIMER, 99));
        assert!(!handle_message(null_mut(), WM_TIMER, TIMER_TICK));
        let generation = runtime.borrow().alert_generation;
        {
            let borrowed = runtime.borrow_mut();
            assert!(handle_message(
                anchor.hwnd(),
                timer_alert::WM_TIMER_ALERT_DISMISSED,
                generation
            ));
            assert!(borrowed.alert_visible());
            assert_eq!(borrowed.book.snapshots(now).len(), 1);
        }
        assert!(handle_message(anchor.hwnd(), WM_TIMER, RETRY_TICK));
        assert!(!runtime.borrow().alert_visible());
        assert!(runtime.borrow().book.snapshots(now).is_empty());
        let popup = runtime.borrow().alert_hwnd();
        runtime.borrow_mut().shutdown();
        unbind();
        assert_eq!(unsafe { IsWindow(popup) }, 0);
        assert!(!handle_message(anchor.hwnd(), WM_TIMER, TIMER_TICK));
    }
}
