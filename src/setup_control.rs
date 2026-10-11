//! An installer can stop only launchers loaded from its exact installation path.
//! Helpers sharing that image must finish first; no process is terminated forcibly.

use std::mem::size_of;
use std::path::{Component, Path, PathBuf, Prefix};
use std::time::Duration;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Globalization::{CompareStringOrdinal, CSTR_EQUAL},
    System::{
        ProcessStatus::K32EnumProcesses,
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, WaitForMultipleObjects,
            WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        },
    },
    UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId, PostThreadMessageW, WM_QUIT,
    },
};

const OPTION: &str = "--shutdown-for-install";
const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROCESSES: usize = 131_072;

struct Process {
    handle: HANDLE,
    pid: u32,
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

struct Window {
    hwnd: HWND,
    pid: u32,
    thread: u32,
}

pub fn run_from_args(args: &[String]) -> Option<i32> {
    if !args.iter().any(|argument| argument == OPTION) {
        return None;
    }
    let expected = match expected_image(args) {
        Ok(expected) => expected,
        Err(error) => {
            eprintln!("Super Menu : {error}");
            return Some(2);
        }
    };
    match shutdown(&expected) {
        Ok(()) => Some(0),
        Err(error) => {
            eprintln!("Super Menu : {error}");
            Some(1)
        }
    }
}

fn expected_image(args: &[String]) -> Result<Vec<u16>, String> {
    let [_, option, expected] = args else {
        return Err(
            "L'arrêt pour installation exige un unique chemin absolu vers supermenu.exe.".into(),
        );
    };
    if option != OPTION
        || expected.chars().any(char::is_control)
        || expected.encode_utf16().count() > 32_767
    {
        return Err("Arguments d'arrêt pour installation invalides.".into());
    }
    let path = Path::new(expected);
    if !path.is_absolute()
        || !path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.eq_ignore_ascii_case("supermenu.exe"))
        || path.components().any(|component| {
            matches!(component, Component::ParentDir)
                || matches!(component, Component::Prefix(prefix) if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_) | Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _)))
        })
    {
        return Err("La cible doit être un chemin absolu sans traversée vers supermenu.exe.".into());
    }
    normalized_image(path)
}

fn normalized_image(path: &Path) -> Result<Vec<u16>, String> {
    // Missing files are valid on a first installation or after a rename. Resolve
    // their existing parent when possible, without requiring the exe to exist.
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| {
        path.parent()
            .and_then(|parent| std::fs::canonicalize(parent).ok())
            .and_then(|parent| path.file_name().map(|name| parent.join(name)))
            .unwrap_or_else(|| PathBuf::from(path))
    });
    let text = resolved
        .to_str()
        .ok_or("Chemin d'exécutable UTF-16 invalide.")?;
    Ok(normalized_text(text))
}

fn normalized_text(text: &str) -> Vec<u16> {
    let text = text.replace('/', "\\");
    let ordinary = if text
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("\\\\?\\UNC\\"))
    {
        format!("\\\\{}", &text[8..])
    } else {
        text.strip_prefix("\\\\?\\").unwrap_or(&text).to_string()
    };
    ordinary.encode_utf16().collect()
}

fn same_image(first: &[u16], second: &[u16]) -> bool {
    !first.is_empty()
        && first.len() <= i32::MAX as usize
        && second.len() <= i32::MAX as usize
        && unsafe {
            CompareStringOrdinal(
                first.as_ptr(),
                first.len() as i32,
                second.as_ptr(),
                second.len() as i32,
                1,
            ) == CSTR_EQUAL
        }
}

fn image_name(process: &Process) -> Result<Vec<u16>, String> {
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process.handle, 0, buffer.as_mut_ptr(), &mut length) }
        == 0
    {
        return Err(os_error("Identification du processus"));
    }
    if length == 0 || length as usize >= buffer.len() {
        return Err("Longueur du chemin du processus invalide.".into());
    }
    let name = String::from_utf16(&buffer[..length as usize])
        .map_err(|_| "Chemin du processus UTF-16 invalide.")?;
    normalized_image(Path::new(&name))
}

