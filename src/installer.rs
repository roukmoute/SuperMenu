//! A separate instance of this executable owns WinGet and its visible console.
//! Package operations and their validation run outside the launcher's UI thread.

use crate::{cleanup::CleanupPlan, config, native::wide, packages};
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::ptr::{null, null_mut};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_CANCELLED, GENERIC_READ, GENERIC_WRITE, HANDLE, HWND, INVALID_HANDLE_VALUE,
    WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    AllocConsole, GetConsoleMode, GetConsoleWindow, SetConsoleCP, SetConsoleMode,
    SetConsoleOutputCP, SetConsoleTitleW, SetStdHandle, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, WaitForSingleObject, DETACHED_PROCESS, INFINITE,
};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowThreadProcessId, IsWindow, MessageBoxW, PostMessageW, MB_ICONERROR,
    MB_OK, SW_SHOWNORMAL, WM_APP,
};

pub const INSTALL_DONE: u32 = WM_APP + 5;
pub const REMOVE_DONE: u32 = WM_APP + 7;

/// Both launch paths expose the same worker-only wait contract. Only a removal
/// of an eligible machine-wide registry entry requests administrative consent.
pub enum RemovalProcess {
    Child(Child),
    ElevatedProcess(ElevatedProcess),
}

impl RemovalProcess {
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        match self {
            Self::Child(child) => child.wait(),
            Self::ElevatedProcess(process) => process.wait(),
        }
    }
}

pub struct ElevatedProcess {
    // An owned process handle, transferable to the waiting worker without
    // transferring any window pointer or introducing an unsafe Send impl.
    handle: usize,
}

impl ElevatedProcess {
    fn wait(&mut self) -> io::Result<ExitStatus> {
        unsafe {
            match WaitForSingleObject(self.handle as HANDLE, INFINITE) {
                WAIT_OBJECT_0 => {}
                WAIT_FAILED => return Err(io::Error::last_os_error()),
                result => {
                    return Err(io::Error::other(crate::i18n::fmt(
                        "État d'attente inattendu du helper : {result}.",
                        &[("result", result.to_string())],
                    )))
                }
            }
            let mut code = 0;
            if GetExitCodeProcess(self.handle as HANDLE, &mut code) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(ExitStatus::from_raw(code))
        }
    }
}

impl Drop for ElevatedProcess {
    fn drop(&mut self) {
        if self.handle != 0 {
            unsafe { CloseHandle(self.handle as HANDLE) };
        }
    }
}

// Official WinGet HRESULTs, compared as u32 because Windows exit codes are unsigned.
// https://github.com/microsoft/winget-cli/blob/master/doc/windows/package-manager/winget/returnCodes.md
const NO_APPLICATIONS_FOUND: u32 = 0x8A15_0014;
const UPDATE_NOT_APPLICABLE: u32 = 0x8A15_002B;
const PACKAGE_ALREADY_INSTALLED: u32 = 0x8A15_0061;

