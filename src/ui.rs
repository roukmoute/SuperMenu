use crate::{
    about, app_icons, brand_icon, catalog, config, core, i18n, installer, language_dialog,
    launch_feedback, native, packages, paint_buffer::PaintBuffer, steam, timer, timer_alert,
    timer_runtime,
};
use core::{Item, Kind};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant};
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
const LAUNCH_DONE: u32 = WM_APP + 6;
const ICONS_READY: u32 = WM_APP + 8;
const ABOUT: u32 = WM_APP + 9;
const SMOKE_STATE: u32 = WM_APP + 10;
const SMOKE_CTRL_BACKSPACE: u32 = WM_APP + 11;
const SMOKE_LANGUAGE_PICKER: u32 = WM_APP + 12;
const TIMER_TICK: usize = 43;
const EDIT_ID: usize = 100;
const ROW_TOP: i32 = 177;
const ROW_HEIGHT: i32 = 48;
const CLASS: &str = "SuperMenuWindow";
const CF_UNICODETEXT: u32 = 13;
thread_local! { static DEFERRED:Cell<u8>=const {Cell::new(0)}; }
thread_local! { static DEFERRED_INSTALL:RefCell<Vec<(usize,isize)>>=const {RefCell::new(Vec::new())}; }
thread_local! { static DEFERRED_REMOVE:RefCell<Vec<(usize,isize)>>=const {RefCell::new(Vec::new())}; }
thread_local! { static DEFERRED_TIMER_ACK:RefCell<Vec<usize>>=const {RefCell::new(Vec::new())}; }

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
    cached_ui: CachedUi,
    paint_text: RefCell<Vec<u16>>,
    paint_buffer: PaintBuffer,
    query_text: Vec<u16>,
    index: Arc<Mutex<Vec<Item>>>,
    steam_enabled: Arc<AtomicBool>,
    installed_names: Arc<Mutex<Vec<String>>>,
    removable_names: Arc<Mutex<Vec<String>>>,
    confirmed_absent_packages: Arc<Mutex<HashSet<String>>>,
    missing_packages: Vec<Item>,
    removable_packages: Vec<Item>,
    installations: Installations,
    removals: Installations,
    removal_refresh_after: HashMap<String, usize>,
    index_generation: usize,
    catalog_ready: Arc<AtomicUsize>,
    feedback: Option<launch_feedback::LaunchFeedback>,
    feedback_request: Option<usize>,
    feedback_artwork: Option<(String, app_icons::Key)>,
    timers: Rc<RefCell<timer_runtime::TimerRuntime>>,
    timer_focus: Option<timer::TimerId>,
    launches: LaunchRequests,
    launch_results: mpsc::Receiver<(usize, Result<(), String>)>,
    launch_sender: mpsc::Sender<(usize, Result<(), String>)>,
    indexing: Arc<AtomicBool>,
    usage: catalog::Usage,
    weights: HashMap<String, u32>,
    clipboard: catalog::ClipHistory,
    items: Vec<Item>,
    item_icon_keys: Vec<Option<app_icons::Key>>,
    item_icon_size: u32,
    selected: usize,
    query: String,
    section: String,
    timing: String,
    status: String,
    fonts: [HFONT; 6],
    edit_bg: HBRUSH,
    icon: HICON,
    app_icons: Option<app_icons::IconCache>,
    about: Option<about::AboutWindow>,
    hotkey: Option<native::Hotkey>,
    watcher: Option<native::Watcher>,
    dpi: i32,
    visible_rows: usize,
    previous: HWND,
    settings_open: bool,
    smoke: bool,
    smoke_fixture: bool,
    smoke_report: Option<String>,
    language_cache: Option<std::path::PathBuf>,
    fixture_activation_attempts: usize,
    feedback_test: bool,
    feedback_test_finished: bool,
    pending_refresh: bool,
    pending_file_refresh: bool,
    checks: Vec<(String, bool)>,
    started: Instant,
    first_frame_ms: f64,
}

// These providers and translated labels only change with settings/language.
// Keeping them prepared avoids normalization, translation locks and allocation
// for every keystroke or repaint.
#[derive(Default)]
struct CachedUi {
    navigation: Vec<Item>,
    commands: Vec<Item>,
    symbols: Vec<Item>,
    system: Vec<Item>,
    tagline: String,
    tabs: [String; 5],
    timer_shortcut: String,
    about_shortcut: String,
    hotkey: String,
    empty: String,
    empty_indexing: String,
    empty_hint: String,
    hints: [String; 4],
}

impl CachedUi {
    fn new(settings: &config::Settings) -> Self {
        Self {
            navigation: navigation(),
            commands: settings
                .commands
                .iter()
                .map(|command| {
                    let mut item = Item::new(
                        format!("command:{}", command.name),
                        &command.name,
                        &command.file_name,
                        "›",
                        Kind::Command,
                        &command.file_name,
                    );
                    item.arguments = command.arguments.clone();
                    item
                })
                .collect(),
            symbols: [
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
                    format!("{symbol}  {}", i18n::tr(title)),
                    i18n::tr("Entrée pour copier"),
                    symbol,
                    Kind::Symbol,
                    symbol,
                )
            })
            .collect(),
            system: system_items(),
            tagline: i18n::tr("/   À portée de clavier"),
            tabs: [
                i18n::tr("Accueil"),
                format!("{}  @", i18n::tr("Applications")),
                format!("{}  /", i18n::tr("Fichiers")),
                format!("{}  ?", i18n::tr("Web")),
                i18n::tr("Modules  ;"),
            ],
            timer_shortcut: i18n::tr("minuteur"),
            about_shortcut: format!("; {}", i18n::tr("À propos")),
            hotkey: settings.hotkey.replace('+', " + "),
            empty: i18n::tr("Aucun résultat"),
            empty_indexing: i18n::tr("L'index se construit en arrière-plan…"),
            empty_hint: i18n::tr("Essaie un autre mot ou change de module avec ;"),
            hints: [
                "↑ ↓ naviguer   ↵ ouvrir   Échap fermer   F1 aide",
                "↑ ↓ naviguer   ↵ installer   Échap fermer   F1 aide",
                "↑ ↓ naviguer   ↵ désinstaller   Échap fermer",
                "↑ ↓ naviguer   ↵ sélectionner   Échap fermer",
            ]
            .map(i18n::tr),
        }
    }
}

#[cfg_attr(test, derive(Debug))]
#[derive(PartialEq, Eq)]
enum LaunchStart {
    Started(usize),
    Duplicate(usize),
}

#[derive(Default)]
struct LaunchRequests {
    pending: HashMap<usize, Item>,
    recent: HashMap<String, (usize, Instant)>,
    next_request: usize,
}

impl LaunchRequests {
    fn start(&mut self, item: &Item, now: Instant) -> Result<LaunchStart, &'static str> {
        self.recent.retain(|_, (_, started)| {
            now.saturating_duration_since(*started) < launch_feedback::DURATION
        });
        let duplicate = self
            .pending
            .iter()
            .find(|(_, pending)| pending.id == item.id)
            .map(|(token, _)| *token)
            .or_else(|| self.recent.get(&item.id).map(|(token, _)| *token));
        if let Some(token) = duplicate {
            self.recent.insert(item.id.clone(), (token, now));
            return Ok(LaunchStart::Duplicate(token));
        }
        if self.pending.len() >= 16 {
            return Err(
                "Plusieurs applications sont en cours de lancement. Réessaie dans un instant.",
            );
        }
        self.next_request = self
            .next_request
            .checked_add(1)
            .ok_or("Trop de demandes de lancement.")?;
        self.pending.insert(self.next_request, item.clone());
        self.recent
            .insert(item.id.clone(), (self.next_request, now));
        Ok(LaunchStart::Started(self.next_request))
    }

    fn finish(&mut self, token: usize, success: bool) -> Option<Item> {
        let item = self.pending.remove(&token)?;
        if !success
            && self
                .recent
                .get(&item.id)
                .is_some_and(|(current, _)| *current == token)
        {
            self.recent.remove(&item.id);
        }
        Some(item)
    }
}

#[derive(Default)]
struct Installations {
    pending: HashMap<usize, String>,
    completed: HashSet<String>,
    next_request: usize,
}

impl Installations {
    fn is_pending(&self, id: &str) -> bool {
        self.pending.values().any(|pending| pending == id)
    }

    fn start(&mut self, id: &str) -> Option<usize> {
        if self.completed.contains(id) || self.is_pending(id) {
            return None;
        }
        self.next_request = self.next_request.checked_add(1)?;
        self.pending.insert(self.next_request, id.into());
        Some(self.next_request)
    }

    fn finish(&mut self, request: usize, success: bool) -> Option<String> {
        let id = self.pending.remove(&request)?;
        if success {
            self.completed.insert(id.clone());
        }
        Some(id)
    }

    fn refresh_missing(&mut self, missing: &mut Vec<Item>) {
        // Keep the success marker only until the local index observes the app.
        // Once observed, a later uninstall can offer installation again.
        self.completed
            .retain(|id| missing.iter().any(|item| &item.target == id));
        missing.retain(|item| !self.completed.contains(&item.target));
    }

    fn refresh_missing_with_absent(
        &mut self,
        missing: &mut Vec<Item>,
        absent: &HashSet<String>,
        observed_generation: usize,
        required_generation: usize,
    ) {
        // A cached absence from before the installation cannot override success.
        if observed_generation >= required_generation {
            self.completed.retain(|id| !absent.contains(id));
        }
        self.refresh_missing(missing);
    }
}

pub fn show_error(error: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(&i18n::tr(error)).as_ptr(),
            wide("Super Menu").as_ptr(),
            MB_ICONERROR | MB_OK,
        );
    }
}

fn reconcile_removals(
    completed: &mut HashSet<String>,
    refresh_after: &mut HashMap<String, usize>,
    observed_generation: usize,
    available: &[Item],
) {
    completed.retain(|target| {
        let fresh = refresh_after
            .get(target)
            .is_some_and(|minimum| observed_generation >= *minimum);
        !fresh || available.iter().any(|item| &item.target == target)
    });
    refresh_after.retain(|target, _| completed.contains(target));
}

fn clear_removal_for_install(
    removals: &mut Installations,
    refresh_after: &mut HashMap<String, usize>,
    id: &str,
) {
    let target = format!("id:{id}");
    removals.completed.remove(&target);
    refresh_after.remove(&target);
}

fn parse_smoke_fixture(bytes: &[u8]) -> Result<Vec<Item>, String> {
    if bytes.len() > 1024 * 1024 {
        return Err("Fixture de test supérieure à 1 Mio.".into());
    }
    let mut items: Vec<Item> =
        serde_json::from_slice(bytes).map_err(|error| format!("Fixture invalide : {error}"))?;
    if items.len() > 256 {
        return Err("Fixture limitée à 256 éléments.".into());
    }
    let mut ids = HashSet::new();
    for item in &mut items {
        if !matches!(item.kind, Kind::Application | Kind::File | Kind::Folder)
            || item.id.is_empty()
            || !ids.insert(item.id.clone())
            || !item.arguments.is_empty()
            || [
                &item.id,
                &item.title,
                &item.subtitle,
                &item.glyph,
                &item.target,
            ]
            .iter()
            .any(|text| text.len() > 4096 || text.contains('\0'))
        {
            return Err(
                "Fixture : types indexables, identifiants uniques et textes bornés requis.".into(),
            );
        }
        item.prepare();
    }
    Ok(items)
}

fn load_smoke_fixture(args: &[String]) -> Result<Option<Vec<Item>>, String> {
    let Some(position) = args
        .iter()
        .position(|argument| argument == "--smoke-fixture")
    else {
        return Ok(None);
    };
    if !args.iter().any(|argument| argument == "--smoke-test") {
        return Err("--smoke-fixture nécessite --smoke-test.".into());
    }
    let path = args
        .get(position + 1)
        .filter(|path| !path.starts_with("--"))
        .ok_or("Chemin de --smoke-fixture manquant.")?;
    let file =
        std::fs::File::open(path).map_err(|error| format!("Fixture inaccessible : {error}"))?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(file, 1024 * 1024 + 1), &mut bytes)
        .map_err(|error| format!("Lecture de la fixture : {error}"))?;
    parse_smoke_fixture(&bytes).map(Some)
}

fn fixture_action_allowed(item: &Item) -> bool {
    matches!(item.kind, Kind::Navigation | Kind::Timer)
        || (item.kind == Kind::System && item.target == "about")
}

/// Launcher snapshot written to the `--report` path during smoke tests.
/// Fields follow the alphabetical key order of the former `json!` document.
#[derive(serde::Serialize)]
struct SmokeReport<'a> {
    about_collecting: bool,
    about_created: bool,
    about_hwnd: usize,
    about_loaded: bool,
    application_icons_loaded: usize,
    applications: usize,
    checks: &'a [(String, bool)],
    edit_created: bool,
    files: usize,
    first_frame_ms: f64,
    fixture_activation_attempts: usize,
    fixture_mode: bool,
    index_items: usize,
    indexing_finished: bool,
    items: &'a [Item],
    language: String,
    native_window_created: bool,
    pending_file_refresh: bool,
    private_bytes: usize,
    query: &'a str,
    section: &'a str,
    selected: usize,
    timer_alert_hwnd: usize,
    timer_alert_visible: bool,
    timer_count: usize,
    visible_results: usize,
    working_set_bytes: usize,
}