fn process_ids() -> Result<Vec<u32>, String> {
    let mut ids = vec![0u32; 1024];
    loop {
        let bytes = (ids.len() * size_of::<u32>()) as u32;
        let mut written = 0;
        if unsafe { K32EnumProcesses(ids.as_mut_ptr(), bytes, &mut written) } == 0 {
            return Err(os_error("Liste des processus"));
        }
        if written < bytes {
            ids.truncate(written as usize / size_of::<u32>());
            return Ok(ids);
        }
        if ids.len() >= MAX_PROCESSES {
            return Err("La liste des processus dépasse la limite de sécurité.".into());
        }
        ids.resize(ids.len() * 2, 0);
    }
}

fn matching_processes(expected: &[u16], session: u32) -> Result<Vec<Process>, String> {
    let own_pid = unsafe { GetCurrentProcessId() };
    let mut matches = Vec::new();
    for pid in process_ids()? {
        if pid == 0 || pid == own_pid {
            continue;
        }
        let mut candidate_session = 0;
        if unsafe { ProcessIdToSessionId(pid, &mut candidate_session) } == 0
            || candidate_session != session
        {
            continue;
        }
        let query = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if query.is_null() {
            // Windows contains protected processes that cannot be queried by a
            // normal user. Never infer their identity or signal them by name.
            continue;
        }
        let query = Process { handle: query, pid };
        if unsafe { ProcessIdToSessionId(pid, &mut candidate_session) } == 0
            || candidate_session != session
        {
            continue;
        }
        if !image_name(&query).is_ok_and(|image| same_image(expected, &image)) {
            continue;
        }
        // Keep the query handle alive while obtaining the waitable handle; a
        // numeric PID alone must never become authority to close a process.
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            return Err(os_error("Attente du processus Super Menu"));
        }
        let process = Process { handle, pid };
        if unsafe { WaitForSingleObject(process.handle, 0) } == WAIT_OBJECT_0 {
            continue;
        }
        if !same_image(expected, &image_name(&process)?) {
            return Err("L'identité d'un processus Super Menu a changé.".into());
        }
        matches.push(process);
    }
    Ok(matches)
}

fn main_windows() -> Result<Vec<Window>, String> {
    unsafe extern "system" fn collect(hwnd: HWND, parameter: LPARAM) -> i32 {
        let mut class = [0u16; 64];
        let length = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        if length > 0
            && class[..length as usize]
                .iter()
                .copied()
                .eq("SuperMenuWindow".encode_utf16())
        {
            let mut pid = 0;
            let thread = GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != 0 && thread != 0 {
                let windows = &mut *(parameter as *mut Vec<Window>);
                windows.push(Window { hwnd, pid, thread });
            }
        }
        1
    }
    let mut windows = Vec::new();
    if unsafe { EnumWindows(Some(collect), (&mut windows as *mut Vec<Window>) as LPARAM) } == 0 {
        return Err(os_error("Identification des fenêtres Super Menu"));
    }
    Ok(windows)
}