struct Request {
    id: &'static str,
    name: String,
    remove: Option<RemoveTarget>,
    parent: HWND,
    parent_pid: u32,
    token: usize,
    check_only: bool,
    no_wait: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum RemoveTarget {
    Id(&'static str),
    Name(String),
}

impl RemoveTarget {
    fn value(&self) -> &str {
        match self {
            Self::Id(id) => id,
            Self::Name(name) => name,
        }
    }

    fn canonical(&self) -> String {
        match self {
            Self::Id(id) => format!("id:{id}"),
            Self::Name(name) => format!("name:{name}"),
        }
    }
}

impl Request {
    fn notify(&self, result: i32) {
        if self.remove.is_some() {
            notify_remove(self.parent, self.parent_pid, self.token, result);
        } else {
            notify(self.parent, self.parent_pid, self.token, result);
        }
    }
}

/// Runs only the private package-helper modes. Normal GUI arguments return None.
pub fn run_from_args(args: &[String]) -> Option<i32> {
    if args.iter().skip(1).any(|arg| arg == "--inspect-cleanup") {
        return Some(inspect_cleanup(args));
    }
    if !args
        .iter()
        .skip(1)
        .any(|arg| matches!(arg.as_str(), "--install-package" | "--uninstall-package"))
    {
        return None;
    }
    // A software-management helper is a separate process. Reuse the language
    // selected by the launcher, without downloading on this execution path.
    #[cfg(not(test))]
    {
        let settings = config::load();
        let language = if settings.language.is_empty() {
            "fr"
        } else {
            &settings.language
        };
        let _ = crate::i18n::init_cached(language, &config::data_dir().join("languages"));
    }
    let request = parse_request(args);
    let heading = if request
        .as_ref()
        .is_ok_and(|request| request.remove.is_some())
    {
        crate::i18n::tr("Super Menu · Désinstallation")
    } else {
        crate::i18n::tr("Super Menu · Installation")
    };
    let console = match Console::open(&heading) {
        Ok(console) => console,
        Err(error) => {
            if let Ok(request) = &request {
                request.notify(1);
            }
            unsafe {
                MessageBoxW(
                    null_mut(),
                    wide(&error).as_ptr(),
                    wide(&heading).as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
            return Some(1);
        }
    };
    let mut wait = true;
    let mut close_after_success = false;
    let result = match request {
        Ok(request) => {
            // One initial confirmation authorizes the complete removal. Successful
            // unattended removals do not require another Enter just to close a log.
            wait = !request.no_wait;
            println!(
                "{}\n",
                crate::i18n::fmt(
                    "{heading} de {name}",
                    &[("heading", heading.clone()), ("name", request.name.clone())]
                )
            );
            let outcome = if valid_parent(request.parent, request.parent_pid) {
                let validation = match &request.remove {
                    Some(target) => {
                        let (all_names, removable_names) = packages::installed_inventory();
                        validate_removal_inventory(target, &all_names, &removable_names)
                    }
                    _ => Ok(()),
                };
                validation
                    .map_err(Failure::local)
                    .and_then(|_| resolve_winget().map_err(Failure::local))
                    .and_then(|exe| {
                        let mut runner = SystemRunner { exe };
                        match &request.remove {
                            Some(target) => {
                                let plan = CleanupPlan::capture(&target.canonical());
                                println!("{}\n", plan.summary());
                                if request.check_only {
                                    // Preview never deletes files or registry values.
                                    let _ = plan.preview_report();
                                }
                                perform_remove_and_cleanup(
                                    &mut runner,
                                    target,
                                    request.check_only,
                                    || finish_cleanup(&plan, &target.canonical(), request.token),
                                )
                            }
                            None => perform_install(&mut runner, request.id, request.check_only),
                        }
                    })
            } else {
                Err(Failure::local(
                    "La fenêtre Super Menu n'est plus disponible.",
                ))
            };
            let result = match outcome {
                Ok(Outcome::Installed) => {
                    println!("{}", crate::i18n::tr("\nInstallation terminée. Le catalogue de Super Menu va être actualisé."));
                    0
                }
                Ok(Outcome::AlreadyInstalled) => {
                    println!("{}", crate::i18n::tr("\nCe logiciel est déjà installé. Aucune mise à jour n'a été effectuée."));
                    0
                }
                Ok(Outcome::Removed) => {
                    println!("{}", crate::i18n::tr("\nDésinstallation terminée. Le catalogue de Super Menu va être actualisé."));
                    wait = false;
                    close_after_success = true;
                    0
                }
                Ok(Outcome::RemoveChecked) => {
                    println!("{}", crate::i18n::tr("\nLe logiciel est installé. Diagnostic uniquement : aucune désinstallation lancée."));
                    0
                }
                Err(error) => {
                    eprintln!(
                        "{}",
                        crate::i18n::fmt("\nÉchec : {error}", &[("error", error.message.clone())])
                    );
                    error.code
                }
            };
            // Notify immediately. Closing the console is independent of the next request.
            request.notify(result);
            result
        }
        Err(error) => {
            eprintln!(
                "{}",
                crate::i18n::fmt(
                    "Requête de gestion de logiciel refusée : {error}",
                    &[("error", error.to_string())]
                )
            );
            1
        }
    };
    if wait {
        println!(
            "{}",
            crate::i18n::tr("\nAppuyez sur Entrée pour fermer cette fenêtre.")
        );
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
        let _ = io::stdin().read_line(&mut String::new());
    } else if close_after_success {
        // Notify the launcher first; closing this result window is independent.
        std::thread::sleep(Duration::from_secs(2));
    }
    drop(console);
    Some(result)
}

/// An explicitly read-only diagnostic. It never invokes WinGet, a remover,
/// a cleanup executor or an elevation request.
fn inspect_cleanup(args: &[String]) -> i32 {
    let parsed = match args {
        [_, flag, target, report_flag, report_path]
            if flag == "--inspect-cleanup" && report_flag == "--report" =>
        {
            remove_target(target).map(|target| (target.canonical(), PathBuf::from(report_path)))
        }
        _ => Err(
            "Diagnostic : --inspect-cleanup <id:Paquet ou name:Nom exact> --report <fichier.json>"
                .into(),
        ),
    };
    let (target, path) = match parsed {
        Ok(parsed) => parsed,
        Err(_) => return 1,
    };
    let plan = CleanupPlan::capture(&target);
    let report = plan.preview_report();
    serde_json::to_vec_pretty(&report)
        .map_err(io::Error::other)
        .and_then(|bytes| std::fs::write(path, bytes))
        .map_or(1, |_| 0)
}

/// Starts the helper; the caller may wait for Child on a worker as a notification fallback.
pub fn spawn(id: &str, parent: HWND, parent_pid: u32, request: usize) -> Result<Child, String> {
    let package = whitelisted(id)?;
    spawn_helper(
        helper_args(package.id, parent as usize, parent_pid, request),
        parent,
        parent_pid,
        request,
    )
}

/// The UI must confirm the removal before calling this. A machine-wide eligible
/// registry entry selects runas; fresh protection validation remains in the helper.
pub fn spawn_remove(
    target: &str,
    parent: HWND,
    parent_pid: u32,
    request: usize,
) -> Result<RemovalProcess, String> {
    let target = remove_target(target)?;
    let canonical = target.canonical();
    let args = remove_helper_args(&canonical, parent as usize, parent_pid, request);
    validate_endpoint(parent, parent_pid, request)?;
    if packages::removal_requires_elevation(&canonical) {
        spawn_elevated_helper(&args, parent).map(RemovalProcess::ElevatedProcess)
    } else {
        spawn_helper(args, parent, parent_pid, request).map(RemovalProcess::Child)
    }
}

fn spawn_helper(
    args: Vec<String>,
    parent: HWND,
    parent_pid: u32,
    request: usize,
) -> Result<Child, String> {
    validate_endpoint(parent, parent_pid, request)?;
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    Command::new(exe)
        .args(args)
        // AllocConsole creates a new visible console even when the debug launcher has one.
        .creation_flags(DETACHED_PROCESS)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            crate::i18n::fmt(
                "Impossible d'ouvrir la console de gestion de logiciels : {error}",
                &[("error", error.to_string())],
            )
        })
}

fn validate_endpoint(parent: HWND, parent_pid: u32, request: usize) -> Result<(), String> {
    if request == 0 || !valid_parent(parent, parent_pid) {
        Err(crate::i18n::tr(
            "La requête n'a pas de fenêtre Super Menu valide.",
        ))
    } else {
        Ok(())
    }
}

fn spawn_elevated_helper(args: &[String], parent: HWND) -> Result<ElevatedProcess, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let file: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let parameters = wide(&helper_parameters(args));
    let verb = wide("runas");
    unsafe {
        let initialized = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED
                | windows::Win32::System::Com::COINIT_DISABLE_OLE1DDE,
        )
        .is_ok();
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        // Security prompts still appear with FLAG_NO_UI; only redundant Shell
        // error dialogs are suppressed. Our helper alone allocates its console.
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
        info.hwnd = parent;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = parameters.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        let success = ShellExecuteExW(&mut info) != 0;
        // Capture GetLastError before balancing COM can change the thread's error.
        let error = (!success).then(io::Error::last_os_error);
        if initialized {
            windows::Win32::System::Com::CoUninitialize();
        }
        if let Some(error) = error {
            if !info.hProcess.is_null() {
                CloseHandle(info.hProcess);
            }
            return Err(elevation_error(&error));
        }
        if info.hProcess.is_null() {
            return Err(crate::i18n::tr(
                "Le helper administrateur n'a pas retourné de processus vérifiable.",
            ));
        }
        Ok(ElevatedProcess {
            handle: info.hProcess as usize,
        })
    }
}