unsafe fn write_smoke_report(state: &State) -> Result<(), String> {
    let Some(path) = &state.smoke_report else {
        return Ok(());
    };
    let index = state
        .index
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut memory: PROCESS_MEMORY_COUNTERS_EX = zeroed();
    memory.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    K32GetProcessMemoryInfo(
        GetCurrentProcess(),
        (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
        memory.cb,
    );
    let report = SmokeReport {
        about_collecting: state
            .about
            .as_ref()
            .is_some_and(about::AboutWindow::is_collecting),
        about_created: state.about.is_some(),
        about_hwnd: state
            .about
            .as_ref()
            .map_or(0, |window| window.hwnd() as usize),
        about_loaded: state
            .about
            .as_ref()
            .is_some_and(about::AboutWindow::is_loaded),
        application_icons_loaded: state
            .app_icons
            .as_ref()
            .map_or(0, app_icons::IconCache::loaded_count),
        applications: index
            .iter()
            .filter(|item| item.kind == Kind::Application)
            .count(),
        checks: &state.checks,
        edit_created: !state.edit.is_null(),
        files: index
            .iter()
            .filter(|item| matches!(item.kind, Kind::File | Kind::Folder))
            .count(),
        first_frame_ms: state.first_frame_ms,
        fixture_activation_attempts: state.fixture_activation_attempts,
        fixture_mode: state.smoke_fixture,
        index_items: index.len(),
        indexing_finished: !state.indexing.load(Ordering::Relaxed),
        items: &state.items,
        language: i18n::current_language(),
        native_window_created: true,
        pending_file_refresh: state.pending_file_refresh,
        private_bytes: memory.PrivateUsage,
        query: &state.query,
        section: &state.section,
        selected: state.selected,
        timer_alert_hwnd: state.timers.borrow().alert_hwnd() as usize,
        timer_alert_visible: state.timers.borrow().alert_visible(),
        timer_count: state.timers.borrow().book.snapshots(Instant::now()).len(),
        visible_results: state.items.len(),
        working_set_bytes: memory.WorkingSetSize,
    };
    let json = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    std::fs::write(path, json).map_err(|error| format!("Rapport de test : {error}"))
}

pub fn run(args: &[String]) -> Result<(), String> {
    let started = Instant::now();
    if !cfg!(target_os = "windows") {
        return Err("Super Menu fonctionne uniquement sous Windows.".into());
    }
    let fixture = load_smoke_fixture(args)?;
    let fixture_mode = fixture.is_some();
    let smoke_first_language_choice = args
        .iter()
        .any(|argument| argument == "--smoke-first-run-language");
    if smoke_first_language_choice && !fixture_mode {
        return Err("Le choix de langue de test nécessite une fixture.".into());
    }
    for flag in ["--smoke-language", "--smoke-language-dir"] {
        if let Some(position) = args.iter().position(|argument| argument == flag) {
            if !fixture_mode
                || args
                    .iter()
                    .filter(|argument| argument.as_str() == flag)
                    .count()
                    != 1
                || args
                    .get(position + 1)
                    .is_none_or(|value| value.trim().is_empty() || value.starts_with("--"))
            {
                return Err("Les options de traduction de test nécessitent une fixture et une valeur unique.".into());
            }
        }
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
        let icon = brand_icon::Icon::new(32);
        let small_icon = brand_icon::Icon::new(tray_icon_size());
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class_name.as_ptr(),
            hIcon: icon.raw(),
            hIconSm: small_icon.raw(),
            style: CS_HREDRAW | CS_VREDRAW,
            ..zeroed()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(i18n::fmt(
                "Création de la fenêtre impossible ({error})",
                &[("error", GetLastError().to_string())],
            ));
        }
        let mut settings = if fixture_mode {
            let mut settings = config::Settings::default();
            settings.file_roots.clear();
            settings
        } else {
            config::load()
        };
        if smoke {
            if let Some(theme) = args
                .iter()
                .position(|argument| argument == "--smoke-theme")
                .and_then(|position| args.get(position + 1))
                .filter(|theme| matches!(theme.as_str(), "dark" | "light"))
            {
                settings.theme = theme.clone();
            }
        }
        let language_cache = if fixture_mode {
            args.iter()
                .position(|argument| argument == "--smoke-language-dir")
                .and_then(|position| args.get(position + 1))
                .map(std::path::PathBuf::from)
        } else {
            Some(config::data_dir().join("languages"))
        };
        if fixture_mode
            && language_cache
                .as_ref()
                .is_some_and(|path| !path.is_absolute())
        {
            return Err("Le cache de traduction de test doit être absolu.".into());
        }
        if smoke_first_language_choice && language_cache.is_none() {
            return Err("Le premier choix de test nécessite un cache isolé.".into());
        }
        let needs_language_choice = if smoke {
            settings.language = args
                .iter()
                .position(|argument| argument == "--smoke-language")
                .and_then(|position| args.get(position + 1))
                .cloned()
                .unwrap_or_else(|| "fr".into());
            if !i18n::is_supported(&settings.language)
                || (settings.language != "fr" && !fixture_mode)
            {
                return Err("La langue de test nécessite une fixture et un code valide.".into());
            }
            if settings.language != "fr"
                && language_cache
                    .as_ref()
                    .is_none_or(|path| !path.is_absolute())
            {
                return Err("La traduction de test nécessite un cache absolu isolé.".into());
            }
            i18n::init_cached(
                &settings.language,
                language_cache
                    .as_deref()
                    .unwrap_or(std::path::Path::new("")),
            )?;
            smoke_first_language_choice
        } else {
            settings.language.is_empty()
                || i18n::init_cached(&settings.language, language_cache.as_deref().unwrap())
                    .is_err()
        };
        let cache = fixture.unwrap_or_default();
        let usage = if fixture_mode {
            catalog::Usage {
                counts: HashMap::new(),
            }
        } else {
            catalog::Usage::new()
        };
        let weights = usage.snapshot_for(&cache);
        let (launch_sender, launch_results) = mpsc::channel();
        let timers = Rc::new(RefCell::new(timer_runtime::TimerRuntime::new(
            settings.clone(),
            smoke,
        )));
        let state_cell = Box::new(RefCell::new(State {
            hwnd: null_mut(),
            edit: null_mut(),
            palette: Palette::from(&settings),
            cached_ui: CachedUi::default(),
            paint_text: RefCell::new(Vec::with_capacity(256)),
            paint_buffer: PaintBuffer::default(),
            query_text: Vec::with_capacity(1025),
            steam_enabled: Arc::new(AtomicBool::new(settings.steam_enabled)),
            settings,
            index: Arc::new(Mutex::new(cache)),
            installed_names: Arc::new(Mutex::new(Vec::new())),
            removable_names: Arc::new(Mutex::new(Vec::new())),
            confirmed_absent_packages: Arc::new(Mutex::new(HashSet::new())),
            missing_packages: Vec::new(),
            removable_packages: Vec::new(),
            installations: Installations::default(),
            removals: Installations::default(),
            removal_refresh_after: HashMap::new(),
            index_generation: 0,
            catalog_ready: Arc::new(AtomicUsize::new(0)),
            feedback: None,
            feedback_request: None,
            feedback_artwork: None,
            timers,
            timer_focus: None,
            launches: LaunchRequests::default(),
            launch_results,
            launch_sender,
            indexing: Arc::new(AtomicBool::new(false)),
            usage,
            weights,
            clipboard: catalog::ClipHistory::new(),
            items: vec![],
            item_icon_keys: Vec::new(),
            item_icon_size: 0,
            selected: 0,
            query: String::new(),
            section: i18n::tr("EXPLORER"),
            timing: String::new(),
            status: i18n::tr("Index local"),
            fonts: [null_mut(); 6],
            edit_bg: null_mut(),
            icon: small_icon.raw(),
            app_icons: None,
            about: None,
            hotkey: None,
            watcher: None,
            dpi: 96,
            visible_rows: 9,
            previous: null_mut(),
            settings_open: false,
            smoke,
            smoke_fixture: fixture_mode,
            smoke_report: args
                .iter()
                .position(|argument| argument == "--report")
                .and_then(|position| args.get(position + 1))
                .cloned(),
            language_cache,
            fixture_activation_attempts: 0,
            feedback_test: smoke && args.iter().any(|arg| arg == "--launch-feedback-test"),
            feedback_test_finished: false,
            pending_refresh: false,
            pending_file_refresh: false,
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
        state.app_icons = app_icons::IconCache::new(hwnd, ICONS_READY).ok();
        state.feedback = launch_feedback::LaunchFeedback::new().ok();
        timer_runtime::bind(&state.timers, hwnd);
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
            wide(&i18n::tr("Où veux-tu aller ?")).as_ptr() as isize,
        );
        rebuild_fonts(&mut state);
        place(&mut state);
        if needs_language_choice {
            if let Some(language) = language_dialog::choose(
                hwnd,
                state.fonts[1],
                state.dpi,
                &state.settings.language,
                state.language_cache.as_ref().unwrap().clone(),
            ) {
                let mut candidate = state.settings.clone();
                candidate.language = language.clone();
                let saved = if state.smoke_fixture {
                    Ok(())
                } else {
                    config::save(&candidate)
                };
                match saved {
                    Ok(()) => {
                        i18n::load_cached(&language, state.language_cache.as_ref().unwrap())?;
                        state.settings = candidate;
                        state.timers.borrow_mut().settings = state.settings.clone();
                    }
                    Err(error) => show_error(&error.to_string()),
                }
            }
        }
        refresh_language(&mut state);
        update_results(&mut state);
        if !smoke {
            match native::install_hotkey(hwnd, &state.settings.hotkey) {
                Ok(h) => state.hotkey = Some(h),
                Err(e) => {
                    state.status = i18n::tr("Raccourci indisponible · icône de notification");
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
            let smoke_timeout = args
                .iter()
                .position(|arg| arg == "--smoke-timeout-ms")
                .and_then(|position| args.get(position + 1))
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|value| (8500..=300_000).contains(value))
                .unwrap_or(8500);
            SetTimer(hwnd, 99, smoke_timeout, None);
            if state.feedback_test {
                SetTimer(hwnd, 98, 300, None);
            }
        }
        drop(state);
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            let handled = state_cell.try_borrow().is_ok_and(|state| {
                state
                    .about
                    .as_ref()
                    .is_some_and(|window| window.handle_message(&msg))
            });
            if !handled {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // Windows menus and ShellExecute can run a nested message loop while
            // State is borrowed. Replay their notifications once that borrow ends.
            replay_deferred(hwnd);
        }
        let mut state = state_cell.borrow_mut();
        let report_result = if smoke {
            write_smoke_report(&state)
        } else {
            Ok(())
        };
        RemoveClipboardFormatListener(hwnd);
        state.hotkey.take();
        state.watcher.take();
        state.feedback.take();
        state.app_icons.take();
        state.about.take();
        state.timers.borrow_mut().shutdown();
        timer_runtime::unbind();
        KillTimer(hwnd, TIMER_TICK);
        tray(&state, NIM_DELETE);
        DestroyWindow(hwnd);
        for font in state.fonts {
            if !font.is_null() {
                DeleteObject(font);
            }
        }
        DeleteObject(state.edit_bg);
        // The class and Shell no longer reference either privately owned icon.
        UnregisterClassW(class_name.as_ptr(), instance);
        report_result
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
    if msg == SMOKE_CTRL_BACKSPACE {
        // Keyboard state belongs to this UI thread's input queue. Exercise the
        // actual edit subclass without injecting input into another application.
        let edit = match cell.try_borrow() {
            Ok(state) if state.smoke_fixture => state.edit,
            _ => return 0,
        };
        let mut saved = [0u8; 256];
        if GetKeyboardState(saved.as_mut_ptr()) == 0 {
            return 0;
        }
        let mut pressed = saved;
        pressed[VK_CONTROL as usize] |= 0x80;
        if SetKeyboardState(pressed.as_ptr()) == 0 {
            return 0;
        }
        SendMessageW(edit, WM_KEYDOWN, VK_BACK as usize, 0);
        return SetKeyboardState(saved.as_ptr()) as isize;
    }
    if timer_runtime::handle_message(hwnd, msg, wp) {
        if let Ok(mut s) = cell.try_borrow_mut() {
            let available = s.timers.try_borrow().is_ok();
            if available {
                refresh_timer_view(&mut s);
            }
        } else {
            DEFERRED.with(|d| d.set(d.get() | 8));
        }
        return if msg == WM_POWERBROADCAST { 1 } else { 0 };
    }
    let Ok(mut borrowed) = cell.try_borrow_mut() else {
        if msg == INDEX || msg == REFRESH {
            let flag = if msg == INDEX {
                1
            } else if wp == native::WATCH_FILES {
                64
            } else {
                2
            };
            DEFERRED.with(|d| d.set(d.get() | flag));
        } else if msg == installer::INSTALL_DONE {
            DEFERRED_INSTALL.with(|d| d.borrow_mut().push((wp, lp)));
        } else if msg == installer::REMOVE_DONE {
            DEFERRED_REMOVE.with(|d| d.borrow_mut().push((wp, lp)));
        } else if msg == LAUNCH_DONE {
            DEFERRED.with(|d| d.set(d.get() | 4));
        } else if msg == ICONS_READY {
            DEFERRED.with(|d| d.set(d.get() | 16));
        } else if msg == ABOUT {
            DEFERRED.with(|d| d.set(d.get() | 32));
        } else if msg == WM_TIMER && wp == TIMER_TICK {
            DEFERRED.with(|d| d.set(d.get() | 8));
        } else if msg == timer_alert::WM_TIMER_ALERT_DISMISSED {
            // The alert can be dismissed during a nested Windows modal loop.
            DEFERRED_TIMER_ACK.with(|d| d.borrow_mut().push(wp));
        }
        return DefWindowProcW(hwnd, msg, wp, lp);
    };
    let s = &mut *borrowed;
    match msg {
        SMOKE_STATE if s.smoke_fixture => isize::from(write_smoke_report(s).is_ok()),
        SMOKE_LANGUAGE_PICKER if s.smoke_fixture => {
            if let Some(cache) = s.language_cache.clone() {
                if let Some(language) = language_dialog::choose(
                    s.hwnd,
                    s.fonts[1],
                    s.dpi,
                    &s.settings.language,
                    cache.clone(),
                ) {
                    if i18n::load_cached(&language, &cache).is_ok() {
                        s.settings.language = language;
                        s.about.take();
                        refresh_language(s);
                        reindex(s);
                        update_results(s);
                    }
                }
            }
            0
        }
        ABOUT => {
            show_about(s);
            0
        }
        ICONS_READY => {
            let changed = s.app_icons.as_mut().is_some_and(|icons| icons.drain());
            request_feedback_icon(s);
            request_result_icons(s);
            if changed {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            paint_frame(s, dc);
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
            request_result_icons(s);
            0
        }
        WM_SIZE => {
            layout_edit(s);
            request_result_icons(s);
            0
        }
        WM_CLIPBOARDUPDATE => {
            if s.settings.clipboard_enabled
                && s.clipboard.capture(s.settings.max_clipboard_entries)
                && s.query.starts_with(':')
            {
                refresh_results(s);
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
            let installed_names = s.installed_names.lock().unwrap_or_else(|e| e.into_inner());
            let visible_apps: Vec<Item> = index
                .iter()
                .filter(|item| {
                    item.kind == Kind::Application
                        && !s
                            .removals
                            .completed
                            .iter()
                            .any(|target| packages::application_matches_removal(item, target))
                })
                .cloned()
                .collect();
            let visible_names: Vec<String> = installed_names
                .iter()
                .filter(|name| {
                    let entry = Item::new("", name.as_str(), "", "", Kind::Application, "");
                    !s.removals
                        .completed
                        .iter()
                        .any(|target| packages::application_matches_removal(&entry, target))
                })
                .cloned()
                .collect();
            let confirmed_absent = s
                .confirmed_absent_packages
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            s.missing_packages = packages::missing_items_with_absent(
                &visible_apps,
                &visible_names,
                &HashSet::new(),
                &confirmed_absent,
            );
            s.installations.refresh_missing_with_absent(
                &mut s.missing_packages,
                &confirmed_absent,
                s.catalog_ready.load(Ordering::Acquire),
                s.index_generation,
            );
            drop(confirmed_absent);
            let removable_names = s.removable_names.lock().unwrap_or_else(|e| e.into_inner());
            let available = packages::removable_items_with_inventory(
                &index,
                &installed_names,
                &removable_names,
            );
            drop(installed_names);
            reconcile_removals(
                &mut s.removals.completed,
                &mut s.removal_refresh_after,
                s.catalog_ready.load(Ordering::Acquire),
                &available,
            );
            s.removable_packages = available
                .into_iter()
                .filter(|item| !s.removals.completed.contains(&item.target))
                .collect();
            drop(removable_names);
            s.status = if s.indexing.load(Ordering::Relaxed) {
                i18n::fmt("{apps} applis · indexation…", &[("apps", apps.to_string())])
            } else {
                i18n::fmt(
                    "{apps} applis · {entries} entrées",
                    &[
                        ("apps", apps.to_string()),
                        ("entries", index.len().to_string()),
                    ],
                )
            };
            drop(index);
            refresh_results(s);
            if !s.indexing.load(Ordering::Relaxed) && s.pending_refresh {
                s.pending_refresh = false;
                reindex(s);
            }
            0
        }
        REFRESH => {
            if defer_file_refresh(wp, IsWindowVisible(hwnd) != 0) {
                s.pending_file_refresh = true;
            } else {
                reindex(s);
            }
            0
        }
        installer::INSTALL_DONE => {
            let success = lp == 0;
            if let Some(id) = s.installations.finish(wp, success) {
                if let Some(package) = packages::find(&id) {
                    if success {
                        if let Some(icons) = s.app_icons.as_mut() {
                            icons.clear();
                        }
                        s.missing_packages.retain(|item| item.target != package.id);
                        clear_removal_for_install(
                            &mut s.removals,
                            &mut s.removal_refresh_after,
                            package.id,
                        );
                        reindex(s);
                    } else {
                        s.status = i18n::fmt(
                            "Installation de {name} interrompue · détails dans les rapports",
                            &[("name", package.name.into())],
                        );
                    }
                    refresh_results(s);
                }
            }
            0
        }
        installer::REMOVE_DONE => {
            let success = lp == 0;
            if let Some(target) = s.removals.finish(wp, success) {
                if success {
                    if let Some(icons) = s.app_icons.as_mut() {
                        icons.clear();
                    }
                    s.removal_refresh_after
                        .insert(target.clone(), s.index_generation.saturating_add(1));
                    s.removable_packages.retain(|item| item.target != target);
                    if let Some(id) = target.strip_prefix("id:") {
                        s.installations.completed.remove(id);
                    }
                    reindex(s);
                } else {
                    s.status = i18n::tr("Désinstallation interrompue · détails dans les rapports");
                    reindex(s);
                }
                refresh_results(s);
            }
            0
        }
        LAUNCH_DONE => {
            process_launch_results(s);
            0
        }
        timer_alert::WM_TIMER_ALERT_DISMISSED => {
            acknowledge_timer_alert(s, wp);
            0
        }
        TRAY => {
            if lp as u32 == WM_LBUTTONUP || lp as u32 == WM_LBUTTONDBLCLK {
                show(s);
            } else if lp as u32 == WM_RBUTTONUP {
                let menu = CreatePopupMenu();
                AppendMenuW(
                    menu,
                    MF_STRING,
                    1,
                    wide(&i18n::tr("Ouvrir Super Menu")).as_ptr(),
                );
                AppendMenuW(menu, MF_STRING, 2, wide(&i18n::tr("Paramètres")).as_ptr());
                AppendMenuW(
                    menu,
                    MF_STRING,
                    3,
                    wide(&i18n::tr("Actualiser l'index")).as_ptr(),
                );
                AppendMenuW(menu, MF_STRING, 5, wide(&i18n::tr("Minuteurs")).as_ptr());
                AppendMenuW(
                    menu,
                    MF_STRING,
                    7,
                    wide(&i18n::tr("À propos de Super Menu")).as_ptr(),
                );
                let reports = config::data_dir().join("uninstall-reports");
                AppendMenuW(
                    menu,
                    MF_STRING | if reports.is_dir() { 0 } else { MF_GRAYED },
                    6,
                    wide(&i18n::tr("Rapports de désinstallation")).as_ptr(),
                );
                let operations = installer::operation_reports_dir();
                AppendMenuW(
                    menu,
                    MF_STRING | if operations.is_dir() { 0 } else { MF_GRAYED },
                    8,
                    wide(&i18n::tr("Dernières opérations de logiciels")).as_ptr(),
                );
                AppendMenuW(menu, MF_SEPARATOR, 0, null());
                AppendMenuW(menu, MF_STRING, 4, wide(&i18n::tr("Quitter")).as_ptr());
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
                    3 => refresh_index_and_icons(s),
                    7 => show_about(s),
                    5 => {
                        show(s);
                        set_query(s, "minuteur ");
                    }
                    6 => {
                        if let Err(error) = native::launch(&reports.to_string_lossy(), "") {
                            show_error(&error);
                        }
                    }
                    8 => {
                        if let Err(error) = native::launch(&operations.to_string_lossy(), "") {
                            show_error(&error);
                        }
                    }
                    4 => {
                        PostQuitMessage(0);
                    }
                    _ => {}
                }
            }
            0
        }
        WM_TIMER => {
            if wp == TIMER_TICK {
                tick_timers(s);
            } else if wp == 98 && s.feedback_test {
                KillTimer(hwnd, 98);
                let item = Item::new(
                    "feedback-test",
                    "Super Menu · application de test",
                    "",
                    "",
                    Kind::Application,
                    "",
                );
                start_application(s, &item);
            } else if wp == 99 {
                smoke_checks(s);
                capture_bmp(s);
                PostQuitMessage(0);
            } else if !s.edit.is_null() && IsWindowVisible(hwnd) != 0 && s.query.starts_with('~') {
                refresh_results(s);
            }
            0
        }
        WM_POWERBROADCAST => {
            // Windows' monotonic clock includes sleep; check overdue timers on
            // resume without waking the computer or keeping it awake.
            if wp == 0x12 || wp == 0x07 {
                tick_timers(s);
            }
            1
        }
        WM_CLOSE => {
            hide(s);
            0
        }
        WM_DESTROY => {
            s.paint_buffer.reset();
            s.app_icons.take();
            s.about.take();
            KillTimer(hwnd, 1);
            KillTimer(hwnd, 99);
            KillTimer(hwnd, TIMER_TICK);
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
    // Do not borrow State here: EM_REPLACESEL synchronously sends EN_CHANGE.
    if msg == WM_KEYDOWN && wp == VK_BACK as usize && GetKeyState(VK_CONTROL as i32) < 0 {
        delete_previous_word(hwnd);
        return 0;
    }
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
        && (wp == VK_RETURN as usize
            || wp == VK_ESCAPE as usize
            || wp == VK_TAB as usize
            || wp == 0x7f)
    {
        return 0;
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

unsafe fn delete_previous_word(hwnd: HWND) {
    let mut start = 0u32;
    let mut end = 0u32;
    SendMessageW(
        hwnd,
        EM_GETSEL,
        (&mut start as *mut u32) as usize,
        (&mut end as *mut u32) as isize,
    );
    if start == end {
        if start == 0 {
            return;
        }
        // Reuse the edit control's Ctrl+Left word boundaries and UTF-16 offsets.
        DefSubclassProc(hwnd, WM_KEYDOWN, VK_LEFT as usize, 0);
        SendMessageW(hwnd, EM_GETSEL, (&mut start as *mut u32) as usize, 0);
    }
    if start < end {
        SendMessageW(hwnd, EM_SETSEL, start as usize, end as isize);
        SendMessageW(hwnd, EM_REPLACESEL, 1, [0u16].as_ptr() as isize);
    }
}

unsafe fn key(s: &mut State, key: u32) {
    match key as u16 {
        VK_UP => select(s, -1),
        VK_DOWN => select(s, 1),
        VK_RETURN => execute(s, GetKeyState(VK_CONTROL as i32) < 0),
        VK_ESCAPE => {
            if s.timer_focus.take().is_some() {
                update_results(s);
            } else if s.query.is_empty() {
                hide(s);
            } else {
                set_query(s, "");
            }
        }
        VK_TAB => set_query(s, ";"),
        VK_F5 => refresh_index_and_icons(s),
        VK_F1 => {
            s.settings_open = true;
            MessageBoxW(s.hwnd,wide(&i18n::tr("Win + Espace : ouvrir / masquer\n↑ ↓ : sélectionner · Entrée : ouvrir / installer / désinstaller\nCtrl + Entrée : dossier parent d'un fichier\nÉchap : effacer puis masquer · F5 : réindexer\nCtrl + , : paramètres · Tab / ; : modules\n\nNom seul : applications, installation ou désinstallation\n@ applications à lancer    + installer    - désinstaller\n/ fichiers    = calcul    ? recherche web\n> commandes    : presse-papiers    . symboles\n~ fenêtres    ! système\nminuteur 5m Thé · timer 90s : compte à rebours\n\nInstaller : Entrée ouvre la progression WinGet.\nDésinstaller : une confirmation, puis désinstallation et nettoyage automatique.\nLa recherche et l'index restent locaux.\nLe presse-papiers est désactivé par défaut.")).as_ptr(),wide(&i18n::tr("Super Menu · Aide")).as_ptr(),MB_OK);
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
    request_result_icons(s);
    InvalidateRect(s.hwnd, null(), 0);
}
unsafe fn set_query(s: &mut State, text: &str) {
    SetWindowTextW(s.edit, wide(text).as_ptr());
    SetFocus(s.edit);
    let end = GetWindowTextLengthW(s.edit) as usize;
    SendMessageW(s.edit, EM_SETSEL, end, end as isize);
    SendMessageW(s.edit, EM_SCROLLCARET, 0, 0);
    update_results(s);
}

fn navigation() -> Vec<Item> {
    [
        (
            "Applications",
            "Rechercher et lancer une application installée",
            "▦",
            "@ ",
        ),
        (
            "Installer",
            "Ajouter un logiciel · catalogue WinGet",
            "↓",
            "+ ",
        ),
        (
            "Désinstaller",
            "Retirer un logiciel et ses résidus · données personnelles conservées",
            "⊠",
            "- ",
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
        (
            "Minuteur",
            "Compte à rebours · essaie minuteur 5m Thé",
            "\u{E916}",
            "minuteur ",
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
        (
            "À propos",
            "Super Menu, matériel et informations Windows",
            "\u{E946}",
            "about",
        ),
    ]
    .into_iter()
    .map(|(title, sub, glyph, target)| {
        Item::new(
            format!("nav:{target}"),
            i18n::tr(title),
            i18n::tr(sub),
            glyph,
            Kind::Navigation,
            target,
        )
    })
    .collect()
}

fn navigation_shortcut<'a>(item: &'a Item, cached: &'a CachedUi) -> Option<(&'a str, i32)> {
    if item.kind != Kind::Navigation {
        return None;
    }
    match item.target.trim() {
        prefix @ ("@" | "+" | "-" | "/" | "=" | "?" | ">" | ":" | "~" | "." | "!") => {
            Some((prefix, 28))
        }
        "minuteur" => Some((&cached.timer_shortcut, 76)),
        "about" => Some((&cached.about_shortcut, 96)),
        _ => None,
    }
}

unsafe fn update_results(s: &mut State) {
    rebuild_results(s, false);
}

// Background updates keep the selected item, including its derived scroll position.
unsafe fn refresh_results(s: &mut State) {
    rebuild_results(s, true);
}

fn selection_after_refresh(items: &[Item], previous: usize, selected_id: Option<&str>) -> usize {
    selected_id
        .and_then(|id| items.iter().position(|item| item.id == id))
        .unwrap_or_else(|| previous.min(items.len().saturating_sub(1)))
}

unsafe fn rebuild_results(s: &mut State, preserve_selection: bool) {
    if s.edit.is_null() {
        return;
    }
    let len = GetWindowTextLengthW(s.edit).max(0) as usize;
    s.query_text.resize(len + 1, 0);
    let read = GetWindowTextW(s.edit, s.query_text.as_mut_ptr(), s.query_text.len() as i32).max(0)
        as usize;
    let query_changed = !s
        .query
        .encode_utf16()
        .eq(s.query_text[..read].iter().copied());
    let preserve_selection = preserve_selection && !query_changed;
    let previous_selection = if preserve_selection { s.selected } else { 0 };
    // Only background refreshes clone one result ID; typing keeps its existing fast path.
    let selected_id = preserve_selection
        .then(|| s.items.get(s.selected).map(|item| item.id.clone()))
        .flatten();
    if query_changed {
        s.timer_focus = None;
        s.query = String::from_utf16_lossy(&s.query_text[..read]);
    }
    let start = Instant::now();
    let trimmed = s.query.trim();
    let prefix = trimmed.chars().next().unwrap_or('\0');
    let body = if "@/+-=?>:.~!;".contains(prefix) && prefix != '\0' {
        trimmed.get(prefix.len_utf8()..).unwrap_or("").trim()
    } else {
        trimmed
    };
    let limit = s.settings.max_results.max(1);
    let weights = &s.weights;
    s.items = if let Some(body) = timer_query(trimmed) {
        s.section = i18n::tr("MINUTEURS");
        timer_items(&s.timers.borrow().book, body, s.timer_focus, start)
    } else {
        match prefix {
            '\0' | ';' => {
                s.section = i18n::tr("EXPLORER");
                if body.is_empty() {
                    s.cached_ui.navigation.clone()
                } else {
                    core::rank(&s.cached_ui.navigation, body, limit, weights)
                }
            }
            '=' => {
                s.section = i18n::tr("CALCULATRICE");
                match core::calculate(body) {
                    Some(result) => vec![Item::new(
                        "calc",
                        result.clone(),
                        i18n::fmt("{body} · Entrée pour copier", &[("body", body.to_string())]),
                        "=",
                        Kind::Calculator,
                        result,
                    )],
                    None if body.is_empty() => vec![Item::new(
                        "example",
                        i18n::tr("Essaie = (12 + 8) × 3"),
                        i18n::tr("Opérateurs + - * / ^ %, fonctions sqrt, sin, cos"),
                        "=",
                        Kind::Navigation,
                        "= (12 + 8) * 3",
                    )],
                    None => vec![],
                }
            }
            '?' => {
                s.section = i18n::tr("RECHERCHE WEB");
                if body.is_empty() {
                    vec![]
                } else {
                    let url = s
                        .settings
                        .web_search_template
                        .replace("{query}", &encode(body));
                    vec![Item::new(
                        "web",
                        i18n::fmt("Rechercher « {body} »", &[("body", body.to_string())]),
                        i18n::tr("Ouvrir dans le navigateur par défaut"),
                        "⌕",
                        Kind::Web,
                        url,
                    )]
                }
            }
            '>' => {
                s.section = i18n::tr("COMMANDES");
                let mut found = core::rank(&s.cached_ui.commands, body, limit, weights);
                if !body.is_empty() && found.len() < limit {
                    let mut cmd = Item::new(
                        "shell",
                        i18n::fmt("Exécuter : {body}", &[("body", body.to_string())]),
                        i18n::tr("Commande Windows · exécutée uniquement avec Entrée"),
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
                s.section = i18n::tr("PRESSE-PAPIERS");
                if !s.settings.clipboard_enabled {
                    vec![Item::new(
                        "clipboard-settings",
                        i18n::tr("Activer l'historique du presse-papiers"),
                        i18n::tr("Désactivé par défaut · conservé uniquement en mémoire"),
                        "▤",
                        Kind::System,
                        "settings",
                    )]
                } else {
                    core::rank(&s.clipboard.items(), body, limit, weights)
                }
            }
            '.' => {
                s.section = i18n::tr("SYMBOLES");
                core::rank(&s.cached_ui.symbols, body, limit, weights)
            }
            '~' => {
                s.section = i18n::tr("FENÊTRES OUVERTES");
                core::rank(&native::windows(), body, limit, weights)
            }
            '!' => {
                s.section = i18n::tr("SYSTÈME");
                core::rank(
                    &s.cached_ui.system,
                    body,
                    if body.is_empty() { 20 } else { limit },
                    weights,
                )
            }
            _ => {
                s.section = i18n::tr(if prefix == '/' {
                    "FICHIERS"
                } else {
                    "APPLICATIONS"
                });
                let index = s.index.lock().unwrap_or_else(|e| e.into_inner());
                if prefix == '/' {
                    search_catalog(&index, trimmed, limit, weights)
                } else {
                    if prefix == '+' {
                        s.section = i18n::tr("INSTALLER AVEC WINGET");
                    } else if prefix == '-' {
                        s.section = i18n::tr("DÉSINSTALLER UN LOGICIEL");
                    }
                    let visible_apps: Vec<Item> = if s.removals.completed.is_empty() {
                        Vec::new()
                    } else {
                        index
                            .iter()
                            .filter(|item| {
                                item.kind == Kind::Application
                                    && !s.removals.completed.iter().any(|target| {
                                        packages::application_matches_removal(item, target)
                                    })
                            })
                            .cloned()
                            .collect()
                    };
                    let search_index = if s.removals.completed.is_empty() {
                        index.as_slice()
                    } else {
                        visible_apps.as_slice()
                    };
                    packages::search_with_removals(
                        search_index,
                        &s.missing_packages,
                        &s.removable_packages,
                        trimmed,
                        limit,
                        weights,
                    )
                }
            }
        }
    };
    for item in &mut s.items {
        if item.kind == Kind::Navigation && item.target == "minuteur " {
            let timers = s.timers.borrow().book.snapshots(start);
            if !timers.is_empty() {
                item.subtitle = i18n::fmt(
                    "{count} minuteur(s) · Entrée pour voir les comptes à rebours",
                    &[("count", timers.len().to_string())],
                );
            }
        }
    }
    for item in &mut s.items {
        if item.kind == Kind::Application {
            let subtitle = i18n::tr(&item.subtitle);
            if subtitle != item.subtitle {
                item.subtitle = subtitle;
                item.prepare();
            }
        }
        if item.kind == Kind::Install && s.installations.is_pending(&item.target) {
            item.subtitle = i18n::tr("Installation en cours · progression dans la console");
        } else if item.kind == Kind::Uninstall && s.removals.is_pending(&item.target) {
            item.subtitle = i18n::tr("Désinstallation en cours · progression dans la console");
        }
    }
    s.selected = selection_after_refresh(&s.items, previous_selection, selected_id.as_deref());
    s.timing = format!("{:.1} ms", start.elapsed().as_secs_f64() * 1000.);
    prepare_result_icons(s);
    request_result_icons(s);
    InvalidateRect(s.hwnd, null(), 0);
}

fn application_icon_size(dpi: i32) -> u32 {
    (24 * dpi / 96).clamp(16, 128) as u32
}

fn prepare_result_icons(s: &mut State) {
    s.item_icon_size = application_icon_size(s.dpi);
    s.item_icon_keys.clear();
    s.item_icon_keys.extend(s.items.iter().map(|item| {
        (item.kind == Kind::Application)
            .then(|| app_icons::Key::new(&item.target, s.item_icon_size))
            .flatten()
    }));
}

// Only schedule visible applications. Extraction and COM stay on the icon worker.
fn request_result_icons(s: &mut State) {
    let size = application_icon_size(s.dpi);
    if s.item_icon_size != size {
        prepare_result_icons(s);
    }
    let offset = s.selected.saturating_sub(s.visible_rows.saturating_sub(1));
    if let Some(icons) = s.app_icons.as_mut() {
        for (item, key) in s
            .items
            .iter()
            .zip(&s.item_icon_keys)
            .skip(offset)
            .take(s.visible_rows)
        {
            if let Some(key) = key {
                icons.request_key(&item.target, key);
            }
        }
    }
}

fn search_catalog(
    index: &[Item],
    query: &str,
    limit: usize,
    weights: &HashMap<String, u32>,
) -> Vec<Item> {
    let query = query.trim();
    let files = query.starts_with('/');
    let body = query.strip_prefix(['@', '/']).unwrap_or(query).trim();
    core::rank_filtered(index, body, limit, weights, |item| {
        if files {
            matches!(item.kind, Kind::File | Kind::Folder)
        } else {
            item.kind == Kind::Application
        }
    })
}

fn timer_query(query: &str) -> Option<&str> {
    let mut parts = query.trim().splitn(2, char::is_whitespace);
    let name = parts.next()?;
    if ["minuteur", "minuteurs", "timer", "timers"]
        .iter()
        .any(|alias| name.eq_ignore_ascii_case(alias))
    {
        Some(parts.next().unwrap_or("").trim())
    } else {
        None
    }
}

fn timer_clock(seconds: u64) -> String {
    if seconds >= 3600 {
        format!(
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

#[inline]
fn timer_item(
    title: impl Into<String>,
    subtitle: impl Into<String>,
    target: impl Into<String>,
) -> Item {
    timer_item_from_strings(title.into(), subtitle.into(), target.into())
}

// Non-generic body shared by every timer_item instantiation.
fn timer_item_from_strings(title: String, subtitle: String, target: String) -> Item {
    Item::new(
        format!("timer:{target}"),
        i18n::tr(&title),
        i18n::tr(&subtitle),
        "\u{E916}",
        Kind::Timer,
        target,
    )
}

fn timer_items(
    book: &timer::TimerBook,
    body: &str,
    focus: Option<timer::TimerId>,
    now: Instant,
) -> Vec<Item> {
    let snapshots = book.snapshots(now);
    if let Some(snapshot) = snapshots.iter().find(|snapshot| Some(snapshot.id) == focus) {
        let caption = format!(
            "{} · {}",
            snapshot.label,
            timer_clock(snapshot.remaining_seconds)
        );
        let mut items = vec![timer_item(
            &caption,
            match snapshot.status {
                timer::TimerStatus::Running => "En cours",
                timer::TimerStatus::Paused => "En pause",
                timer::TimerStatus::Finished => "Terminé",
            },
            "help",
        )];
        items[0].subtitle = i18n::fmt(
            "{status} · Durée initiale {duration}",
            &[
                ("status", items[0].subtitle.clone()),
                ("duration", timer_clock(snapshot.total_seconds)),
            ],
        );
        match snapshot.status {
            timer::TimerStatus::Running => items.push(timer_item(
                "Mettre en pause",
                &snapshot.label,
                format!("pause:{}", snapshot.id),
            )),
            timer::TimerStatus::Paused => items.push(timer_item(
                "Reprendre",
                &snapshot.label,
                format!("resume:{}", snapshot.id),
            )),
            timer::TimerStatus::Finished => items.push(timer_item(
                "Fermer cette alerte",
                &snapshot.label,
                format!("ack:{}", snapshot.id),
            )),
        }
        if snapshot.status != timer::TimerStatus::Finished {
            items.push(timer_item(
                "Annuler ce minuteur",
                &snapshot.label,
                format!("cancel:{}", snapshot.id),
            ));
        }
        items.push(timer_item(
            "Tous les minuteurs",
            "Revenir à la liste",
            "back",
        ));
        return items;
    }
    if !body.is_empty() {
        return vec![match timer::parse(body) {
            Ok(spec) if snapshots.len() < timer::MAX_TIMERS => timer_item(
                i18n::fmt(
                    "Démarrer · {duration}",
                    &[("duration", timer_clock(spec.duration.as_secs()))],
                ),
                i18n::fmt(
                    "{label} · Entrée pour démarrer",
                    &[("label", spec.label.clone())],
                ),
                format!("start:{body}"),
            ),
            Ok(_) => timer_item(
                "Huit minuteurs maximum",
                "Annule ou ferme un minuteur dans la liste pour en ajouter un",
                "back",
            ),
            Err(error) => timer_item(
                error.to_string(),
                "Exemples : minuteur 5m Thé · minuteur 01:30 · minuteur 1h 30m",
                "help",
            ),
        }];
    }
    let mut items: Vec<Item> = snapshots
        .iter()
        .map(|snapshot| {
            timer_item(
                format!(
                    "{} · {}",
                    snapshot.label,
                    if snapshot.status == timer::TimerStatus::Finished {
                        i18n::tr("Terminé")
                    } else {
                        timer_clock(snapshot.remaining_seconds)
                    }
                ),
                match snapshot.status {
                    timer::TimerStatus::Running => "En cours · Entrée pour gérer",
                    timer::TimerStatus::Paused => "En pause · Entrée pour gérer",
                    timer::TimerStatus::Finished => "Entrée pour fermer l'alerte",
                },
                format!("view:{}", snapshot.id),
            )
        })
        .collect();
    if snapshots.len() < timer::MAX_TIMERS {
        for (minutes, label) in [
            (5, "Minuteur"),
            (10, "Minuteur"),
            (25, "Concentration"),
            (60, "Minuteur"),
        ] {
            items.push(timer_item(
                i18n::fmt(
                    "Démarrer · {minutes} minutes",
                    &[("minutes", minutes.to_string())],
                ),
                if label == "Minuteur" {
                    "Entrée pour démarrer"
                } else {
                    label
                },
                format!("start:{minutes}m {label}"),
            ));
        }
    }
    items.push(timer_item(
        "Choisir une durée",
        "Tape minuteur 5m Thé · 90s · 01:30 · 1h 30m (24 h maximum)",
        "help",
    ));
    items
}

unsafe fn refresh_timer_view(s: &mut State) {
    if IsWindowVisible(s.hwnd) == 0
        || !(timer_query(&s.query).is_some()
            || s.query.trim().is_empty()
            || s.query.trim().starts_with(';'))
    {
        return;
    }
    refresh_results(s);
}

unsafe fn schedule_timer_tick(s: &mut State) {
    s.timers.borrow_mut().synchronize();
}

unsafe fn refresh_timer_alert(s: &mut State) {
    s.timers.borrow_mut().refresh_alert();
}

unsafe fn tick_timers(s: &mut State) {
    s.timers.borrow_mut().tick();
    refresh_timer_view(s);
}

unsafe fn acknowledge_timer_alert(s: &mut State, generation: usize) {
    s.timers.borrow_mut().acknowledge_alert(generation);
    refresh_timer_view(s);
}
unsafe fn execute_timer(s: &mut State, item: &Item) {
    tick_timers(s);
    if let Some(body) = item.target.strip_prefix("start:") {
        let result = timer::parse(body)
            .map_err(|error| error.to_string())
            .and_then(|spec| {
                s.timers
                    .borrow_mut()
                    .book
                    .start(spec, Instant::now())
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(id) => {
                set_query(s, "minuteur");
                s.timer_focus = Some(id);
                update_results(s);
            }
            Err(error) => {
                s.status = error;
            }
        }
    } else if item.target == "back" {
        s.timer_focus = None;
        set_query(s, "minuteur");
    } else if let Some((action, id)) = item
        .target
        .split_once(':')
        .and_then(|(action, id)| id.parse::<timer::TimerId>().ok().map(|id| (action, id)))
    {
        let now = Instant::now();
        let result = match action {
            "view" => {
                s.timer_focus = Some(id);
                Ok(())
            }
            "pause" => s.timers.borrow_mut().book.pause(id, now),
            "resume" => s.timers.borrow_mut().book.resume(id, now),
            "cancel" => s.timers.borrow_mut().book.cancel(id),
            "ack" => s.timers.borrow_mut().book.acknowledge(id, now),
            _ => return,
        };
        if let Err(error) = result {
            s.status = error.to_string();
        }
        if matches!(action, "cancel" | "ack") {
            s.timer_focus = None;
        }
        refresh_timer_alert(s);
        update_results(s);
    }
    schedule_timer_tick(s);
    InvalidateRect(s.hwnd, null(), 0);
}

fn active_tab(query: &str) -> &'static str {
    if timer_query(query).is_some() {
        return "Modules  ;";
    }
    match query.trim().chars().next() {
        None => "Accueil",
        Some('/') => "Fichiers",
        Some('?') => "Web",
        Some(';') => "Modules  ;",
        Some('=' | '>' | ':' | '.' | '~' | '!') => "",
        _ => "Applications",
    }
}

fn system_items() -> Vec<Item> {
    [
        (
            "Super Menu · Paramètres",
            "Personnaliser ton menu",
            "settings",
        ),
        (
            "À propos de Super Menu",
            "Version, matériel et informations Windows",
            "about",
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
            i18n::tr(title),
            i18n::tr(sub),
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
    if s.smoke_fixture && !fixture_action_allowed(&item) {
        s.fixture_activation_attempts += 1;
        s.status = i18n::tr("Activation externe désactivée pendant les tests");
        InvalidateRect(s.hwnd, null(), 0);
        return;
    }
    let result = match item.kind {
        Kind::Application => {
            start_application(s, &item);
            return;
        }
        Kind::Install => {
            start_install(s, &item.target);
            return;
        }
        Kind::Uninstall => {
            start_remove(s, &item);
            return;
        }
        Kind::Timer => {
            execute_timer(s, &item);
            return;
        }
        Kind::Navigation => {
            if item.target == "about" {
                show_about(s);
                return;
            }
            set_query(s, &item.target);
            return;
        }
        Kind::Calculator | Kind::Clipboard | Kind::Symbol => copy_text(s.hwnd, &item.target),
        Kind::Window => {
            hide(s);
            native::activate_window(&item.target)
        }
        Kind::System => match item.target.as_str() {
            "about" => {
                show_about(s);
                return;
            }
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

unsafe fn start_application(s: &mut State, item: &Item) {
    let started = match s.launches.start(item, Instant::now()) {
        Ok(started) => started,
        Err(error) => {
            s.status = error.into();
            InvalidateRect(s.hwnd, null(), 0);
            return;
        }
    };
    let token = match started {
        LaunchStart::Started(token) | LaunchStart::Duplicate(token) => token,
    };
    hide(s);
    s.feedback_request = None;
    s.feedback_artwork = None;
    let key = app_icons::Key::new(&item.target, launch_feedback::icon_size(s.dpi));
    let exact_icon = s
        .app_icons
        .as_ref()
        .and_then(|icons| key.as_ref().and_then(|key| icons.get_key(key)));
    // Reuse the visible row's artwork immediately while the worker obtains
    // the popup's larger size. Shell extraction never delays the launch.
    let icon = exact_icon.or_else(|| {
        s.app_icons.as_ref().and_then(|icons| {
            app_icons::Key::new(&item.target, s.item_icon_size)
                .as_ref()
                .and_then(|key| icons.get_key(key))
        })
    });
    if let Some(feedback) = &mut s.feedback {
        if feedback
            .show(&item.title, icon, s.hwnd, &s.settings)
            .is_ok()
        {
            s.feedback_request = Some(token);
            if exact_icon.is_none() {
                s.feedback_artwork = key.map(|key| (item.target.clone(), key));
            }
        }
    }
    request_feedback_icon(s);
    if matches!(started, LaunchStart::Duplicate(_)) {
        return;
    }
    let target = item.target.clone();
    let arguments = item.arguments.clone();
    let sender = s.launch_sender.clone();
    let hwnd = s.hwnd as usize;
    let parent_pid = GetCurrentProcessId();
    let test = s.feedback_test;
    let spawned = std::thread::Builder::new()
        .name("supermenu-launch".into())
        .spawn(move || {
            let result = if test {
                // Diagnostic mode exercises delayed completion without opening another app.
                std::thread::sleep(Duration::from_millis(1800));
                Ok(())
            } else {
                native::launch_background(&target, &arguments)
            };
            if sender.send((token, result)).is_ok() {
                let mut actual_pid = 0;
                GetWindowThreadProcessId(hwnd as HWND, &mut actual_pid);
                if actual_pid == parent_pid {
                    PostMessageW(hwnd as HWND, LAUNCH_DONE, 0, 0);
                }
            }
        });
    if let Err(error) = spawned {
        let _ = s.launch_sender.send((
            token,
            Err(i18n::fmt(
                "Impossible de lancer l'application : {error}",
                &[("error", error.to_string())],
            )),
        ));
        process_launch_results(s);
    }
}

fn request_feedback_icon(s: &mut State) {
    if !s
        .feedback
        .as_ref()
        .is_some_and(|feedback| feedback.is_active())
    {
        s.feedback_artwork = None;
        return;
    }
    let Some((target, key)) = &s.feedback_artwork else {
        return;
    };
    let Some(icons) = &mut s.app_icons else {
        s.feedback_artwork = None;
        return;
    };
    // A full worker queue is retried on its next wake. Only the latest popup
    // retains a key, so a late response cannot replace another app's artwork.
    icons.request_key(target, key);
    if let Some(icon) = icons.get_key(key) {
        if let Some(feedback) = &mut s.feedback {
            feedback.update_icon(icon);
        }
        s.feedback_artwork = None;
    }
}

unsafe fn process_launch_results(s: &mut State) {
    while let Ok((token, result)) = s.launch_results.try_recv() {
        let Some(item) = s.launches.finish(token, result.is_ok()) else {
            continue;
        };
        match result {
            Ok(()) => {
                if s.feedback_test && item.id == "feedback-test" {
                    s.feedback_test_finished = true;
                } else if !s.smoke {
                    s.usage.record(&item.id);
                    let index = s.index.lock().unwrap_or_else(|e| e.into_inner());
                    s.weights = s.usage.snapshot_for(&index);
                }
                // Shell acceptance never reopens the popup or changes foreground focus.
            }
            Err(error) => {
                if s.feedback_request == Some(token) {
                    if let Some(feedback) = &mut s.feedback {
                        feedback.hide();
                    }
                    s.feedback_request = None;
                    s.feedback_artwork = None;
                }
                s.status = i18n::fmt(
                    "Lancement de {name} impossible",
                    &[("name", item.title.clone())],
                );
                s.settings_open = true;
                show_error(&error);
                s.settings_open = false;
                finish_modal(s);
                if IsWindowVisible(s.hwnd) != 0 {
                    SetFocus(s.edit);
                }
            }
        }
    }
}

unsafe fn start_install(s: &mut State, id: &str) {
    if packages::find(id).is_none() || s.removals.is_pending(&format!("id:{id}")) {
        return;
    }
    let Some(request) = s.installations.start(id) else {
        return;
    };
    let parent_pid = GetCurrentProcessId();
    match installer::spawn(id, s.hwnd, parent_pid, request) {
        Ok(mut child) => {
            let hwnd = s.hwnd as usize;
            std::thread::spawn(move || {
                let result = child
                    .wait()
                    .ok()
                    .and_then(|status| status.code())
                    .unwrap_or(1);
                // The helper normally notifies before waiting for Enter. This covers
                // a forcibly closed console or a helper that exits unexpectedly.
                installer::notify(hwnd as HWND, parent_pid, request, result);
            });
            s.status = i18n::tr("Installation en cours · console WinGet");
            update_results(s);
            hide(s);
        }
        Err(error) => {
            s.installations.finish(request, false);
            s.settings_open = true;
            show_error(&error);
            s.settings_open = false;
            finish_modal(s);
            SetFocus(s.edit);
        }
    }
}

unsafe fn start_remove(s: &mut State, item: &Item) {
    if item.kind != Kind::Uninstall
        || s.removals.is_pending(&item.target)
        || s.removals.completed.contains(&item.target)
        || !s
            .removable_packages
            .iter()
            .any(|entry| entry.target == item.target)
    {
        return;
    }
    if item
        .target
        .strip_prefix("id:")
        .is_some_and(|id| s.installations.is_pending(id))
    {
        s.status = i18n::tr("Installation en cours · réessaie après son achèvement");
        InvalidateRect(s.hwnd, null(), 0);
        return;
    }
    s.settings_open = true;
    let confirmation = MessageBoxW(
        s.hwnd,
        wide(&i18n::fmt(
            "Désinstaller « {name} » et nettoyer ses résidus ?\n\nLe logiciel sera retiré, puis ses fichiers de programme, caches reconnus et traces techniques seront recherchés automatiquement.\n\nLe nettoyage conservera les profils, réglages et données personnelles. Les résidus incertains ou partagés seront laissés en place.\n\nLes fichiers retirés iront dans la Corbeille ; un rapport et les sauvegardes du registre resteront disponibles.",
            &[("name", item.title.clone())]
        ))
        .as_ptr(),
        wide(&i18n::tr("Super Menu · Désinstaller")).as_ptr(),
        MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING,
    );
    s.settings_open = false;
    finish_modal(s);
    if confirmation != IDYES {
        SetFocus(s.edit);
        return;
    }
    let Some(request) = s.removals.start(&item.target) else {
        return;
    };
    let parent_pid = GetCurrentProcessId();
    match installer::spawn_remove(&item.target, s.hwnd, parent_pid, request) {
        Ok(mut child) => {
            let hwnd = s.hwnd as usize;
            std::thread::spawn(move || {
                let result = child
                    .wait()
                    .ok()
                    .and_then(|status| status.code())
                    .unwrap_or(1);
                installer::notify_remove(hwnd as HWND, parent_pid, request, result);
            });
            s.status = i18n::tr("Désinstallation et nettoyage en cours · voir la console");
            update_results(s);
            hide(s);
        }
        Err(error) => {
            s.removals.finish(request, false);
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
        return Err(i18n::tr("Le presse-papiers est occupé. Réessaie."));
    }
    let value = wide(text);
    let memory = GlobalAlloc(GMEM_MOVEABLE, value.len() * 2);
    if memory.is_null() {
        CloseClipboard();
        return Err(i18n::tr("Mémoire insuffisante."));
    }
    let ptr = GlobalLock(memory);
    if ptr.is_null() {
        GlobalFree(memory);
        CloseClipboard();
        return Err(i18n::tr("Accès au presse-papiers impossible."));
    }
    std::ptr::copy_nonoverlapping(value.as_ptr(), ptr as *mut u16, value.len());
    GlobalUnlock(memory);
    EmptyClipboard();
    if SetClipboardData(CF_UNICODETEXT, memory).is_null() {
        GlobalFree(memory);
        CloseClipboard();
        return Err(i18n::tr("Copie impossible."));
    }
    CloseClipboard();
    Ok(())
}

unsafe fn refresh_index_and_icons(s: &mut State) {
    if let Some(icons) = s.app_icons.as_mut() {
        icons.clear();
    }
    reindex(s);
    request_result_icons(s);
    InvalidateRect(s.hwnd, null(), 0);
}

// Keep the opt-in path out of per-keystroke ranking: only publications and
// settings changes filter generated entries. Ordinary Steam shortcuts stay intact.
fn retain_enabled_steam(items: &mut Vec<Item>, enabled: bool) {
    if !enabled {
        items.retain(|item| !item.id.starts_with("steam:"));
    }
}

fn merge_steam_games(apps: &mut Vec<Item>, games: Vec<Item>) {
    for game in games {
        let game_id = steam::app_id(&game.target);
        let existing = apps.iter().any(|app| {
            app.kind == Kind::Application
                && steam::app_id(&app.target).map_or_else(
                    || app.title.eq_ignore_ascii_case(&game.title),
                    |id| game_id == Some(id),
                )
        });
        if !existing {
            apps.push(game);
        }
    }
}

fn defer_file_refresh(flags: usize, visible: bool) -> bool {
    flags == native::WATCH_FILES && !visible
}

unsafe fn reindex(s: &mut State) {
    // Explicit/app refreshes also reconcile any pending file changes. A refresh
    // requested during a scan is retained by pending_refresh below.
    s.pending_file_refresh = false;
    if s.smoke_fixture {
        s.indexing.store(false, Ordering::Release);
        PostMessageW(s.hwnd, INDEX, 0, 0);
        return;
    }
    if s.indexing.swap(true, Ordering::AcqRel) {
        s.pending_refresh = true;
        return;
    }
    s.status = i18n::tr("Indexation en arrière-plan…");
    InvalidateRect(s.hwnd, null(), 0);
    let index = s.index.clone();
    let steam_enabled = s.steam_enabled.clone();
    let installed_names = s.installed_names.clone();
    let removable_names = s.removable_names.clone();
    let confirmed_absent_packages = s.confirmed_absent_packages.clone();
    s.index_generation = s.index_generation.saturating_add(1);
    let generation = s.index_generation;
    let catalog_ready = s.catalog_ready.clone();
    let busy = s.indexing.clone();
    let settings = s.settings.clone();
    let language = i18n::current_language();
    let language_revision = i18n::revision();
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
        let mut apps = native::app_catalog();
        let (all_names, remove_names, steam_games) =
            packages::installed_inventory_with_steam(settings.steam_enabled);
        merge_steam_games(&mut apps, steam_games);
        let absent = packages::confirmed_absent_packages(&all_names);
        *confirmed_absent_packages
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = absent;
        *installed_names.lock().unwrap_or_else(|e| e.into_inner()) = all_names;
        *removable_names.lock().unwrap_or_else(|e| e.into_inner()) = remove_names;
        if i18n::revision() == language_revision {
            // Allocate and clone only the small application catalog off-lock.
            // Existing file entries can move into the new snapshot instead of
            // cloning every path and normalized search field while typing waits.
            let previous_count = index.lock().unwrap_or_else(|e| e.into_inner()).len();
            let mut published = Vec::with_capacity(apps.len() + previous_count);
            published.extend_from_slice(&apps);
            let mut index = index.lock().unwrap_or_else(|e| e.into_inner());
            index.retain(|item| matches!(item.kind, Kind::File | Kind::Folder));
            published.append(&mut index);
            // Toggle-off and publication share this lock: an older worker cannot
            // put optional entries back after the settings dialog removed them.
            if settings.steam_enabled {
                retain_enabled_steam(&mut published, steam_enabled.load(Ordering::Acquire));
            }
            *index = published;
        }
        catalog_ready.store(generation, Ordering::Release);
        unsafe {
            PostMessageW(hwnd as HWND, INDEX, 0, 0);
        }
        let previous_files = index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
            .saturating_sub(apps.len())
            .min(settings.max_indexed_files);
        let mut items = apps;
        items.reserve(previous_files);
        catalog::append_files(&settings, &mut items);
        // A first scan or growing catalog can exceed the previous reservation.
        items.shrink_to_fit();
        if i18n::revision() == language_revision {
            let _ = catalog::save_cache(&items, &language);
            let previous = {
                let mut index = index.lock().unwrap_or_else(|e| e.into_inner());
                if settings.steam_enabled {
                    retain_enabled_steam(&mut items, steam_enabled.load(Ordering::Acquire));
                }
                std::mem::replace(&mut *index, items)
            };
            // Releasing thousands of strings must not hold up a concurrent search.
            drop(previous);
        }
        busy.store(false, Ordering::Release);
        unsafe {
            PostMessageW(hwnd as HWND, INDEX, 0, 0);
        }
    });
}

unsafe fn show(s: &mut State) {
    if let Some(feedback) = &mut s.feedback {
        feedback.hide();
    }
    s.feedback_request = None;
    s.feedback_artwork = None;
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
    // Show the cached results immediately; deferred disk work stays off the UI.
    if s.pending_file_refresh {
        reindex(s);
    }
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
    // Move once to obtain the destination DPI only when changing monitors.
    // Reopening on the same screen need not jump to the corner and back.
    if MonitorFromWindow(s.hwnd, MONITOR_DEFAULTTONEAREST) != monitor {
        SetWindowPos(
            s.hwnd,
            null_mut(),
            area.left + 40,
            area.top + 40,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    let old_dpi = s.dpi;
    s.dpi = GetDpiForWindow(s.hwnd).max(96) as i32;
    let width = (s.settings.window_width * s.dpi / 96).min(area.right - area.left - 32);
    let height = (680 * s.dpi / 96).min(area.bottom - area.top - 32);
    s.visible_rows = (((height * 96 / s.dpi) - ROW_TOP - 58) / ROW_HEIGHT).max(1) as usize;
    let x = area.left + (area.right - area.left - width) / 2;
    let y = area.top + ((area.bottom - area.top - height) / 4).max(16);
    let mut current: RECT = zeroed();
    GetWindowRect(s.hwnd, &mut current);
    if current.left != x
        || current.top != y
        || current.right - x != width
        || current.bottom - y != height
    {
        SetWindowPos(s.hwnd, HWND_TOPMOST, x, y, width, height, SWP_NOACTIVATE);
    }
    if old_dpi != s.dpi {
        rebuild_fonts(s);
    }
    layout_edit(s);
    request_result_icons(s);
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
    for (i, (size, weight)) in [
        (20, 400),
        (15, 400),
        (11, 400),
        (11, 600),
        (20, 400),
        (18, 400),
    ]
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
            wide(match i {
                4 => "Segoe UI Symbol",
                5 => "Segoe MDL2 Assets",
                _ => "Segoe UI",
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
    scratch: &mut Vec<u16>,
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
    scratch.clear();
    scratch.extend(value.encode_utf16());
    scratch.push(0);
    DrawTextW(
        dc,
        scratch.as_ptr(),
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
fn centered_glyph_origin(bounds: &RECT, metrics: &GLYPHMETRICS) -> Option<(i32, i32)> {
    let width = bounds.right.checked_sub(bounds.left)?;
    let height = bounds.bottom.checked_sub(bounds.top)?;
    let ink_width = i32::try_from(metrics.gmBlackBoxX).ok()?;
    let ink_height = i32::try_from(metrics.gmBlackBoxY).ok()?;
    if width <= 0 || height <= 0 || ink_width <= 0 || ink_height <= 0 {
        return None;
    }
    let x = bounds
        .left
        .checked_add((width - ink_width) / 2)?
        .checked_sub(metrics.gmptGlyphOrigin.x)?;
    let y = bounds
        .top
        .checked_add((height - ink_height) / 2)?
        .checked_add(metrics.gmptGlyphOrigin.y)?;
    Some((x, y))
}

unsafe fn draw_centered_glyph(dc: HDC, glyph: u16, bounds: &RECT) -> bool {
    let mut index = 0;
    if GetGlyphIndicesW(dc, &glyph, 1, &mut index, GGI_MARK_NONEXISTING_GLYPHS) == GDI_ERROR as u32
        || index == u16::MAX
    {
        return false;
    }
    let identity = MAT2 {
        eM11: FIXED { fract: 0, value: 1 },
        eM22: FIXED { fract: 0, value: 1 },
        ..zeroed()
    };
    let mut metrics: GLYPHMETRICS = zeroed();
    if GetGlyphOutlineW(
        dc,
        u32::from(index),
        GGO_METRICS | GGO_GLYPH_INDEX,
        &mut metrics,
        0,
        null_mut(),
        &identity,
    ) == GDI_ERROR as u32
    {
        return false;
    }
    let Some((x, y)) = centered_glyph_origin(bounds, &metrics) else {
        return false;
    };
    let previous = SetTextAlign(dc, TA_LEFT | TA_BASELINE | TA_NOUPDATECP);
    if previous == GDI_ERROR as u32 {
        return false;
    }
    let drawn = ExtTextOutW(
        dc,
        x,
        y,
        ETO_CLIPPED | ETO_GLYPH_INDEX,
        bounds,
        &index,
        1,
        null(),
    );
    SetTextAlign(dc, previous);
    drawn != 0
}

unsafe fn shortcut_text(
    dc: HDC,
    scratch: &mut Vec<u16>,
    s: &State,
    label: &str,
    selected: bool,
    mut bounds: RECT,
) {
    let old = SelectObject(dc, s.fonts[2]);
    SetTextColor(
        dc,
        if selected {
            s.palette.accent
        } else {
            s.palette.muted
        },
    );
    SetBkMode(dc, TRANSPARENT as i32);
    scratch.clear();
    scratch.extend(label.encode_utf16());
    scratch.push(0);
    if scratch.len() != 2 || !draw_centered_glyph(dc, scratch[0], &bounds) {
        DrawTextW(
            dc,
            scratch.as_ptr(),
            -1,
            &mut bounds,
            DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
    }
    SelectObject(dc, old);
}

// Center the visible glyph, rather than its font's line height and side bearings.
// Keep the original text renderer for multi-character symbols or font fallback.
unsafe fn result_icon(
    dc: HDC,
    scratch: &mut Vec<u16>,
    s: &State,
    item: &Item,
    key: Option<&app_icons::Key>,
    mut bounds: RECT,
) {
    if item.kind == Kind::Application {
        let size = application_icon_size(s.dpi);
        if let Some(icon) = s
            .app_icons
            .as_ref()
            .and_then(|icons| key.and_then(|key| icons.get_key(key)))
        {
            let pixels = size as i32;
            let x = bounds.left + (bounds.right - bounds.left - pixels) / 2;
            let y = bounds.top + (bounds.bottom - bounds.top - pixels) / 2;
            if DrawIconEx(dc, x, y, icon, pixels, pixels, 0, null_mut(), DI_NORMAL) != 0 {
                return;
            }
        }
    }
    let glyph = match item.kind {
        Kind::Install => "\u{E896}",   // Download
        Kind::Uninstall => "\u{E74D}", // Delete
        Kind::Navigation if item.target.trim() == "+" => "\u{E896}",
        Kind::Navigation if item.target.trim() == "-" => "\u{E74D}",
        _ => &item.glyph,
    };
    let mut characters = glyph.chars();
    let is_windows_icon = characters
        .next()
        .is_some_and(|character| ('\u{E000}'..='\u{F8FF}').contains(&character))
        && characters.next().is_none();
    let old = SelectObject(dc, s.fonts[if is_windows_icon { 5 } else { 4 }]);
    SetTextColor(dc, s.palette.accent);
    SetBkMode(dc, TRANSPARENT as i32);
    scratch.clear();
    scratch.extend(glyph.encode_utf16());
    scratch.push(0);
    if scratch.len() != 2 || !draw_centered_glyph(dc, scratch[0], &bounds) {
        DrawTextW(
            dc,
            scratch.as_ptr(),
            -1,
            &mut bounds,
            DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_NOPREFIX,
        );
    }
    SelectObject(dc, old);
}
unsafe fn rect(dc: HDC, c: u32, x: i32, y: i32, w: i32, h: i32, radius: i32) {
    // Windows supplies a per-DC stock brush. Recoloring it avoids creating and
    // deleting dozens of GDI objects for each frame, with the same geometry.
    let old = SelectObject(dc, GetStockObject(DC_BRUSH));
    let previous_color = SetDCBrushColor(dc, c);
    let pen = SelectObject(dc, GetStockObject(NULL_PEN));
    if radius > 0 {
        RoundRect(dc, x, y, x + w, y + h, radius, radius);
    } else {
        PatBlt(dc, x, y, w, h, PATCOPY);
    }
    SelectObject(dc, old);
    SelectObject(dc, pen);
    SetDCBrushColor(dc, previous_color);
}
// Compose a complete parent frame before copying it to the visible window.
// BeginPaint’s destination clip preserves the child EDIT and its caret.
unsafe fn paint_frame(s: &mut State, dc: HDC) {
    let mut bounds: RECT = zeroed();
    if GetClientRect(s.hwnd, &mut bounds) != 0 {
        if let Some(buffer) = s.paint_buffer.target(dc, bounds.right, bounds.bottom) {
            paint(s, buffer);
            if s.paint_buffer.present(dc) {
                return;
            }
            // Recreate a compatible surface next time if the display device changed.
            s.paint_buffer.reset();
        }
    }
    // Allocation or transfer failure still produces a usable frame.
    paint(s, dc);
}

unsafe fn paint(s: &State, dc: HDC) {
    let mut scratch = s.paint_text.borrow_mut();
    let scratch = &mut *scratch;
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
        scratch,
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
        scratch,
        s.fonts[2],
        s.palette.muted,
        &s.cached_ui.tagline,
        scale(162),
        scale(20),
        scale(250),
        scale(28),
        false,
    );
    text(
        dc,
        scratch,
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
        scratch,
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
        scratch,
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
        scratch,
        s.fonts[2],
        s.palette.muted,
        &s.cached_ui.hotkey,
        scale(w - 165),
        scale(78),
        scale(126),
        scale(25),
        true,
    );
    for ((label, x, width), translated) in [
        ("Accueil", 28, 48),
        ("Applications", 89, 117),
        ("Fichiers", 224, 77),
        ("Web", 322, 48),
        ("Modules  ;", 389, 112),
    ]
    .into_iter()
    .zip(&s.cached_ui.tabs)
    {
        text(
            dc,
            scratch,
            s.fonts[2],
            if label == active_tab(&s.query) {
                s.palette.accent
            } else {
                s.palette.muted
            },
            translated,
            scale(x),
            scale(128),
            scale(width),
            scale(25),
            false,
        );
    }
    text(
        dc,
        scratch,
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
        scratch,
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
        let shortcut = navigation_shortcut(item, &s.cached_ui);
        let content_width = shortcut.map_or(w - 134, |(_, width)| w - 62 - width - 12 - 83);
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
        result_icon(
            dc,
            scratch,
            s,
            item,
            s.item_icon_keys.get(row + offset).and_then(Option::as_ref),
            RECT {
                left: scale(36),
                top: scale(y + 8),
                right: scale(36) + scale(31),
                bottom: scale(y + 8) + scale(31),
            },
        );
        text(
            dc,
            scratch,
            s.fonts[1],
            if row + offset == s.selected {
                s.palette.accent
            } else {
                s.palette.text
            },
            &item.title,
            scale(83),
            scale(y + 4),
            scale(content_width),
            scale(23),
            false,
        );
        text(
            dc,
            scratch,
            s.fonts[2],
            s.palette.muted,
            &item.subtitle,
            scale(83),
            scale(y + 26),
            scale(content_width),
            scale(16),
            false,
        );
        if let Some((label, width)) = shortcut {
            let x = w - 62 - width;
            rect(
                dc,
                s.palette.panel,
                scale(x),
                scale(y + 12),
                scale(width),
                scale(24),
                scale(5),
            );
            shortcut_text(
                dc,
                scratch,
                s,
                label,
                row + offset == s.selected,
                RECT {
                    left: scale(x),
                    top: scale(y + 12),
                    right: scale(x) + scale(width),
                    bottom: scale(y + 12) + scale(24),
                },
            );
        }
        text(
            dc,
            scratch,
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
            scratch,
            s.fonts[0],
            s.palette.text,
            &s.cached_ui.empty,
            scale(28),
            scale(y),
            scale(w - 56),
            scale(40),
            false,
        );
        text(
            dc,
            scratch,
            s.fonts[2],
            s.palette.muted,
            if s.indexing.load(Ordering::Relaxed) {
                &s.cached_ui.empty_indexing
            } else {
                &s.cached_ui.empty_hint
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
        scratch,
        s.fonts[2],
        s.palette.muted,
        &s.cached_ui.hints[match s.items.get(s.selected).map(|item| &item.kind) {
            Some(Kind::Install) => 1,
            Some(Kind::Uninstall) => 2,
            Some(Kind::Timer) => 3,
            _ => 0,
        }],
        scale(28),
        scale(h - 39),
        scale(347),
        scale(25),
        false,
    );
    text(
        dc,
        scratch,
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

unsafe fn tray_icon_size() -> u32 {
    let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
    let dpi = GetDpiForWindow(taskbar);
    let dpi = if dpi == 0 {
        GetDpiForSystem().max(96)
    } else {
        dpi
    };
    GetSystemMetricsForDpi(SM_CXSMICON, dpi).clamp(16, 64) as u32
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
        let placeholder = i18n::tr("Où veux-tu aller ?");
        let text_value = if s.query.is_empty() {
            &placeholder
        } else {
            &s.query
        };
        text(
            mem,
            &mut s.paint_text.borrow_mut(),
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

fn about_context(s: &State) -> about::Context {
    let index = s.index.lock().unwrap_or_else(|error| error.into_inner());
    about::Context {
        theme: s.settings.theme.clone(),
        accent_color: s.settings.accent_color.clone(),
        hotkey: s.settings.hotkey.clone(),
        applications: index
            .iter()
            .filter(|item| item.kind == Kind::Application)
            .count(),
        files: index
            .iter()
            .filter(|item| matches!(item.kind, Kind::File | Kind::Folder))
            .count(),
        index_items: index.len(),
        app_uptime: s.started.elapsed(),
        external_links: !s.smoke_fixture,
    }
}

unsafe fn show_about(s: &mut State) {
    let context = about_context(s);
    // Opening the information window should keep focus there instead of
    // restoring the previously active application through hide().
    ShowWindow(s.hwnd, SW_HIDE);
    if let Some(window) = s.about.as_mut() {
        window.show(context);
    } else {
        match about::AboutWindow::new(s.hwnd, context) {
            Ok(mut window) => {
                window.show(about_context(s));
                s.about = Some(window);
            }
            Err(error) => {
                show(s);
                show_error(&error);
            }
        }
    }
}

unsafe fn refresh_language(s: &mut State) {
    s.cached_ui = CachedUi::new(&s.settings);
    SendMessageW(
        s.edit,
        EM_SETCUEBANNER,
        1,
        wide(&i18n::tr("Où veux-tu aller ?")).as_ptr() as isize,
    );
    s.status = i18n::tr("Index local");
}

// A native modal settings window. No embedded browser and no UI framework runtime.
unsafe fn settings(s: &mut State) {
    if s.smoke_fixture {
        return;
    }
    if s.settings_open {
        return;
    }
    s.settings_open = true;
    if let Some(settings) = settings_dialog(s) {
        let cache = config::data_dir().join("languages");
        if let Err(error) = i18n::load_cached(&settings.language, &cache) {
            show_error(&error);
            s.settings_open = false;
            finish_modal(s);
            return;
        }
        if let Err(error) = config::save(&settings) {
            let _ = i18n::load_cached(&s.settings.language, &cache);
            show_error(&error.to_string());
            s.settings_open = false;
            finish_modal(s);
            return;
        }
        s.steam_enabled
            .store(settings.steam_enabled, Ordering::Release);
        if s.settings.steam_enabled && !settings.steam_enabled {
            retain_enabled_steam(
                &mut s.index.lock().unwrap_or_else(|e| e.into_inner()),
                false,
            );
            if let Some(icons) = s.app_icons.as_mut() {
                icons.clear();
            }
        }
        s.settings = settings;
        s.timers.borrow_mut().settings = s.settings.clone();
        s.palette = Palette::from(&s.settings);
        s.clipboard.clear();
        refresh_language(s);
        s.about.take();
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
    if s.feedback_test {
        s.checks.push((
            "launch_completion_without_ui_wait".into(),
            s.feedback_test_finished && s.launches.pending.is_empty(),
        ));
        s.checks.push((
            "launch_popup_expired".into(),
            s.feedback
                .as_ref()
                .is_some_and(|feedback| IsWindowVisible(feedback.hwnd()) == 0),
        ));
    }
    set_query(s, "");
    s.checks.push(("all_providers".into(), s.items.len() == 13));
    s.checks.push((
        "about_menu_entry".into(),
        s.items.iter().any(|item| item.target == "about")
            && system_items().iter().any(|item| item.target == "about"),
    ));
    s.checks.push((
        "timer_menu_entry".into(),
        s.items.iter().any(|item| item.target == "minuteur "),
    ));
    let existing_timers = s.timers.borrow().book.snapshots(Instant::now()).len();
    set_query(s, "minuteur 5m Thé");
    s.checks.push((
        "timer_preview_without_start".into(),
        s.items
            .first()
            .is_some_and(|item| item.kind == Kind::Timer && item.target == "start:5m Thé")
            && s.timers.borrow().book.snapshots(Instant::now()).len() == existing_timers,
    ));
    set_query(s, "minuteur -5s");
    s.checks.push((
        "timer_invalid_duration".into(),
        s.items
            .first()
            .is_some_and(|item| item.kind == Kind::Timer && item.target == "help"),
    ));
    set_query(s, "");
    s.checks.push((
        "package_menu_entries".into(),
        s.items
            .iter()
            .any(|item| item.id == "nav:+ " && item.target == "+ ")
            && s.items
                .iter()
                .any(|item| item.id == "nav:- " && item.target == "- "),
    ));
    set_query(s, "@ ");
    s.checks.push((
        "applications".into(),
        !s.items.is_empty() && s.items.iter().all(|i| i.kind == Kind::Application),
    ));
    let app_query = s
        .items
        .iter()
        .find(|item| {
            !item
                .title
                .starts_with(['@', '/', '+', '-', '=', '?', '>', ':', '.', '~', '!', ';'])
        })
        .map(|item| item.title.clone())
        .unwrap_or_default();
    set_query(s, &app_query);
    s.checks.push((
        "default_applications".into(),
        !s.items.is_empty()
            && s.items
                .iter()
                .all(|i| matches!(i.kind, Kind::Application | Kind::Install | Kind::Uninstall)),
    ));
    set_query(s, "/ ");
    s.checks.push((
        "explicit_files".into(),
        s.items
            .iter()
            .all(|i| matches!(i.kind, Kind::File | Kind::Folder)),
    ));
    set_query(s, "+ ");
    s.checks.push((
        "install_catalog".into(),
        packages::PACKAGES.len() >= 40
            && s.items
                .iter()
                .all(|item| item.kind == Kind::Install && packages::find(&item.target).is_some()),
    ));
    set_query(s, "- ");
    s.checks.push((
        "remove_catalog".into(),
        s.items.iter().all(|item| {
            item.kind == Kind::Uninstall
                && (item.target.starts_with("id:") || item.target.starts_with("name:"))
        }),
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
    PostMessageW(s.hwnd, INDEX, 0, 0);
    replay_deferred(s.hwnd);
}

unsafe fn replay_deferred(hwnd: HWND) {
    let deferred = DEFERRED.with(|d| d.replace(0));
    if deferred & 1 != 0 {
        PostMessageW(hwnd, INDEX, 0, 0);
    }
    if deferred & (2 | 64) != 0 {
        let flags = if deferred & 2 != 0 {
            0
        } else {
            native::WATCH_FILES
        };
        PostMessageW(hwnd, REFRESH, flags, 0);
    }
    if deferred & 4 != 0 {
        PostMessageW(hwnd, LAUNCH_DONE, 0, 0);
    }
    if deferred & 8 != 0 {
        PostMessageW(hwnd, WM_TIMER, TIMER_TICK, 0);
    }
    if deferred & 16 != 0 {
        PostMessageW(hwnd, ICONS_READY, 0, 0);
    }
    if deferred & 32 != 0 {
        PostMessageW(hwnd, ABOUT, 0, 0);
    }
    DEFERRED_TIMER_ACK.with(|d| {
        for generation in d.borrow_mut().drain(..) {
            PostMessageW(hwnd, timer_alert::WM_TIMER_ALERT_DISMISSED, generation, 0);
        }
    });
    DEFERRED_INSTALL.with(|d| {
        for (wp, lp) in d.borrow_mut().drain(..) {
            PostMessageW(hwnd, installer::INSTALL_DONE, wp, lp);
        }
    });
    DEFERRED_REMOVE.with(|d| {
        for (wp, lp) in d.borrow_mut().drain(..) {
            PostMessageW(hwnd, installer::REMOVE_DONE, wp, lp);
        }
    });
}

struct SettingsDialog {
    source: config::Settings,
    result: Option<config::Settings>,
    done: bool,
    controls: Vec<HWND>,
    font: HFONT,
    dpi: i32,
    language_button: HWND,
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
        font: s.fonts[1],
        dpi: s.dpi,
        language_button: null_mut(),
    });
    let mut parent: RECT = zeroed();
    GetWindowRect(s.hwnd, &mut parent);
    let scale = |value: i32| value * s.dpi / 96;
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_DLGMODALFRAME,
        class.as_ptr(),
        wide(&i18n::tr("Super Menu · Paramètres")).as_ptr(),
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
        parent.left + scale(20),
        parent.top + scale(15),
        scale(640),
        scale(686),
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
                wide(&i18n::tr(label)).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                scale(x),
                scale(if y >= 253 {
                    y + 78
                } else if y >= 54 {
                    y + 42
                } else {
                    y
                }),
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
    // This row downloads on a worker and only changes the active language on Save.
    let language_name = i18n::LANGUAGES
        .iter()
        .find(|entry| entry.code == s.settings.language)
        .map_or("Français", |entry| entry.name);
    let language_button = CreateWindowExW(
        0,
        wide("BUTTON").as_ptr(),
        wide(&i18n::fmt(
            "{language} · Changer…",
            &[("language", language_name.to_string())],
        ))
        .as_ptr(),
        WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON as u32 | WS_TABSTOP,
        scale(187),
        scale(54),
        scale(260),
        scale(30),
        hwnd,
        211 as _,
        instance,
        null(),
    );
    SendMessageW(language_button, WM_SETFONT, s.fonts[1] as usize, 1);
    dialog.language_button = language_button;
    let language_label = CreateWindowExW(
        0,
        wide("STATIC").as_ptr(),
        wide(&i18n::tr("Langue")).as_ptr(),
        WS_CHILD | WS_VISIBLE,
        scale(22),
        scale(57),
        scale(155),
        scale(23),
        hwnd,
        null_mut(),
        instance,
        null(),
    );
    SendMessageW(language_label, WM_SETFONT, s.fonts[1] as usize, 1);
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
        SendMessageW(
            theme,
            CB_ADDSTRING,
            0,
            wide(&i18n::tr(label)).as_ptr() as isize,
        );
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
    let steam = control(
        "BUTTON",
        "Jeux Steam installés",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP,
        22,
        247,
        580,
        26,
        212,
    );
    SendMessageW(
        steam,
        BM_SETCHECK,
        if s.settings.steam_enabled {
            BST_CHECKED as usize
        } else {
            BST_UNCHECKED as usize
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
    dialog.controls = vec![hotkey, theme, accent, clip, startup, roots, web, steam];
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
            211 => {
                if let Some(language) = language_dialog::choose(
                    hwnd,
                    d.font,
                    d.dpi,
                    &d.source.language,
                    config::data_dir().join("languages"),
                ) {
                    d.source.language = language;
                    let name = i18n::LANGUAGES
                        .iter()
                        .find(|entry| entry.code == d.source.language)
                        .map_or("Français", |entry| entry.name);
                    SetWindowTextW(
                        d.language_button,
                        wide(&i18n::fmt(
                            "{language} · Changer…",
                            &[("language", name.to_string())],
                        ))
                        .as_ptr(),
                    );
                }
                0
            }
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
                if d.controls.len() < 8 {
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
                        wide(&i18n::tr(
                            "Utilise une couleur hexadécimale, par exemple #9A8CFF.",
                        ))
                        .as_ptr(),
                        wide(&i18n::tr("Couleur")).as_ptr(),
                        MB_OK | MB_ICONWARNING,
                    );
                    return 0;
                }
                settings.clipboard_enabled =
                    SendMessageW(d.controls[3], BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
                settings.steam_enabled =
                    SendMessageW(d.controls[7], BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
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
                    MessageBoxW(hwnd,wide(&i18n::tr("L'adresse de recherche doit commencer par https:// et contenir {query}.")).as_ptr(),wide(&i18n::tr("Recherche web")).as_ptr(),MB_OK|MB_ICONWARNING);
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
    fn only_hidden_file_changes_are_deferred() {
        for flags in [
            0,
            native::WATCH_APPLICATIONS,
            native::WATCH_FILES | native::WATCH_APPLICATIONS,
        ] {
            assert!(!defer_file_refresh(flags, false));
            assert!(!defer_file_refresh(flags, true));
        }
        assert!(defer_file_refresh(native::WATCH_FILES, false));
        assert!(!defer_file_refresh(native::WATCH_FILES, true));
    }

    fn steam_game(id: u32, title: &str) -> Item {
        Item::new(
            format!("steam:{id}"),
            title,
            "Jeu Steam",
            "",
            Kind::Application,
            format!("steam://run/{id}"),
        )
    }

    #[test]
    fn steam_toggle_removes_generated_entries_but_keeps_existing_shortcuts() {
        let shortcut = Item::new(
            "shortcut:artisan",
            "Artisan TD",
            "Application Windows",
            "",
            Kind::Application,
            "steam://run/2224640",
        );
        let ordinary = Item::new("app", "Steam", "", "", Kind::Application, "steam.exe");
        let mut items = vec![shortcut, steam_game(2224640, "Artisan TD"), ordinary];
        retain_enabled_steam(&mut items, true);
        assert_eq!(items.len(), 3);
        retain_enabled_steam(&mut items, false);
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["shortcut:artisan", "app"]
        );
        // A stale worker snapshot obeys the latest toggle before publication.
        items.push(steam_game(2224640, "Artisan TD"));
        retain_enabled_steam(&mut items, false);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn steam_games_deduplicate_launch_ids_and_existing_shortcut_titles() {
        let mut apps = vec![
            Item::new(
                "shortcut:alias",
                "My favorite game",
                "",
                "",
                Kind::Application,
                "steam://run/2224640",
            ),
            Item::new(
                "shortcut:name",
                "Portal 2",
                "",
                "",
                Kind::Application,
                r"C:\Games\Portal 2.lnk",
            ),
        ];
        merge_steam_games(
            &mut apps,
            vec![
                steam_game(2224640, "Artisan TD"),
                steam_game(620, "portal 2"),
                steam_game(440, "Team Fortress 2"),
                steam_game(440, "Team Fortress 2"),
            ],
        );
        assert_eq!(apps.len(), 3);
        assert_eq!(apps[2].id, "steam:440");
        assert_eq!(apps[2].target, "steam://run/440");
    }

    #[test]
    fn steam_deduplication_uses_known_launch_ids_before_names() {
        let mut apps = vec![steam_game(100, "Same title")];
        merge_steam_games(&mut apps, vec![steam_game(200, "Same title")]);
        assert_eq!(apps.len(), 2);
    }

    #[test]
    fn refreshed_selection_follows_identity_when_results_move() {
        let item = |id| Item::new(id, id, "", "", Kind::Application, "");
        let results = [item("new"), item("first"), item("selected")];
        assert_eq!(selection_after_refresh(&results, 1, Some("selected")), 2);
        assert_eq!(selection_after_refresh(&results, 2, Some("first")), 1);
        assert_eq!(selection_after_refresh(&results, 0, None), 0);
    }

    #[test]
    fn missing_selection_keeps_the_nearest_valid_row() {
        let item = |id| Item::new(id, id, "", "", Kind::Application, "");
        let results = [item("first"), item("second"), item("third")];
        assert_eq!(selection_after_refresh(&results, 1, Some("removed")), 1);
        assert_eq!(
            selection_after_refresh(&results, 8, Some("outside-limit")),
            2
        );
        assert_eq!(
            selection_after_refresh(&results[..1], 8, Some("removed")),
            0
        );
    }

    #[test]
    fn empty_refresh_resets_selection_without_underflow() {
        assert_eq!(selection_after_refresh(&[], 9, Some("removed")), 0);
        assert_eq!(selection_after_refresh(&[], usize::MAX, None), 0);
    }

    #[test]
    fn glyph_origin_centers_visible_contours_instead_of_character_advance() {
        let bounds = RECT {
            left: 36,
            top: 185,
            right: 67,
            bottom: 216,
        };
        let mut metrics = GLYPHMETRICS {
            gmBlackBoxX: 13,
            gmBlackBoxY: 17,
            gmptGlyphOrigin: POINT { x: -3, y: 19 },
            gmCellIncX: 50,
            gmCellIncY: 0,
        };
        assert_eq!(centered_glyph_origin(&bounds, &metrics), Some((48, 211)));
        metrics.gmCellIncX = -100;
        metrics.gmCellIncY = 90;
        assert_eq!(centered_glyph_origin(&bounds, &metrics), Some((48, 211)));

        for side in [31, 38, 46, 62] {
            let tile = RECT {
                left: 8,
                top: 11,
                right: 8 + side,
                bottom: 11 + side,
            };
            for (width, height, origin_x, origin_y) in
                [(8, 4, 2, 7), (13, 17, -3, 19), (17, 22, 5, 24)]
            {
                metrics.gmBlackBoxX = width;
                metrics.gmBlackBoxY = height;
                metrics.gmptGlyphOrigin = POINT {
                    x: origin_x,
                    y: origin_y,
                };
                let (x, baseline) = centered_glyph_origin(&tile, &metrics).unwrap();
                let ink_left = x + origin_x;
                let ink_top = baseline - origin_y;
                assert!((2 * ink_left + width as i32 - tile.left - tile.right).abs() <= 1);
                assert!((2 * ink_top + height as i32 - tile.top - tile.bottom).abs() <= 1);
            }
        }
    }

    #[test]
    fn glyph_origin_rejects_empty_contours_and_invalid_tiles() {
        let mut metrics: GLYPHMETRICS = unsafe { zeroed() };
        metrics.gmBlackBoxX = 13;
        metrics.gmBlackBoxY = 17;
        for (width, height) in [(0, 31), (31, 0), (-1, 31), (31, -1)] {
            let bounds = RECT {
                left: 8,
                top: 8,
                right: 8 + width,
                bottom: 8 + height,
            };
            assert_eq!(centered_glyph_origin(&bounds, &metrics), None);
        }
        let bounds = RECT {
            left: 8,
            top: 8,
            right: 39,
            bottom: 39,
        };
        for (width, height) in [(0, 17), (13, 0), (0, 0)] {
            metrics.gmBlackBoxX = width;
            metrics.gmBlackBoxY = height;
            assert_eq!(centered_glyph_origin(&bounds, &metrics), None);
        }
    }

    #[test]
    fn home_glyph_pixels_are_centered_at_common_windows_dpis() {
        // This target is entirely offscreen: no desktop input or capture.
        struct Canvas {
            dc: HDC,
            bitmap: HBITMAP,
            old_bitmap: HGDIOBJ,
            font: HFONT,
            old_font: HGDIOBJ,
            pixels: *mut u32,
            side: usize,
        }
        impl Drop for Canvas {
            fn drop(&mut self) {
                unsafe {
                    if !self.old_font.is_null() {
                        SelectObject(self.dc, self.old_font);
                    }
                    if !self.font.is_null() {
                        DeleteObject(self.font);
                    }
                    if !self.old_bitmap.is_null() {
                        SelectObject(self.dc, self.old_bitmap);
                    }
                    if !self.bitmap.is_null() {
                        DeleteObject(self.bitmap);
                    }
                    if !self.dc.is_null() {
                        DeleteDC(self.dc);
                    }
                }
            }
        }
        impl Canvas {
            unsafe fn new(side: i32) -> Self {
                let mut canvas = Self {
                    dc: CreateCompatibleDC(null_mut()),
                    bitmap: null_mut(),
                    old_bitmap: null_mut(),
                    font: null_mut(),
                    old_font: null_mut(),
                    pixels: null_mut(),
                    side: side as usize,
                };
                assert!(!canvas.dc.is_null());
                let mut info: BITMAPINFO = zeroed();
                info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
                info.bmiHeader.biWidth = side;
                info.bmiHeader.biHeight = -side;
                info.bmiHeader.biPlanes = 1;
                info.bmiHeader.biBitCount = 32;
                info.bmiHeader.biCompression = BI_RGB;
                let mut pixels = null_mut();
                canvas.bitmap =
                    CreateDIBSection(canvas.dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0);
                assert!(!canvas.bitmap.is_null() && !pixels.is_null());
                canvas.pixels = pixels.cast();
                canvas.old_bitmap = SelectObject(canvas.dc, canvas.bitmap);
                assert!(!canvas.old_bitmap.is_null());
                SetBkMode(canvas.dc, TRANSPARENT as i32);
                SetTextColor(canvas.dc, color(255, 255, 255));
                canvas
            }

            unsafe fn set_font(&mut self, family: &str, size: i32, dpi: i32) {
                if !self.font.is_null() {
                    SelectObject(self.dc, self.old_font);
                    DeleteObject(self.font);
                    self.font = null_mut();
                }
                self.font = CreateFontW(
                    -size * dpi / 96,
                    0,
                    0,
                    0,
                    400,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    CLEARTYPE_QUALITY as u32,
                    DEFAULT_PITCH as u32,
                    wide(family).as_ptr(),
                );
                assert!(!self.font.is_null());
                self.old_font = SelectObject(self.dc, self.font);
                assert!(!self.old_font.is_null());
            }
        }

        for dpi in [96, 120, 144, 192] {
            let side = 31 * dpi / 96;
            let bounds = RECT {
                left: 8,
                top: 8,
                right: 8 + side,
                bottom: 8 + side,
            };
            unsafe {
                let mut canvas = Canvas::new(side + 16);
                for (glyph, icon_font) in [
                    ('▦', false),
                    ('\u{E896}', true),
                    ('\u{E74D}', true),
                    ('▱', false),
                    ('=', false),
                    ('\u{E916}', true),
                    ('⌕', false),
                    ('›', false),
                    ('▤', false),
                ] {
                    canvas.set_font(
                        if icon_font {
                            "Segoe MDL2 Assets"
                        } else {
                            "Segoe UI Symbol"
                        },
                        if icon_font { 18 } else { 20 },
                        dpi,
                    );
                    let pixels =
                        std::slice::from_raw_parts_mut(canvas.pixels, canvas.side * canvas.side);
                    pixels.fill(0);
                    let alignment = TA_RIGHT | TA_TOP;
                    SetTextAlign(canvas.dc, alignment);
                    assert!(
                        draw_centered_glyph(canvas.dc, glyph as u16, &bounds),
                        "glyph {glyph:?} at {dpi} DPI"
                    );
                    assert_eq!(GetTextAlign(canvas.dc), alignment);
                    assert_ne!(GdiFlush(), 0);
                    let mut ink: Option<(i32, i32, i32, i32)> = None;
                    for (index, &pixel) in pixels.iter().enumerate() {
                        if pixel & 0x00ff_ffff == 0 {
                            continue;
                        }
                        let x = (index % canvas.side) as i32;
                        let y = (index / canvas.side) as i32;
                        ink = Some(match ink {
                            Some((left, top, right, bottom)) => {
                                (left.min(x), top.min(y), right.max(x + 1), bottom.max(y + 1))
                            }
                            None => (x, y, x + 1, y + 1),
                        });
                    }
                    let (left, top, right, bottom) =
                        ink.unwrap_or_else(|| panic!("missing glyph {glyph:?} at {dpi} DPI"));
                    assert!(left >= bounds.left && top >= bounds.top);
                    assert!(right <= bounds.right && bottom <= bounds.bottom);
                    assert!(
                        (left + right - bounds.left - bounds.right).abs() <= 2
                            && (top + bottom - bounds.top - bounds.bottom).abs() <= 2,
                        "glyph {glyph:?} at {dpi} DPI has ink {ink:?} in a {side}px tile"
                    );
                }
            }
        }
    }

    #[test]
    fn fixture_restores_search_data_after_deserialization() {
        let item = Item::new(
            "fixture:app",
            "Éditeur",
            "Application fictive",
            "",
            Kind::Application,
            "fixture://editor",
        );
        let items = parse_smoke_fixture(&serde_json::to_vec(&vec![item]).unwrap()).unwrap();
        let found = core::rank(&items, "editeur", 1, &HashMap::new());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "fixture:app");
    }

    #[test]
    fn fixture_rejects_commands_and_launch_arguments() {
        for kind in [
            Kind::Install,
            Kind::Uninstall,
            Kind::Command,
            Kind::System,
            Kind::Navigation,
        ] {
            let item = Item::new("fixture", "Test", "", "", kind, "fixture://test");
            assert!(parse_smoke_fixture(&serde_json::to_vec(&vec![item]).unwrap()).is_err());
        }
        let mut item = Item::new(
            "fixture",
            "Test",
            "",
            "",
            Kind::Application,
            "fixture://test",
        );
        item.arguments = "--unexpected".into();
        assert!(parse_smoke_fixture(&serde_json::to_vec(&vec![item]).unwrap()).is_err());
    }

    #[test]
    fn fixture_input_is_bounded_and_ids_are_unique() {
        assert!(parse_smoke_fixture(&vec![b' '; 1024 * 1024 + 1]).is_err());
        assert!(parse_smoke_fixture(b"not json").is_err());
        let item = Item::new(
            "fixture",
            "Test",
            "",
            "",
            Kind::Application,
            "fixture://test",
        );
        assert!(parse_smoke_fixture(
            &serde_json::to_vec(&vec![item.clone(), item.clone()]).unwrap()
        )
        .is_err());
        let mut nul = item.clone();
        nul.target.push('\0');
        assert!(parse_smoke_fixture(&serde_json::to_vec(&vec![nul]).unwrap()).is_err());
        let too_many: Vec<_> = (0..257)
            .map(|index| {
                Item::new(
                    index.to_string(),
                    "Test",
                    "",
                    "",
                    Kind::File,
                    "fixture://test",
                )
            })
            .collect();
        assert!(parse_smoke_fixture(&serde_json::to_vec(&too_many).unwrap()).is_err());
    }

    #[test]
    fn fixture_cli_requires_explicit_smoke_mode_and_path() {
        assert!(
            load_smoke_fixture(&["--smoke-fixture".into(), "never-open-this.json".into()]).is_err()
        );
        assert!(load_smoke_fixture(&["--smoke-test".into(), "--smoke-fixture".into()]).is_err());
        assert!(load_smoke_fixture(&["--smoke-test".into()])
            .unwrap()
            .is_none());
    }

    #[test]
    fn fixture_cannot_activate_software_files_clipboard_or_system_actions() {
        for kind in [
            Kind::Application,
            Kind::Install,
            Kind::Uninstall,
            Kind::File,
            Kind::Folder,
            Kind::Command,
            Kind::Web,
            Kind::Calculator,
            Kind::Clipboard,
            Kind::Symbol,
            Kind::Window,
        ] {
            assert!(!fixture_action_allowed(&Item::new(
                "",
                "",
                "",
                "",
                kind,
                "fixture://test"
            )));
        }
        for target in ["settings", "reset-usage", "lock", "quit"] {
            assert!(!fixture_action_allowed(&Item::new(
                "",
                "",
                "",
                "",
                Kind::System,
                target
            )));
        }
        assert!(fixture_action_allowed(&Item::new(
            "",
            "",
            "",
            "",
            Kind::System,
            "about"
        )));
        assert!(fixture_action_allowed(&Item::new(
            "",
            "",
            "",
            "",
            Kind::Navigation,
            "@ "
        )));
        assert!(fixture_action_allowed(&Item::new(
            "",
            "",
            "",
            "",
            Kind::Timer,
            "start:1s"
        )));
    }
    fn test_app(id: &str) -> Item {
        Item::new(id, id, "", "", Kind::Application, format!("{id}.exe"))
    }

    #[test]
    fn repeated_app_launches_are_deduplicated_during_shell_and_feedback_delay() {
        let now = Instant::now();
        let item = test_app("slow");
        let mut requests = LaunchRequests::default();
        assert_eq!(requests.start(&item, now), Ok(LaunchStart::Started(1)));
        assert_eq!(
            requests.start(&item, now + Duration::from_secs(1)),
            Ok(LaunchStart::Duplicate(1))
        );
        requests.finish(1, true).unwrap();
        assert_eq!(
            requests.start(&item, now + Duration::from_secs(2)),
            Ok(LaunchStart::Duplicate(1))
        );
        assert_eq!(
            requests.start(&item, now + Duration::from_millis(3999)),
            Ok(LaunchStart::Duplicate(1))
        );
        assert_eq!(
            requests.start(&item, now + Duration::from_millis(5999)),
            Ok(LaunchStart::Started(2))
        );
    }

    #[test]
    fn removal_markers_survive_stale_snapshots_and_allow_later_reinstallation() {
        let target = "id:Mozilla.Firefox".to_string();
        let mut removals = Installations::default();
        let token = removals.start(&target).unwrap();
        removals.finish(token, true).unwrap();
        assert!(removals.finish(token, true).is_none());
        let mut refresh_after = HashMap::from([(target.clone(), 2)]);
        reconcile_removals(&mut removals.completed, &mut refresh_after, 1, &[]);
        assert!(removals.completed.contains(&target));
        let still_present = Item::new("remove", "Firefox", "", "", Kind::Uninstall, &target);
        reconcile_removals(
            &mut removals.completed,
            &mut refresh_after,
            2,
            &[still_present],
        );
        assert!(removals.completed.contains(&target));
        reconcile_removals(&mut removals.completed, &mut refresh_after, 2, &[]);
        assert!(removals.completed.is_empty());
        assert!(refresh_after.is_empty());
        assert!(removals.start(&target).is_some());
    }

    #[test]
    fn successful_install_clears_an_unobserved_removal_for_the_same_product() {
        let mut removals = Installations::default();
        removals.completed.insert("id:Mozilla.Firefox".into());
        removals.completed.insert("id:VideoLAN.VLC".into());
        let mut refresh_after = HashMap::from([
            ("id:Mozilla.Firefox".into(), 2),
            ("id:VideoLAN.VLC".into(), 2),
        ]);
        clear_removal_for_install(&mut removals, &mut refresh_after, "Mozilla.Firefox");
        assert!(!removals.completed.contains("id:Mozilla.Firefox"));
        assert!(!refresh_after.contains_key("id:Mozilla.Firefox"));
        assert!(removals.completed.contains("id:VideoLAN.VLC"));
    }

    #[test]
    fn failed_app_launch_can_retry_and_old_completion_cannot_finish_new_attempt() {
        let now = Instant::now();
        let item = test_app("retry");
        let other = test_app("other");
        let mut requests = LaunchRequests::default();
        assert_eq!(requests.start(&item, now), Ok(LaunchStart::Started(1)));
        assert_eq!(requests.start(&other, now), Ok(LaunchStart::Started(2)));
        requests.finish(1, false).unwrap();
        assert_eq!(requests.start(&item, now), Ok(LaunchStart::Started(3)));
        assert!(requests.finish(1, true).is_none());
        assert!(requests.pending.contains_key(&3));
        assert!(requests.pending.contains_key(&2));
        assert_eq!(
            requests.start(&other, now + Duration::from_secs(40)),
            Ok(LaunchStart::Duplicate(2))
        );
    }

    #[test]
    fn concurrent_launches_are_bounded_and_failure_releases_a_slot() {
        let now = Instant::now();
        let mut requests = LaunchRequests::default();
        for ordinal in 0..16 {
            assert!(matches!(
                requests.start(&test_app(&ordinal.to_string()), now),
                Ok(LaunchStart::Started(_))
            ));
        }
        assert!(requests.start(&test_app("overflow"), now).is_err());
        requests.finish(1, false).unwrap();
        assert!(matches!(
            requests.start(&test_app("overflow"), now),
            Ok(LaunchStart::Started(_))
        ));
    }

    #[test]
    fn installed_software_can_be_offered_again_after_a_later_uninstall() {
        let mut installs = Installations::default();
        let request = installs.start("Mozilla.Firefox").unwrap();
        installs.finish(request, true).unwrap();
        let mut absent = packages::missing_items(&[], &[], &HashSet::new());
        installs.refresh_missing(&mut absent);
        assert!(!absent.iter().any(|item| item.target == "Mozilla.Firefox"));
        assert!(installs.completed.contains("Mozilla.Firefox"));
        let mut observed = packages::missing_items(
            &[],
            &["Mozilla Firefox 150.0 (x64 fr)".into()],
            &HashSet::new(),
        );
        installs.refresh_missing(&mut observed);
        assert!(!installs.completed.contains("Mozilla.Firefox"));
        let mut removed = packages::missing_items(&[], &[], &HashSet::new());
        installs.refresh_missing(&mut removed);
        assert!(removed.iter().any(|item| item.target == "Mozilla.Firefox"));
        assert!(installs.start("Mozilla.Firefox").is_some());
    }

    #[test]
    fn only_fresh_conclusive_absence_can_override_an_install_success() {
        let mut installs = Installations::default();
        let request = installs.start("Google.Chrome").unwrap();
        installs.finish(request, true).unwrap();
        let absent = HashSet::from(["Google.Chrome".to_string()]);
        let missing = packages::missing_items_with_absent(
            &[],
            &["Google Chrome".into()],
            &HashSet::new(),
            &absent,
        );
        let mut stale = missing.clone();
        installs.refresh_missing_with_absent(&mut stale, &absent, 3, 4);
        assert!(installs.completed.contains("Google.Chrome"));
        assert!(!stale.iter().any(|item| item.target == "Google.Chrome"));
        let mut fresh = missing;
        installs.refresh_missing_with_absent(&mut fresh, &absent, 4, 4);
        assert!(fresh.iter().any(|item| item.target == "Google.Chrome"));
        assert!(!installs.completed.contains("Google.Chrome"));
        assert!(installs.start("Google.Chrome").is_some());
    }

    #[test]
    fn installation_requests_ignore_duplicates_and_late_replies_after_retry() {
        let mut installs = Installations::default();
        let first = installs.start("Mozilla.Firefox").unwrap();
        assert!(installs.start("Mozilla.Firefox").is_none());
        assert!(installs.finish(999, true).is_none());
        assert_eq!(
            installs.finish(first, false).as_deref(),
            Some("Mozilla.Firefox")
        );
        let retry = installs.start("Mozilla.Firefox").unwrap();
        assert_ne!(first, retry);
        assert!(installs.finish(first, false).is_none());
        assert!(installs.is_pending("Mozilla.Firefox"));
        assert_eq!(
            installs.finish(retry, true).as_deref(),
            Some("Mozilla.Firefox")
        );
        assert!(installs.finish(retry, true).is_none());
        assert!(installs.completed.contains("Mozilla.Firefox"));
        assert!(installs.start("Mozilla.Firefox").is_none());
    }

    #[test]
    fn index_search_defaults_to_applications_and_files_require_prefix() {
        let index = vec![
            Item::new(
                "app",
                "Microsoft Edge",
                "Application Windows",
                "",
                Kind::Application,
                "edge.exe",
            ),
            Item::new(
                "file",
                "Edge",
                "C:\\Documents\\Edge",
                "",
                Kind::File,
                "file",
            ),
            Item::new(
                "folder",
                "EdgeDrop",
                "C:\\Documents\\EdgeDrop",
                "",
                Kind::Folder,
                "folder",
            ),
            Item::new(
                "notes",
                "notes.txt",
                "C:\\Documents\\EdgeDrop\\notes.txt",
                "",
                Kind::File,
                "notes",
            ),
        ];
        let weights = HashMap::from([("file".into(), u32::MAX)]);
        for query in ["edge", "  EDGE  ", "@ edge"] {
            let hits = search_catalog(&index, query, 1, &weights);
            assert_eq!(hits.len(), 1, "query: {query}");
            assert_eq!(hits[0].id, "app", "query: {query}");
        }
        assert!(search_catalog(&index, "edgedrop", 9, &weights).is_empty());
        for query in ["/ edge", " / edge ", "/", "/ edgedrop"] {
            let hits = search_catalog(&index, query, 9, &weights);
            assert!(!hits.is_empty(), "query: {query}");
            assert!(hits
                .iter()
                .all(|i| matches!(i.kind, Kind::File | Kind::Folder)));
            assert!(hits.iter().any(|i| i.id == "folder"));
            assert!(hits.iter().any(|i| i.id == "notes"));
        }
        let mut restored: Vec<Item> =
            serde_json::from_str(&serde_json::to_string(&index).unwrap()).unwrap();
        restored.iter_mut().for_each(Item::prepare);
        assert_eq!(search_catalog(&restored, "edge", 1, &weights)[0].id, "app");
    }

    #[test]
    fn tabs_match_default_apps_and_trimmed_prefixes() {
        for (query, expected) in [
            ("", "Accueil"),
            ("   ", "Accueil"),
            ("edge", "Applications"),
            (" @ edge", "Applications"),
            (" / edge", "Fichiers"),
            (" ? edge", "Web"),
            (" ;", "Modules  ;"),
            ("= 1+1", ""),
        ] {
            assert_eq!(active_tab(query), expected, "query: {query}");
        }
    }

    thread_local! { static EDIT_CHANGES: Cell<usize> = const { Cell::new(0) }; }

    unsafe extern "system" fn edit_test_parent(
        hwnd: HWND,
        msg: u32,
        wp: usize,
        lp: isize,
    ) -> isize {
        if msg == WM_COMMAND && (wp >> 16) & 0xffff == EN_CHANGE as usize {
            EDIT_CHANGES.with(|count| count.set(count.get() + 1));
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }

    #[test]
    fn native_ctrl_backspace_deletes_words_and_preserves_editing() {
        let _ui_test_guard = native::UI_TEST_LOCK.lock().unwrap();
        unsafe {
            let mut original_keys = [0u8; 256];
            assert_ne!(GetKeyboardState(original_keys.as_mut_ptr()), 0);
            struct Controls {
                parent: HWND,
                keys: [u8; 256],
                class: Vec<u16>,
                instance: HINSTANCE,
            }
            impl Drop for Controls {
                fn drop(&mut self) {
                    unsafe {
                        SetKeyboardState(self.keys.as_ptr());
                        DestroyWindow(self.parent);
                        UnregisterClassW(self.class.as_ptr(), self.instance);
                    }
                }
            }
            let instance = GetModuleHandleW(null());
            let class = wide("SuperMenuEditRegression");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(edit_test_parent),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..zeroed()
            };
            assert_ne!(RegisterClassW(&wc), 0);
            let parent = CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                WS_OVERLAPPED,
                0,
                0,
                400,
                100,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            assert!(!parent.is_null());
            let _controls = Controls {
                parent,
                keys: original_keys,
                class,
                instance,
            };
            let edit = CreateWindowExW(
                0,
                wide("EDIT").as_ptr(),
                null(),
                WS_CHILD | ES_AUTOHSCROLL as u32,
                0,
                0,
                300,
                30,
                parent,
                EDIT_ID as _,
                instance,
                null(),
            );
            assert!(!edit.is_null());
            assert_ne!(SetWindowSubclass(edit, Some(edit_proc), 1, 0), 0);
            SetFocus(edit);
            for (input, start, end, expected) in [
                ("hello world", 11, 11, "hello "),
                ("hello world   ", 14, 14, "hello "),
                ("bonjour café", 12, 12, "bonjour "),
                ("bonjour 🦀", 10, 10, "bonjour "),
                ("one three tail", 6, 6, "one ree tail"),
                ("hello world", 6, 11, "hello "),
                ("hello world", 11, 6, "hello "),
                ("hello", 0, 0, "hello"),
                ("", 0, 0, ""),
            ] {
                SetWindowTextW(edit, wide(input).as_ptr());
                SendMessageW(edit, EM_SETSEL, start, end);
                let mut keys = [0u8; 256];
                keys[VK_CONTROL as usize] = 0x80;
                assert_ne!(SetKeyboardState(keys.as_ptr()), 0);
                let before = EDIT_CHANGES.with(Cell::get);
                SendMessageW(edit, WM_KEYDOWN, VK_BACK as usize, 0);
                // The translated DEL character may arrive after Ctrl is released.
                assert_ne!(SetKeyboardState([0u8; 256].as_ptr()), 0);
                SendMessageW(edit, WM_CHAR, 0x7f, 0);
                assert_eq!(control_text(edit), expected, "input: {input:?}");
                if expected != input {
                    assert!(EDIT_CHANGES.with(Cell::get) > before);
                    assert_ne!(SendMessageW(edit, EM_UNDO, 0, 0), 0);
                    assert_eq!(control_text(edit), input, "undo: {input:?}");
                } else {
                    assert_eq!(EDIT_CHANGES.with(Cell::get), before);
                }
            }
            SetWindowTextW(edit, wide("hello").as_ptr());
            SendMessageW(edit, EM_SETSEL, 5, 5);
            SendMessageW(edit, WM_CHAR, VK_BACK as usize, 0);
            assert_eq!(control_text(edit), "hell");
        }
    }

    #[test]
    fn web_query_encoding() {
        assert_eq!(encode("a&b é"), "a%26b%20%C3%A9");
    }
    #[test]
    fn home_providers() {
        let menu = navigation();
        assert_eq!(menu.len(), 13);
        assert!(menu.iter().all(|i| i.kind == Kind::Navigation));
        let about = core::rank(&menu, "à propos", 1, &HashMap::new());
        assert_eq!(about.len(), 1);
        assert_eq!(about[0].target, "about");
        assert!(system_items().iter().any(|item| item.target == "about"));
    }

    #[test]
    fn timer_words_are_explicit_and_preserve_application_and_provider_search() {
        assert_eq!(timer_query(" Minuteur 5m Thé "), Some("5m Thé"));
        assert_eq!(timer_query("timer"), Some(""));
        assert_eq!(timer_query("MINUTEURS"), Some(""));
        for query in [
            "TimerTool",
            "minuteur.exe",
            "5m",
            "@ timer",
            "/ minuteur.txt",
            "+ timer",
        ] {
            assert_eq!(timer_query(query), None, "{query}");
        }
        assert_eq!(active_tab("minuteur 5m"), "Modules  ;");
    }

    #[test]
    fn timer_preview_never_creates_a_countdown_and_invalid_input_never_launches() {
        let book = timer::TimerBook::new();
        let now = Instant::now();
        let rows = timer_items(&book, "5m Thé", None, now);
        assert_eq!(rows[0].kind, Kind::Timer);
        assert_eq!(rows[0].target, "start:5m Thé");
        assert!(rows[0].title.contains("05:00"));
        assert!(book.snapshots(now).is_empty());
        for query in ["0", "-5m", "25h", "1:99", "5ms"] {
            let rows = timer_items(&book, query, None, now);
            assert_eq!(rows[0].kind, Kind::Timer);
            assert_eq!(rows[0].target, "help");
        }
    }

    #[test]
    fn timer_rows_track_pause_resume_and_finished_actions() {
        let now = Instant::now();
        let mut book = timer::TimerBook::new();
        let id = book.start(timer::parse("5s Thé").unwrap(), now).unwrap();
        let rows = timer_items(&book, "", Some(id), now);
        assert!(rows.iter().any(|row| row.target == format!("pause:{id}")));
        book.pause(id, now + Duration::from_secs(2)).unwrap();
        let rows = timer_items(&book, "", Some(id), now + Duration::from_secs(100));
        assert!(rows[0].title.contains("00:03"));
        assert!(rows.iter().any(|row| row.target == format!("resume:{id}")));
        book.resume(id, now + Duration::from_secs(100)).unwrap();
        book.update(now + Duration::from_secs(104));
        let rows = timer_items(&book, "", Some(id), now + Duration::from_secs(104));
        assert!(rows.iter().any(|row| row.target == format!("ack:{id}")));
        assert!(!rows.iter().any(|row| row.target == format!("cancel:{id}")));
    }
}