fn shutdown(expected: &[u16]) -> Result<(), String> {
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return Err(os_error("Identification de la session Windows"));
    }
    let processes = matching_processes(expected, session)?;
    if processes.is_empty() {
        return Ok(());
    }
    if processes.len() > 64 {
        return Err("Trop de processus Super Menu pour un arrêt borné.".into());
    }
    let windows = main_windows()?;
    let mut launchers = Vec::new();
    for process in &processes {
        if unsafe { WaitForSingleObject(process.handle, 0) } == WAIT_OBJECT_0 {
            continue;
        }
        let owned: Vec<_> = windows
            .iter()
            .filter(|window| window.pid == process.pid)
            .collect();
        let [window] = owned.as_slice() else {
            return Err("Une opération de Super Menu est en cours, ou son démarrage n'est pas terminé. Réessaie après son achèvement.".into());
        };
        launchers.push((process, *window));
    }
    // Preflight all matching processes before closing any launcher. In particular
    // an elevated WinGet/cleanup helper never has this main UI window.
    for (process, window) in &launchers {
        let mut owner = 0;
        let thread = unsafe { GetWindowThreadProcessId(window.hwnd, &mut owner) };
        let mut class = [0u16; 64];
        let length = unsafe { GetClassNameW(window.hwnd, class.as_mut_ptr(), class.len() as i32) };
        if owner != process.pid
            || thread != window.thread
            || length <= 0
            || !class[..length as usize]
                .iter()
                .copied()
                .eq("SuperMenuWindow".encode_utf16())
            || !same_image(expected, &image_name(process)?)
        {
            if unsafe { WaitForSingleObject(process.handle, 0) } == WAIT_OBJECT_0 {
                continue;
            }
            return Err("La fenêtre du launcher a changé ; arrêt refusé.".into());
        }
        if unsafe { PostThreadMessageW(thread, WM_QUIT, 0, 0) } == 0 {
            return Err(os_error("Arrêt du launcher Super Menu"));
        }
    }
    if !launchers.is_empty() {
        let handles: Vec<_> = launchers
            .iter()
            .map(|(process, _)| process.handle)
            .collect();
        let result = unsafe {
            WaitForMultipleObjects(
                handles.len() as u32,
                handles.as_ptr(),
                1,
                TIMEOUT.as_millis() as u32,
            )
        };
        if result != WAIT_OBJECT_0 {
            return Err(if result == WAIT_TIMEOUT {
                "Super Menu n'a pas terminé son arrêt en cinq secondes. Aucun processus n'a été tué.".into()
            } else {
                os_error("Attente de l'arrêt de Super Menu")
            });
        }
    }
    if !matching_processes(expected, session)?.is_empty() {
        return Err(
            "Un processus Super Menu est encore actif ; remplacement des fichiers refusé.".into(),
        );
    }
    Ok(())
}

fn os_error(context: &str) -> String {
    format!("{context} : {}", std::io::Error::last_os_error())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn shutdown_arguments_reject_ambiguous_or_foreign_executables() {
        assert_eq!(
            run_from_args(&args(&["supermenu.exe", "--benchmark"])),
            None
        );
        for values in [
            vec!["supermenu.exe", OPTION],
            vec!["supermenu.exe", OPTION, "supermenu.exe"],
            vec!["supermenu.exe", OPTION, "C:\\fixture\\other.exe"],
            vec!["supermenu.exe", OPTION, "C:\\fixture\\..\\supermenu.exe"],
            vec!["supermenu.exe", OPTION, "C:\\fixture\\supermenu.exe\0"],
            vec![
                "supermenu.exe",
                OPTION,
                "C:\\fixture\\supermenu.exe",
                "--background",
            ],
            vec![
                "supermenu.exe",
                OPTION,
                "C:\\fixture\\supermenu.exe",
                OPTION,
            ],
            vec!["supermenu.exe", OPTION, "\\\\.\\fixture\\supermenu.exe"],
        ] {
            assert!(expected_image(&args(&values)).is_err(), "{values:?}");
        }
    }

    #[test]
    fn image_comparison_handles_unicode_case_and_fresh_absolute_paths() {
        let ordinary = expected_image(&args(&[
            "supermenu.exe",
            OPTION,
            "C:\\SuperMenu-fixture-missing\\Éditeur\\supermenu.exe",
        ]))
        .unwrap();
        let verbatim = expected_image(&args(&[
            "supermenu.exe",
            OPTION,
            "\\\\?\\C:\\SuperMenu-fixture-missing\\Éditeur\\supermenu.exe",
        ]))
        .unwrap();
        let other_case: Vec<u16> = "c:\\supermenu-fixture-missing\\éditeur\\SUPERMENU.EXE"
            .encode_utf16()
            .collect();
        let foreign: Vec<u16> = "c:\\supermenu-fixture-missing\\éditeur-autre\\supermenu.exe"
            .encode_utf16()
            .collect();
        assert!(same_image(&ordinary, &verbatim));
        assert!(same_image(&ordinary, &other_case));
        assert!(!same_image(&ordinary, &foreign));
        assert!(!same_image(&[], &[]));
        let unc = normalized_text("\\\\supermenu-fixture.invalid\\share\\supermenu.exe");
        let unc_verbatim =
            normalized_text("\\\\?\\UNC\\supermenu-fixture.invalid\\share\\supermenu.exe");
        assert!(same_image(&unc, &unc_verbatim));
    }
}