fn elevation_error(error: &io::Error) -> String {
    if error.raw_os_error() == Some(ERROR_CANCELLED as i32) {
        crate::i18n::tr(
            "Autorisation administrateur annulée. Aucune désinstallation n'a été lancée.",
        )
    } else {
        crate::i18n::fmt(
            "Impossible d'ouvrir la console de désinstallation administrateur : {error}",
            &[("error", error.to_string())],
        )
    }
}

fn helper_parameters(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_windows_argument(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_windows_argument(arg: &str) -> String {
    // ShellExecuteEx passes one raw parameter string to the executable. Preserve
    // exact registry names through Windows' quote/backslash argument rules.
    let mut quoted = String::with_capacity(arg.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for character in arg.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        let count = if character == '"' {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        quoted.extend(std::iter::repeat_n('\\', count));
        quoted.push(character);
        backslashes = 0;
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

/// Tokens identify attempts, so an old console cannot complete a newer retry.
pub fn notify(parent: HWND, parent_pid: u32, request: usize, result: i32) -> bool {
    notify_message(parent, parent_pid, request, result, INSTALL_DONE)
}

pub fn notify_remove(parent: HWND, parent_pid: u32, request: usize, result: i32) -> bool {
    notify_message(parent, parent_pid, request, result, REMOVE_DONE)
}

fn notify_message(
    parent: HWND,
    parent_pid: u32,
    request: usize,
    result: i32,
    message: u32,
) -> bool {
    request != 0
        && valid_parent(parent, parent_pid)
        && unsafe { PostMessageW(parent, message, request, result as isize) != 0 }
}

fn valid_parent(parent: HWND, parent_pid: u32) -> bool {
    if parent.is_null() || parent_pid == 0 || unsafe { IsWindow(parent) } == 0 {
        return false;
    }
    let mut actual_pid = 0;
    let mut class = [0u16; 64];
    unsafe {
        GetWindowThreadProcessId(parent, &mut actual_pid);
        let length = GetClassNameW(parent, class.as_mut_ptr(), class.len() as i32);
        actual_pid == parent_pid
            && length > 0
            && class[..length as usize]
                .iter()
                .copied()
                .eq("SuperMenuWindow".encode_utf16())
    }
}

fn whitelisted(id: &str) -> Result<&'static packages::Package, String> {
    packages::find(id)
        .filter(|package| package.id.eq_ignore_ascii_case(id))
        .ok_or_else(|| crate::i18n::tr("Identifiant absent du catalogue d'installation autorisé."))
}

fn helper_args(id: &str, parent: usize, pid: u32, token: usize) -> Vec<String> {
    vec![
        "--install-package".into(),
        id.into(),
        "--notify".into(),
        parent.to_string(),
        "--parent-pid".into(),
        pid.to_string(),
        "--request".into(),
        token.to_string(),
    ]
}

fn remove_helper_args(target: &str, parent: usize, pid: u32, token: usize) -> Vec<String> {
    let mut args = helper_args(target, parent, pid, token);
    args[0] = "--uninstall-package".into();
    args
}

fn remove_target(target: &str) -> Result<RemoveTarget, String> {
    if let Some(id) = target.strip_prefix("id:") {
        return whitelisted(id).map(|package| RemoveTarget::Id(package.id));
    }
    if let Some(name) = target.strip_prefix("name:") {
        if !name.is_empty()
            && name.trim() == name
            && !name.starts_with('-')
            && name.chars().count() <= 2048
            && !name.chars().any(char::is_control)
        {
            return Ok(RemoveTarget::Name(name.into()));
        }
    }
    Err(crate::i18n::tr(
        "Cible de désinstallation invalide : identifiant autorisé ou nom installé exact requis.",
    ))
}

fn validate_removal_inventory(
    target: &RemoveTarget,
    all_names: &[String],
    removable_names: &[String],
) -> Result<(), String> {
    let key = match target {
        RemoveTarget::Id(id) => format!("id:{id}"),
        RemoveTarget::Name(name) => format!("name:{name}"),
    };
    let exact_name = match target {
        RemoveTarget::Name(name) => removable_names.iter().any(|installed| installed == name),
        RemoveTarget::Id(_) => true,
    };
    if !exact_name || !packages::removal_is_allowed(&key, all_names, removable_names) {
        return Err(crate::i18n::tr("Ce logiciel est protégé ou son nom exact n'est plus présent parmi les logiciels désinstallables. Aucune désinstallation lancée."));
    }
    Ok(())
}

fn parse_request(args: &[String]) -> Result<Request, String> {
    let mut id = None;
    let mut removal = None;
    let mut parent = None;
    let mut pid = None;
    let mut token = None;
    let mut check_only = false;
    let mut no_wait = false;
    let mut values = args.iter().skip(1);
    while let Some(flag) = values.next() {
        match flag.as_str() {
            "--check-only" if !check_only => {
                check_only = true;
                continue;
            }
            "--no-wait" if !no_wait => {
                no_wait = true;
                continue;
            }
            _ => {}
        }
        let slot = match flag.as_str() {
            "--install-package" => &mut id,
            "--uninstall-package" => &mut removal,
            "--notify" => &mut parent,
            "--parent-pid" => &mut pid,
            "--request" => &mut token,
            _ => {
                return Err(crate::i18n::fmt(
                    "Option inconnue : {flag}",
                    &[("flag", flag.to_string())],
                ))
            }
        };
        if slot.is_some() {
            return Err(crate::i18n::fmt(
                "Option répétée : {flag}",
                &[("flag", flag.to_string())],
            ));
        }
        *slot = Some(values.next().ok_or_else(|| {
            crate::i18n::fmt("Valeur manquante : {flag}", &[("flag", flag.to_string())])
        })?);
    }
    let (id, name, remove) = match (id, removal) {
        (Some(id), None) => {
            let package = whitelisted(id)?;
            (package.id, package.name.to_string(), None)
        }
        (None, Some(value)) => {
            let target = remove_target(value)?;
            let name = match &target {
                RemoveTarget::Id(id) => whitelisted(id)?.name.to_string(),
                RemoveTarget::Name(name) => name.clone(),
            };
            ("", name, Some(target))
        }
        _ => {
            return Err(crate::i18n::tr(
                "Choisissez exactement un mode : installation ou désinstallation.",
            ))
        }
    };
    let parent = parse_number::<usize>(parent, "Fenêtre")?;
    let pid = parse_number::<u32>(pid, "Processus")?;
    let token = parse_number::<usize>(token, "Requête")?;
    if parent == 0 || pid == 0 || token == 0 {
        return Err(crate::i18n::tr(
            "Fenêtre, processus et requête doivent être non nuls.",
        ));
    }
    if no_wait && !check_only {
        return Err(crate::i18n::tr(
            "--no-wait est réservé au diagnostic --check-only.",
        ));
    }
    Ok(Request {
        id,
        name,
        remove,
        parent: parent as HWND,
        parent_pid: pid,
        token,
        check_only,
        no_wait,
    })
}

fn parse_number<T: std::str::FromStr>(value: Option<&String>, name: &str) -> Result<T, String> {
    let value = value
        .ok_or_else(|| crate::i18n::fmt("{name} manquant.", &[("name", crate::i18n::tr(name))]))?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(crate::i18n::fmt(
            "{name} invalide.",
            &[("name", crate::i18n::tr(name))],
        ));
    }
    value
        .parse()
        .map_err(|_| crate::i18n::fmt("{name} invalide.", &[("name", crate::i18n::tr(name))]))
}

fn resolve_winget() -> Result<PathBuf, String> {
    // Prefer the official per-user App Installer execution alias, never search the CWD.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let alias = PathBuf::from(local).join("Microsoft\\WindowsApps\\winget.exe");
        if alias.is_absolute() && alias.try_exists().unwrap_or(false) {
            return Ok(alias);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path).filter(|path| path.is_absolute()) {
            let exe = directory.join("winget.exe");
            if exe.is_file() {
                return Ok(exe);
            }
        }
    }
    Err(crate::i18n::tr("WinGet est introuvable. Installez ou réparez « App Installer » de Microsoft, puis réessayez. https://aka.ms/getwinget"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    List,
    ListName,
    Install,
    RemoveId,
    RemoveName,
}

fn command_args(operation: Operation, id: &str) -> Vec<&str> {
    let mut args = vec![
        match operation {
            Operation::List | Operation::ListName => "list",
            Operation::Install => "install",
            Operation::RemoveId | Operation::RemoveName => "uninstall",
        },
        if matches!(operation, Operation::ListName | Operation::RemoveName) {
            "--name"
        } else {
            "--id"
        },
        id,
        "--exact",
    ];
    // Registry-named software can have no source association at all. Filtering
    // that inventory to the winget source would hide it or falsely imply absence.
    // Curated IDs retain their explicit source; named targets remain exact and
    // ambiguous results are still refused by WinGet.
    if !matches!(operation, Operation::ListName | Operation::RemoveName) {
        args.extend(["--source", "winget"]);
    }
    args.extend(["--accept-source-agreements", "--disable-interactivity"]);
    if operation == Operation::Install {
        args.extend(["--no-upgrade", "--accept-package-agreements", "--silent"]);
    } else if matches!(operation, Operation::RemoveId | Operation::RemoveName) {
        args.push("--silent");
    }
    args
}

trait Runner {
    fn run(&mut self, operation: Operation, id: &str) -> Result<i32, String>;
}

struct SystemRunner {
    exe: PathBuf,
}

impl Runner for SystemRunner {
    fn run(&mut self, operation: Operation, id: &str) -> Result<i32, String> {
        let args = command_args(operation, id);
        println!("> winget {}\n", args.join(" "));
        let _ = io::stdout().flush();
        Command::new(&self.exe)
            .args(args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|error| {
                crate::i18n::fmt("WinGet ne peut pas être exécuté ({error}). Vérifiez App Installer et l'alias d'exécution des applications dans les paramètres Windows.", &[("error", error.to_string())])
            })?
            .code()
            .ok_or_else(|| crate::i18n::tr("WinGet a été interrompu sans résultat."))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Installed,
    AlreadyInstalled,
    Removed,
    RemoveChecked,
}

#[derive(Debug)]
struct Failure {
    code: i32,
    message: String,
}

impl Failure {
    fn local(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            message: crate::i18n::tr(&message.into()),
        }
    }

    fn winget(code: i32, context: &str) -> Self {
        Self {
            code,
            message: crate::i18n::fmt(
                "{context} (WinGet {code}). Consultez le détail affiché ci-dessus.",
                &[
                    ("context", crate::i18n::tr(context)),
                    ("code", format!("0x{:08X}", code as u32)),
                ],
            ),
        }
    }
}

fn perform_install(
    runner: &mut impl Runner,
    id: &str,
    check_only: bool,
) -> Result<Outcome, Failure> {
    let status = runner.run(Operation::List, id).map_err(Failure::local)?;
    match status as u32 {
        0 => return Ok(Outcome::AlreadyInstalled),
        NO_APPLICATIONS_FOUND => {}
        _ => return Err(Failure::winget(
            status,
            "Impossible de déterminer si le logiciel est déjà installé. Aucune installation lancée",
        )),
    }
    if check_only {
        return Err(Failure::winget(
            status,
            "Le logiciel est absent. Le diagnostic n'installe aucun logiciel",
        ));
    }
    let status = runner.run(Operation::Install, id).map_err(Failure::local)?;
    match status as u32 {
        0 => Ok(Outcome::Installed),
        PACKAGE_ALREADY_INSTALLED => Ok(Outcome::AlreadyInstalled),
        UPDATE_NOT_APPLICABLE => {
            // WinGet can discover a concurrently installed app. Confirm its presence;
            // "no applicable update" alone does not prove a successful installation.
            let verified = runner.run(Operation::List, id).map_err(Failure::local)?;
            if verified == 0 {
                Ok(Outcome::AlreadyInstalled)
            } else {
                Err(Failure::winget(
                    verified,
                    "L'installation n'a pas pu être confirmée",
                ))
            }
        }
        _ => Err(Failure::winget(
            status,
            "L'installation a échoué ou a été annulée",
        )),
    }
}

fn perform_remove(
    runner: &mut impl Runner,
    target: &RemoveTarget,
    check_only: bool,
) -> Result<Outcome, Failure> {
    let (list, remove) = match target {
        RemoveTarget::Id(_) => (Operation::List, Operation::RemoveId),
        RemoveTarget::Name(_) => (Operation::ListName, Operation::RemoveName),
    };
    let status = runner.run(list, target.value()).map_err(Failure::local)?;
    match status as u32 {
        0 => {}
        NO_APPLICATIONS_FOUND => {
            return Err(Failure::winget(
                status,
                "Déjà absent ou non reconnu par WinGet, aucun désinstallateur lancé",
            ))
        }
        _ => {
            return Err(Failure::winget(
                status,
                "État absent, incertain ou ambigu. Aucune désinstallation lancée",
            ))
        }
    }
    if check_only {
        return Ok(Outcome::RemoveChecked);
    }
    // Exact filters, disabled prompts and no --all leave WinGet responsible for
    // rejecting multiple matches instead of selecting an arbitrary application.
    let status = runner.run(remove, target.value()).map_err(Failure::local)?;
    match status as u32 {
        0 => {
            // A successful uninstaller exit alone is not proof that the product
            // disappeared. Never clean program directories while WinGet still
            // reports the exact target installed, or when verification failed.
            let verified = runner.run(list, target.value()).map_err(Failure::local)?;
            match verified as u32 {
                NO_APPLICATIONS_FOUND => Ok(Outcome::Removed),
                0 => Err(Failure::local(
                    "Le logiciel est encore présent après la désinstallation. Aucun nettoyage de résidus n'a été lancé.",
                )),
                _ => Err(Failure::winget(
                    verified,
                    "Impossible de vérifier la disparition du logiciel. Aucun nettoyage de résidus n'a été lancé",
                )),
            }
        }
        NO_APPLICATIONS_FOUND => Err(Failure::winget(
            status,
            "Déjà absent ou non reconnu par WinGet, aucun désinstallateur lancé",
        )),
        _ => Err(Failure::winget(
            status,
            "La désinstallation a échoué, a été annulée ou correspond à plusieurs logiciels",
        )),
    }
}

fn perform_remove_and_cleanup(
    runner: &mut impl Runner,
    target: &RemoveTarget,
    check_only: bool,
    cleanup: impl FnOnce(),
) -> Result<Outcome, Failure> {
    let outcome = perform_remove(runner, target, check_only)?;
    if outcome == Outcome::Removed {
        cleanup();
    }
    Ok(outcome)
}

fn finish_cleanup(plan: &CleanupPlan, target: &str, request: usize) {
    let directory = config::data_dir().join("uninstall-reports");
    let _journal_parents = match prepare_report_directory(&directory) {
        Ok(handles) => handles,
        Err(error) => {
            eprintln!("{}", crate::i18n::fmt("\nLe logiciel est retiré ; nettoyage ignoré car son journal ne peut pas être enregistré : {error}", &[("error", error.to_string())]));
            return;
        }
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = directory.join(format!("{timestamp}-{}-{request}.json", std::process::id()));
    // Reserve a unique report before cleanup. Existing files and links are never
    // overwritten, including in an elevated helper.
    let intent_path = path.with_extension("plan.json");
    let file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&intent_path)
    {
        Ok(file) => file,
        Err(error) => {
            eprintln!("{}", crate::i18n::fmt("\nLe logiciel est retiré ; nettoyage ignoré car son journal ne peut pas être créé : {error}", &[("error", error.to_string())]));
            return;
        }
    };
    let intent = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "phase": "planned",
        "target": target,
        "timestamp_ms": timestamp,
        "cleanup": plan.preview_report(),
    });
    let mut writer = io::BufWriter::new(file);
    if let Err(error) = serde_json::to_writer_pretty(&mut writer, &intent)
        .map_err(io::Error::other)
        .and_then(|_| writer.flush())
        .and_then(|_| writer.get_ref().sync_all())
    {
        eprintln!("{}", crate::i18n::fmt("\nLe logiciel est retiré ; nettoyage ignoré car son plan ne peut pas être sauvegardé : {error}", &[("error", error.to_string())]));
        return;
    }
    drop(writer);
    let report = plan.execute_after_success();
    println!(
        "{}",
        crate::i18n::fmt(
            "\nNettoyage : {removed} résidu(s) retiré(s), {retained} conservé(s).",
            &[
                ("removed", report.removed.len().to_string()),
                ("retained", report.retained.len().to_string())
            ]
        )
    );
    for note in &report.notes {
        println!("  {note}");
    }
    let document = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "phase": "finished",
        "target": target,
        "timestamp_ms": timestamp,
        "cleanup": report,
    });
    let file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) => {
            eprintln!("{}", crate::i18n::fmt("Le bilan ne peut pas être créé : {error}. Le plan initial reste disponible : {path}", &[("error", error.to_string()), ("path", intent_path.display().to_string())]));
            return;
        }
    };
    let mut writer = io::BufWriter::new(file);
    if let Err(error) = serde_json::to_writer_pretty(&mut writer, &document)
        .map_err(io::Error::other)
        .and_then(|_| writer.flush())
        .and_then(|_| writer.get_ref().sync_all())
    {
        eprintln!(
            "{}",
            crate::i18n::fmt(
                "Le journal n'a pas pu être finalisé : {error}",
                &[("error", error.to_string())]
            )
        );
    } else {
        println!(
            "{}",
            crate::i18n::fmt("Rapport : {path}", &[("path", path.display().to_string())])
        );
    }
}

