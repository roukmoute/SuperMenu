//! A separate instance of this executable owns WinGet and its visible console.
//! No installation or network request runs on the launcher's UI thread.

use crate::{native::wide, packages};
use std::io::{self, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, HWND, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    AllocConsole, GetConsoleMode, GetConsoleWindow, SetConsoleCP, SetConsoleMode,
    SetConsoleOutputCP, SetConsoleTitleW, SetStdHandle, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Threading::DETACHED_PROCESS;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowThreadProcessId, IsWindow, MessageBoxW, PostMessageW, MB_ICONERROR,
    MB_OK, WM_APP,
};

pub const INSTALL_DONE: u32 = WM_APP + 5;

// Official WinGet HRESULTs, compared as u32 because Windows exit codes are unsigned.
// https://github.com/microsoft/winget-cli/blob/master/doc/windows/package-manager/winget/returnCodes.md
const NO_APPLICATIONS_FOUND: u32 = 0x8A15_0014;
const UPDATE_NOT_APPLICABLE: u32 = 0x8A15_002B;
const PACKAGE_ALREADY_INSTALLED: u32 = 0x8A15_0061;

struct Request {
    id: &'static str,
    name: &'static str,
    parent: HWND,
    parent_pid: u32,
    token: usize,
    check_only: bool,
    no_wait: bool,
}