fn prepare_report_directory(path: &std::path::Path) -> io::Result<Vec<std::fs::File>> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };
    if !path.is_absolute() {
        return Err(io::Error::other(crate::i18n::tr(
            "Le dossier du journal n'est pas absolu.",
        )));
    }
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 => {
                return Err(io::Error::other(crate::i18n::tr(
                    "Le dossier du journal traverse un lien ou une jonction.",
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    std::fs::create_dir_all(path)?;
    // Keep the journal directory and its parents from being replaced during an
    // elevated cleanup. Create-new files alone cannot pin a changing directory.
    let mut handles = Vec::new();
    for ancestor in path.ancestors() {
        let handle = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(ancestor)?;
        let metadata = handle.metadata()?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::other(crate::i18n::tr(
                "Le parent du journal est devenu un lien.",
            )));
        }
        handles.push(handle);
    }
    Ok(handles)
}

struct Console {
    input: HANDLE,
    output: HANDLE,
}

impl Console {
    fn open(title: &str) -> Result<Self, String> {
        unsafe {
            if GetConsoleWindow().is_null() && AllocConsole() == 0 {
                return Err(crate::i18n::fmt(
                    "Impossible d'ouvrir la console : {error}",
                    &[("error", io::Error::last_os_error().to_string())],
                ));
            }
            // A GUI executable starts with null standard handles. Explicitly rebind
            // them before using Rust stdio or inheriting them in the WinGet child.
            let input = open_console("CONIN$")?;
            let output = match open_console("CONOUT$") {
                Ok(output) => output,
                Err(error) => {
                    CloseHandle(input);
                    return Err(error);
                }
            };
            let console = Self { input, output };
            if SetStdHandle(STD_INPUT_HANDLE, input) == 0
                || SetStdHandle(STD_OUTPUT_HANDLE, output) == 0
                || SetStdHandle(STD_ERROR_HANDLE, output) == 0
            {
                return Err(crate::i18n::fmt(
                    "Console indisponible : {error}",
                    &[("error", io::Error::last_os_error().to_string())],
                ));
            }
            SetConsoleTitleW(wide(title).as_ptr());
            SetConsoleCP(65001);
            SetConsoleOutputCP(65001);
            let mut mode = 0;
            if GetConsoleMode(output, &mut mode) != 0 {
                SetConsoleMode(output, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
            Ok(console)
        }
    }
}

fn open_console(name: &str) -> Result<HANDLE, String> {
    let handle = unsafe {
        CreateFileW(
            wide(name).as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(crate::i18n::fmt(
            "Console indisponible : {error}",
            &[("error", io::Error::last_os_error().to_string())],
        ))
    } else {
        Ok(handle)
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.input);
            CloseHandle(self.output);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn journal_parents_stay_pinned_until_both_reports_are_written() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "supermenu-journal-fixture-{}-{nonce}",
            std::process::id()
        ));
        let moved = root.with_extension("moved");
        assert!(!root.exists() && !moved.exists());
        let directory = root.join("reports");
        let handles = prepare_report_directory(&directory).unwrap();
        assert!(std::fs::rename(&root, &moved).is_err());
        let plan = directory.join("fixture.plan.json");
        let finished = directory.join("fixture.json");
        std::fs::write(&plan, b"owned fixture plan").unwrap();
        std::fs::write(&finished, b"owned fixture result").unwrap();
        drop(handles);
        std::fs::rename(&root, &moved).unwrap();
        std::fs::remove_file(moved.join("reports/fixture.plan.json")).unwrap();
        std::fs::remove_file(moved.join("reports/fixture.json")).unwrap();
        std::fs::remove_dir(moved.join("reports")).unwrap();
        std::fs::remove_dir(moved).unwrap();
    }

    struct FakeRunner {
        results: VecDeque<Result<i32, String>>,
        calls: Vec<Operation>,
    }

    impl FakeRunner {
        fn codes(codes: &[u32]) -> Self {
            Self {
                results: codes.iter().map(|code| Ok(*code as i32)).collect(),
                calls: Vec::new(),
            }
        }
    }

    impl Runner for FakeRunner {
        fn run(&mut self, operation: Operation, _id: &str) -> Result<i32, String> {
            self.calls.push(operation);
            self.results.pop_front().expect("Unexpected WinGet command")
        }
    }

    fn decoded_parameters(parameters: &str) -> Vec<String> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::UI::Shell::CommandLineToArgvW;
        let command = wide(&format!("supermenu.exe {parameters}"));
        let mut count = 0;
        let arguments = unsafe { CommandLineToArgvW(command.as_ptr(), &mut count) };
        assert!(!arguments.is_null());
        struct Allocation(*mut windows_sys::core::PWSTR);
        impl Drop for Allocation {
            fn drop(&mut self) {
                unsafe { LocalFree(self.0.cast()) };
            }
        }
        let _allocation = Allocation(arguments);
        (0..count as usize)
            .map(|index| unsafe {
                let argument = *arguments.add(index);
                let mut length = 0;
                while *argument.add(length) != 0 {
                    length += 1;
                }
                String::from_utf16(std::slice::from_raw_parts(argument, length)).unwrap()
            })
            .collect()
    }

    #[test]
    fn elevated_parameters_preserve_exact_names_quotes_backslashes_and_tokens() {
        let names = [
            "Google Chrome",
            "Éditeur C++ \"Pro\" 2026",
            "A\\\"B",
            "Trailing\\",
            "Multiple\\\\slashes\\\\",
            "💻 Utility",
        ];
        for name in names {
            let target = format!("name:{name}");
            let args = remove_helper_args(&target, 123, 456, 789);
            let decoded = decoded_parameters(&helper_parameters(&args));
            assert_eq!(&decoded[1..], args);
            let request = parse_request(&decoded).unwrap();
            assert_eq!(request.remove, Some(RemoveTarget::Name(name.into())));
            assert_eq!(request.parent as usize, 123);
            assert_eq!(request.parent_pid, 456);
            assert_eq!(request.token, 789);
            assert!(!request.check_only && !request.no_wait);
        }
        let args = ["", "plain", "with spaces", "\"", "\\", "\t"].map(String::from);
        assert_eq!(&decoded_parameters(&helper_parameters(&args))[1..], args);
    }

    #[test]
    fn elevation_cancellation_is_an_error_and_process_wrapper_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<RemovalProcess>();
        let message = elevation_error(&io::Error::from_raw_os_error(ERROR_CANCELLED as i32));
        assert!(message.contains("annulée"));
        assert!(message.contains("Aucune désinstallation"));
        let other = elevation_error(&io::Error::from_raw_os_error(5));
        assert!(other.contains("Impossible"));
        assert!(!other.contains("annulée"));
    }

    #[test]
    fn owned_process_handle_wait_preserves_exit_code_without_elevation() {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;

        // The owned Windows shell runs only its built-in exit command. Neither
        // this test nor its process invokes runas, WinGet, or any package action.
        let shell = PathBuf::from(std::env::var_os("WINDIR").unwrap())
            .join("System32")
            .join("cmd.exe");
        let mut child = Command::new(shell)
            .args(["/d", "/c", "exit", "23"])
            .creation_flags(DETACHED_PROCESS)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut duplicate = null_mut();
        let success = unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                child.as_raw_handle() as HANDLE,
                GetCurrentProcess(),
                &mut duplicate,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        };
        assert_ne!(success, 0);
        let mut process = RemovalProcess::ElevatedProcess(ElevatedProcess {
            handle: duplicate as usize,
        });
        assert_eq!(process.wait().unwrap().code(), Some(23));
        assert_eq!(process.wait().unwrap().code(), Some(23));
        // Drop closes the duplicate; Child retains and then closes its own handle.
        drop(process);
        assert_eq!(child.wait().unwrap().code(), Some(23));
    }

    #[test]
    fn helper_arguments_round_trip_and_canonicalize_id() {
        let package = &packages::PACKAGES[0];
        let mut args = vec!["supermenu.exe".into()];
        args.extend(helper_args(&package.id.to_ascii_lowercase(), 123, 456, 789));
        let request = parse_request(&args).unwrap();
        assert_eq!(request.id, package.id);
        assert_eq!(request.parent as usize, 123);
        assert_eq!(request.parent_pid, 456);
        assert_eq!(request.token, 789);
        assert!(!request.check_only);
        assert!(!request.no_wait);
    }

    #[test]
    fn helper_rejects_unknown_packages_duplicates_missing_and_invalid_numbers() {
        let mut valid = vec!["supermenu.exe".into()];
        valid.extend(helper_args(packages::PACKAGES[0].id, 123, 456, 789));
        for (position, value) in [
            (2, "Git.Git; calc.exe"),
            (4, "0"),
            (6, "-1"),
            (8, "+2"),
            (8, "184467440737095516160"),
        ] {
            let mut args = valid.clone();
            args[position] = value.into();
            assert!(parse_request(&args).is_err());
        }
        let mut duplicate = valid.clone();
        duplicate.extend(["--request".into(), "999".into()]);
        assert!(parse_request(&duplicate).is_err());
        valid.pop();
        assert!(parse_request(&valid).is_err());
        assert!(parse_request(&["supermenu.exe".into(), "--install-package".into()]).is_err());
    }

    #[test]
    fn commands_are_exact_source_bound_and_never_upgrade() {
        let id = packages::PACKAGES[0].id;
        assert_eq!(
            command_args(Operation::List, id),
            vec![
                "list",
                "--id",
                id,
                "--exact",
                "--source",
                "winget",
                "--accept-source-agreements",
                "--disable-interactivity"
            ]
        );
        let install = command_args(Operation::Install, id);
        assert_eq!(install[0], "install");
        assert_eq!(install[2], id);
        assert!(install.contains(&"--no-upgrade"));
        assert!(install.contains(&"--accept-package-agreements"));
        assert!(!install.contains(&"--force"));
        assert!(install.contains(&"--silent"));
    }

    #[test]
    fn existing_software_is_skipped_and_unknown_state_never_installs() {
        for code in [0, 0x8A15_000F, 0x8A15_0045, 5, 1] {
            let mut runner = FakeRunner::codes(&[code]);
            let outcome = perform_install(&mut runner, "Git.Git", false);
            assert_eq!(outcome.is_ok(), code == 0);
            assert_eq!(runner.calls, vec![Operation::List]);
        }
        let mut runner = FakeRunner::codes(&[]);
        runner.results.push_back(Err("Access denied".into()));
        assert!(perform_install(&mut runner, "Git.Git", false).is_err());
        assert_eq!(runner.calls, vec![Operation::List]);
    }

    #[test]
    fn only_explicit_missing_result_runs_install_and_preserves_failures() {
        for code in [0, PACKAGE_ALREADY_INSTALLED, 0x8A15_0005, 0x8A15_0011, 1603] {
            let mut runner = FakeRunner::codes(&[NO_APPLICATIONS_FOUND, code]);
            let outcome = perform_install(&mut runner, "Git.Git", false);
            assert_eq!(
                outcome.is_ok(),
                code == 0 || code == PACKAGE_ALREADY_INSTALLED
            );
            if let Err(error) = outcome {
                assert_eq!(error.code as u32, code);
            }
            assert_eq!(runner.calls, vec![Operation::List, Operation::Install]);
        }
    }

    #[test]
    fn concurrent_install_is_confirmed_after_no_applicable_update() {
        for verified in [0, NO_APPLICATIONS_FOUND, 0x8A15_0045] {
            let mut runner =
                FakeRunner::codes(&[NO_APPLICATIONS_FOUND, UPDATE_NOT_APPLICABLE, verified]);
            let outcome = perform_install(&mut runner, "Git.Git", false);
            assert_eq!(outcome.is_ok(), verified == 0);
            assert_eq!(
                runner.calls,
                vec![Operation::List, Operation::Install, Operation::List]
            );
        }
    }

    #[test]
    fn ordinary_gui_arguments_and_invalid_notification_are_side_effect_free() {
        assert_eq!(
            run_from_args(&["supermenu.exe".into(), "--background".into()]),
            None
        );
        assert!(!notify(null_mut(), 0, 0, 0));
    }

    #[test]
    fn diagnostic_never_installs_and_no_wait_requires_check_only() {
        for code in [0, NO_APPLICATIONS_FOUND, 0x8A15_0045] {
            let mut runner = FakeRunner::codes(&[code]);
            assert_eq!(
                perform_install(&mut runner, "Git.Git", true).is_ok(),
                code == 0
            );
            assert_eq!(runner.calls, vec![Operation::List]);
        }
        let mut args = vec!["supermenu.exe".into()];
        args.extend(helper_args(packages::PACKAGES[0].id, 123, 456, 789));
        args.push("--no-wait".into());
        assert!(parse_request(&args).is_err());
        args.push("--check-only".into());
        let request = parse_request(&args).unwrap();
        assert!(request.check_only && request.no_wait);
        args.push("--check-only".into());
        assert!(parse_request(&args).is_err());
    }

    #[test]
    fn removal_targets_require_whitelisted_ids_or_exact_removable_names() {
        let id = packages::PACKAGES[0].id;
        assert_eq!(
            remove_target(&format!("id:{}", id.to_ascii_lowercase())).unwrap(),
            RemoveTarget::Id(id)
        );
        let name = "Éditeur \"Pro\" (x64)";
        let target = remove_target(&format!("name:{name}")).unwrap();
        assert!(validate_removal_inventory(&target, &[name.into()], &[name.into()]).is_ok());
        assert!(
            validate_removal_inventory(&target, &[name.into()], &[name.to_ascii_lowercase()])
                .is_err()
        );
        assert!(validate_removal_inventory(&target, &[], &[]).is_err());
        for invalid in [
            "id:Arbitrary.Package",
            "Firefox",
            "name:",
            "name:--all",
            "name: Leading",
            "name:Trailing ",
            "name:A\0B",
            "name:A\nB",
        ] {
            assert!(remove_target(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn helper_package_modes_are_exclusive_and_removal_diagnostics_parse() {
        let target = format!("id:{}", packages::PACKAGES[0].id);
        let mut args = vec!["supermenu.exe".into()];
        args.extend(remove_helper_args(&target, 123, 456, 789));
        args.extend(["--check-only".into(), "--no-wait".into()]);
        let request = parse_request(&args).unwrap();
        assert!(request.remove.is_some());
        assert!(request.check_only && request.no_wait);
        assert_eq!(request.token, 789);
        args.extend(["--install-package".into(), packages::PACKAGES[0].id.into()]);
        assert!(parse_request(&args).is_err());
        let mut duplicate = vec!["supermenu.exe".into()];
        duplicate.extend(remove_helper_args(&target, 123, 456, 789));
        duplicate.extend(["--uninstall-package".into(), target]);
        assert!(parse_request(&duplicate).is_err());
        assert!(!notify_remove(null_mut(), 0, 1, 0));
    }

    #[test]
    fn removal_commands_preserve_one_exact_selector_without_destructive_overrides() {
        for (operation, selector) in [
            (Operation::RemoveId, "--id"),
            (Operation::RemoveName, "--name"),
        ] {
            let value = "Exact Display Name";
            let args = command_args(operation, value);
            let mut expected = vec!["uninstall", selector, value, "--exact"];
            if operation == Operation::RemoveId {
                expected.extend(["--source", "winget"]);
            }
            expected.extend([
                "--accept-source-agreements",
                "--disable-interactivity",
                "--silent",
            ]);
            assert_eq!(args, expected);
            for forbidden in [
                "--all",
                "--all-versions",
                "--force",
                "--purge",
                "--accept-package-agreements",
                "--no-upgrade",
            ] {
                assert!(!args.contains(&forbidden));
            }
        }
        assert_eq!(
            command_args(Operation::ListName, "Display Name")[1],
            "--name"
        );
    }

    #[test]
    fn absent_uncertain_and_ambiguous_removal_states_never_uninstall() {
        let target = RemoveTarget::Id(packages::PACKAGES[0].id);
        for code in [NO_APPLICATIONS_FOUND, 0x8A15_0016, 0x8A15_0045, 1, 5] {
            let mut runner = FakeRunner::codes(&[code]);
            let outcome = perform_remove(&mut runner, &target, false);
            assert!(outcome.is_err());
            assert_eq!(runner.calls, vec![Operation::List]);
        }
        let mut runner = FakeRunner::codes(&[]);
        runner
            .results
            .push_back(Err("Unable to query source".into()));
        assert!(perform_remove(&mut runner, &target, false).is_err());
        assert_eq!(runner.calls, vec![Operation::List]);
    }

    #[test]
    fn removal_success_race_cancellation_and_ambiguity_preserve_exit_codes() {
        for (target, list, remove) in [
            (
                RemoveTarget::Id(packages::PACKAGES[0].id),
                Operation::List,
                Operation::RemoveId,
            ),
            (
                RemoveTarget::Name("Display Name".into()),
                Operation::ListName,
                Operation::RemoveName,
            ),
        ] {
            for code in [0, NO_APPLICATIONS_FOUND, 0x8A15_0016, 0x8A15_0005, 1603] {
                let mut runner = if code == 0 {
                    FakeRunner::codes(&[0, code, NO_APPLICATIONS_FOUND])
                } else {
                    FakeRunner::codes(&[0, code])
                };
                let outcome = perform_remove(&mut runner, &target, false);
                assert_eq!(outcome.is_ok(), code == 0);
                if let Err(error) = outcome {
                    assert_eq!(error.code as u32, code);
                }
                assert_eq!(
                    runner.calls,
                    if code == 0 {
                        vec![list, remove, list]
                    } else {
                        vec![list, remove]
                    }
                );
            }
        }
    }

    #[test]
    fn removal_diagnostic_never_invokes_uninstall_even_when_present() {
        let target = RemoveTarget::Name("Display Name".into());
        for code in [0, NO_APPLICATIONS_FOUND, 0x8A15_0016, 0x8A15_0045] {
            let mut runner = FakeRunner::codes(&[code]);
            let outcome = perform_remove(&mut runner, &target, true);
            assert_eq!(outcome.is_ok(), code == 0);
            assert_eq!(runner.calls, vec![Operation::ListName]);
        }
    }

    #[test]
    fn cleanup_only_runs_after_verified_absence_and_never_on_diagnostics_or_failures() {
        use std::cell::Cell;
        let target = RemoveTarget::Name("SuperMenu Cleanup Fixture".into());
        for (codes, diagnostic, should_clean) in [
            (vec![0, 0, NO_APPLICATIONS_FOUND], false, true),
            (vec![0, 0, 0], false, false),
            (vec![0, 0, 5], false, false),
            (vec![0, 1603], false, false),
            (vec![NO_APPLICATIONS_FOUND], false, false),
            (vec![0], true, false),
        ] {
            let mut runner = FakeRunner::codes(&codes);
            let cleaned = Cell::new(false);
            let result = perform_remove_and_cleanup(&mut runner, &target, diagnostic, || {
                cleaned.set(true);
            });
            assert_eq!(cleaned.get(), should_clean, "{codes:?}");
            assert_eq!(result.is_ok(), should_clean || diagnostic, "{codes:?}");
        }
    }

    #[test]
    fn protected_catalog_ids_and_names_stop_before_any_winget_command() {
        let package = &packages::PACKAGES[0];
        for target in [
            RemoveTarget::Id(package.id),
            RemoveTarget::Name(package.name.into()),
        ] {
            let mut runner = FakeRunner::codes(&[]);
            let outcome = validate_removal_inventory(&target, &[package.name.into()], &[])
                .map_err(Failure::local)
                .and_then(|_| perform_remove(&mut runner, &target, false));
            assert!(outcome.is_err());
            assert!(runner.calls.is_empty());
            assert!(validate_removal_inventory(
                &target,
                &[package.name.into()],
                &[package.name.into()]
            )
            .is_ok());
        }
        // WinGet still performs the authoritative preflight for UWP/index entries.
        assert!(validate_removal_inventory(&RemoveTarget::Id(package.id), &[], &[]).is_ok());
    }
}