/// Runs only the private installation-helper mode. Normal GUI arguments return None.
pub fn run_from_args(args: &[String]) -> Option<i32> {
    if !args.iter().skip(1).any(|arg| arg == "--install-package") {
        return None;
    }
    let request = parse_request(args);
    let console = match Console::open() {
        Ok(console) => console,
        Err(error) => {
            if let Ok(request) = &request {
                notify(request.parent, request.parent_pid, request.token, 1);
            }
            unsafe {
                MessageBoxW(
                    null_mut(),
                    wide(&error).as_ptr(),
                    wide("Super Menu · Installation").as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
            return Some(1);
        }
    };
    let mut wait = true;
    let result = match request {
        Ok(request) => {
            wait = !request.no_wait;
            println!("Super Menu · Installation de {}\n", request.name);
            let outcome = if valid_parent(request.parent, request.parent_pid) {
                resolve_winget().map_err(Failure::local).and_then(|exe| {
                    perform_install(&mut SystemRunner { exe }, request.id, request.check_only)
                })
            } else {
                Err(Failure::local(
                    "La fenêtre Super Menu n'est plus disponible.",
                ))
            };
            let result = match outcome {
                Ok(Outcome::Installed) => {
                    println!(
                        "\nInstallation terminée. Le catalogue de Super Menu va être actualisé."
                    );
                    0
                }
                Ok(Outcome::AlreadyInstalled) => {
                    println!(
                        "\nCe logiciel est déjà installé. Aucune mise à jour n'a été effectuée."
                    );
                    0
                }
                Err(error) => {
                    eprintln!("\nÉchec : {}", error.message);
                    error.code
                }
            };
            // Notify immediately. Closing the console is independent of the next request.
            notify(request.parent, request.parent_pid, request.token, result);
            result
        }
        Err(error) => {
            eprintln!("Requête d'installation refusée : {error}");
            1
        }
    };
    if wait {
        println!("\nAppuyez sur Entrée pour fermer cette fenêtre.");
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
        let _ = io::stdin().read_line(&mut String::new());
    }
    drop(console);
    Some(result)
}

/// Starts the helper; the caller may wait for Child on a worker as a notification fallback.
pub fn spawn(id: &str, parent: HWND, parent_pid: u32, request: usize) -> Result<Child, String> {
    let package = whitelisted(id)?;
    if request == 0 || !valid_parent(parent, parent_pid) {
        return Err("La requête d'installation n'a pas de fenêtre Super Menu valide.".into());
    }
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    Command::new(exe)
        .args(helper_args(
            package.id,
            parent as usize,
            parent_pid,
            request,
        ))
        // AllocConsole creates a new visible console even when the debug launcher has one.
        .creation_flags(DETACHED_PROCESS)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Impossible d'ouvrir la console d'installation : {error}"))
}

/// Tokens identify attempts, so an old console cannot complete a newer retry.
pub fn notify(parent: HWND, parent_pid: u32, request: usize, result: i32) -> bool {
    request != 0
        && valid_parent(parent, parent_pid)
        && unsafe { PostMessageW(parent, INSTALL_DONE, request, result as isize) != 0 }
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
        .ok_or_else(|| "Identifiant absent du catalogue d'installation autorisé.".into())
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

fn parse_request(args: &[String]) -> Result<Request, String> {
    let mut id = None;
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
            "--notify" => &mut parent,
            "--parent-pid" => &mut pid,
            "--request" => &mut token,
            _ => return Err(format!("Option inconnue : {flag}")),
        };
        if slot.is_some() {
            return Err(format!("Option répétée : {flag}"));
        }
        *slot = Some(
            values
                .next()
                .ok_or_else(|| format!("Valeur manquante : {flag}"))?,
        );
    }
    let package = whitelisted(id.ok_or("Identifiant manquant.")?)?;
    let parent = parse_number::<usize>(parent, "Fenêtre")?;
    let pid = parse_number::<u32>(pid, "Processus")?;
    let token = parse_number::<usize>(token, "Requête")?;
    if parent == 0 || pid == 0 || token == 0 {
        return Err("Fenêtre, processus et requête doivent être non nuls.".into());
    }
    if no_wait && !check_only {
        return Err("--no-wait est réservé au diagnostic --check-only.".into());
    }
    Ok(Request {
        id: package.id,
        name: package.name,
        parent: parent as HWND,
        parent_pid: pid,
        token,
        check_only,
        no_wait,
    })
}

fn parse_number<T: std::str::FromStr>(value: Option<&String>, name: &str) -> Result<T, String> {
    let value = value.ok_or_else(|| format!("{name} manquant."))?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{name} invalide."));
    }
    value.parse().map_err(|_| format!("{name} invalide."))
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
    Err("WinGet est introuvable. Installez ou réparez « App Installer » de Microsoft, puis réessayez. https://aka.ms/getwinget".into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    List,
    Install,
}

fn command_args(operation: Operation, id: &str) -> Vec<&str> {
    let mut args = vec![
        match operation {
            Operation::List => "list",
            Operation::Install => "install",
        },
        "--id",
        id,
        "--exact",
        "--source",
        "winget",
        "--accept-source-agreements",
        "--disable-interactivity",
    ];
    if operation == Operation::Install {
        args.extend(["--no-upgrade", "--accept-package-agreements", "--silent"]);
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
                format!("WinGet ne peut pas être exécuté ({error}). Vérifiez App Installer et l'alias d'exécution des applications dans les paramètres Windows.")
            })?
            .code()
            .ok_or_else(|| "WinGet a été interrompu sans résultat.".into())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Installed,
    AlreadyInstalled,
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
            message: message.into(),
        }
    }

    fn winget(code: i32, context: &str) -> Self {
        Self {
            code,
            message: format!(
                "{context} (WinGet 0x{:08X}). Consultez le détail affiché ci-dessus.",
                code as u32
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

struct Console {
    input: HANDLE,
    output: HANDLE,
}

impl Console {
    fn open() -> Result<Self, String> {
        unsafe {
            if GetConsoleWindow().is_null() && AllocConsole() == 0 {
                return Err(format!(
                    "Impossible d'ouvrir la console : {}",
                    io::Error::last_os_error()
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
                return Err(format!(
                    "Console indisponible : {}",
                    io::Error::last_os_error()
                ));
            }
            SetConsoleTitleW(wide("Super Menu · Installation").as_ptr());
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
        Err(format!(
            "Console indisponible : {}",
            io::Error::last_os_error()
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
}
