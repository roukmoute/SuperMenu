//! Wait for a package command and the descendants it actually starts.
//! A bootstrapper can exit while its uninstaller is still showing a window.
//! The anonymous job observes those processes; it never kills running software.

use std::io;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::ptr::{null, null_mut};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, ERROR_MORE_DATA, GENERIC_READ,
    GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    GetConsoleCP, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
    JobObjectBasicAccountingInformation, JobObjectBasicProcessIdList, QueryInformationJobObject,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_PROCESS_ID_LIST,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess,
    InitializeProcThreadAttributeList, OpenProcess, QueryFullProcessImageNameW, ResumeThread,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW,
    CREATE_SUSPENDED, EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// The handle list limits inheritance to our three duplicated console streams.
/// Pointer-sized storage supplies the native structure's required alignment.
struct Attributes {
    storage: Vec<usize>,
    initialized: bool,
}

impl Attributes {
    fn new(handles: &[HANDLE]) -> io::Result<Self> {
        unsafe {
            let mut bytes = 0;
            InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes);
            if bytes == 0 || bytes > 1024 * 1024 {
                return Err(last_error("Cannot size the process handle list"));
            }
            let mut attributes = Self {
                storage: vec![0; bytes.div_ceil(size_of::<usize>())],
                initialized: false,
            };
            if InitializeProcThreadAttributeList(attributes.pointer(), 1, 0, &mut bytes) == 0 {
                // An uninitialized list must not be passed to its native destructor.
                return Err(last_error("Cannot initialize the process handle list"));
            }
            attributes.initialized = true;
            if UpdateProcThreadAttribute(
                attributes.pointer(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                std::mem::size_of_val(handles),
                null_mut(),
                null(),
            ) == 0
            {
                return Err(last_error("Cannot set the inherited process handles"));
            }
            Ok(attributes)
        }
    }

    fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { DeleteProcThreadAttributeList(self.pointer()) };
        }
    }
}

/// Only this newly created, never-resumed process may be terminated on an error.
/// Once resumed, even a wait failure merely releases handles without closing apps.
struct SuspendedChild {
    process: Handle,
    thread: Handle,
    suspended: bool,
}

impl Drop for SuspendedChild {
    fn drop(&mut self) {
        if self.suspended {
            unsafe {
                if TerminateProcess(self.process.0, 1) != 0 {
                    WaitForSingleObject(self.process.0, 5000);
                }
            }
        }
    }
}

fn last_error(context: &str) -> io::Error {
    let error = io::Error::last_os_error();
    io::Error::new(error.kind(), format!("{context}: {error}"))
}

fn duplicate_stream(kind: u32) -> io::Result<Handle> {
    unsafe {
        let source = GetStdHandle(kind);
        // Headless test runners can have no stdin. Preserve that behavior with
        // NUL rather than supplying an invalid startup handle to the child.
        let fallback = if source.is_null() || source == INVALID_HANDLE_VALUE {
            let nul = [b'N' as u16, b'U' as u16, b'L' as u16, 0];
            let handle = CreateFileW(
                nul.as_ptr(),
                if kind == STD_INPUT_HANDLE {
                    GENERIC_READ
                } else {
                    GENERIC_WRITE
                },
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            );
            if handle == INVALID_HANDLE_VALUE {
                return Err(last_error("Cannot open the absent standard stream"));
            }
            Some(Handle(handle))
        } else {
            None
        };
        let source = fallback.as_ref().map_or(source, |handle| handle.0);
        let process = GetCurrentProcess();
        let mut duplicate = null_mut();
        if DuplicateHandle(
            process,
            source,
            process,
            &mut duplicate,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        ) == 0
        {
            return Err(last_error("Cannot inherit the standard stream"));
        }
        Ok(Handle(duplicate))
    }
}

fn quote_argument(value: &str) -> String {
    // Match the standard Windows argument convention: leave switches and simple
    // values bare. cmd.exe parses its switches itself rather than using CRT argv.
    if !value.is_empty()
        && !value
            .bytes()
            .any(|byte| matches!(byte, b' ' | b'\t' | b'"'))
    {
        return value.to_owned();
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        quoted.extend(std::iter::repeat_n(
            '\\',
            if character == '"' {
                backslashes * 2 + 1
            } else {
                backslashes
            },
        ));
        quoted.push(character);
        backslashes = 0;
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

/// Starts the command in a job before its first instruction and waits for its
/// complete process tree. The returned status always belongs to the root command.
/// CWD, environment, console and standard streams are inherited from the helper.
pub(crate) fn run(program: &Path, args: &[&str]) -> io::Result<ExitStatus> {
    run_inner(program, args, None)
}

/// The caller may exclude the canonical default-browser executable only after
/// verifying that it is outside the product being removed. A browser launched
/// for a vendor survey must not keep the package operation open indefinitely.
pub(crate) fn run_with_ignored_browser(
    program: &Path,
    args: &[&str],
    ignored_browser: Option<&Path>,
) -> io::Result<ExitStatus> {
    match ignored_browser {
        Some(browser) => run_inner(program, args, Some(browser)),
        None => run(program, args),
    }
}

fn same_image(left: &Path, right: &Path) -> bool {
    let left: Vec<u16> = left.as_os_str().encode_wide().collect();
    let right: Vec<u16> = right.as_os_str().encode_wide().collect();
    unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
            1,
        ) == CSTR_EQUAL
    }
}

fn job_processes(job: HANDLE) -> io::Result<Vec<usize>> {
    let mut capacity = 64_usize;
    loop {
        // The two DWORD counters precede the native pointer-sized PID array.
        // Aligned storage accommodates the structure's variable trailing array.
        let bytes = 8 + capacity * size_of::<usize>();
        let mut storage = vec![0_usize; bytes.div_ceil(size_of::<usize>())];
        unsafe {
            let list = storage
                .as_mut_ptr()
                .cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
            let success = QueryInformationJobObject(
                job,
                JobObjectBasicProcessIdList,
                list.cast(),
                bytes as u32,
                null_mut(),
            ) != 0;
            let error = (!success).then(io::Error::last_os_error);
            let assigned = (*list).NumberOfAssignedProcesses as usize;
            let count = (*list).NumberOfProcessIdsInList as usize;
            if success && assigned <= count && count <= capacity {
                return Ok(std::slice::from_raw_parts(
                    std::ptr::addr_of!((*list).ProcessIdList).cast::<usize>(),
                    count,
                )
                .to_vec());
            }
            if let Some(error) = error {
                if error.raw_os_error() != Some(ERROR_MORE_DATA as i32) {
                    return Err(io::Error::new(
                        error.kind(),
                        format!("Cannot enumerate the package process tree: {error}"),
                    ));
                }
            }
            capacity = capacity.saturating_mul(2).max(assigned);
            if capacity > 4096 {
                return Err(io::Error::other(
                    "The package process tree exceeds 4096 processes",
                ));
            }
        }
    }
}

fn ignored_job_process(job: HANDLE, pid: usize, browser: &Path) -> bool {
    let Ok(pid) = u32::try_from(pid) else {
        return false;
    };
    unsafe {
        let process = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        );
        if process.is_null() {
            // An exited process disappears on the next job snapshot. Incomplete
            // permission/path evidence never lets us disregard a live process.
            return false;
        }
        let process = Handle(process);
        let mut member = 0;
        let membership_success = IsProcessInJob(process.0, job, &mut member) != 0;
        if !membership_success || member == 0 {
            // Recheck membership through the opened handle to avoid PID reuse.
            return false;
        }
        // An exited root can remain in the job's PID list while its process
        // handle is retained for the exit status. It no longer delays cleanup.
        if WaitForSingleObject(process.0, 0) == WAIT_OBJECT_0 {
            return true;
        }
        let mut image = vec![0_u16; 32_768];
        let mut length = image.len() as u32;
        if QueryFullProcessImageNameW(process.0, 0, image.as_mut_ptr(), &mut length) == 0 {
            return false;
        }
        image.truncate(length as usize);
        let image = PathBuf::from(std::ffi::OsString::from_wide(&image));
        let canonical = std::fs::canonicalize(&image);
        canonical.is_ok_and(|image| same_image(&image, browser))
    }
}

fn run_inner(
    program: &Path,
    args: &[&str],
    ignored_browser: Option<&Path>,
) -> io::Result<ExitStatus> {
    let mut executable: Vec<u16> = program.as_os_str().encode_wide().collect();
    if !program.is_absolute()
        || executable.contains(&0)
        || args.iter().any(|arg| arg.contains('\0'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "An absolute executable and arguments without NUL are required",
        ));
    }
    // argv[0] needs only surrounding quotes: a Windows filename cannot contain
    // a literal quote, and an executable filename cannot end in a backslash.
    let mut command = vec![b'"' as u16];
    command.extend_from_slice(&executable);
    command.push(b'"' as u16);
    for argument in args {
        command.push(b' ' as u16);
        command.extend(quote_argument(argument).encode_utf16());
    }
    command.push(0);
    if command.len() > 32_767 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The Windows command line exceeds 32767 UTF-16 characters",
        ));
    }
    executable.push(0);
    // The seed executable itself can never be excluded. A missing or unresolved
    // exclusion stays conservative and uses the strict complete-tree wait.
    let ignored_browser = ignored_browser
        .and_then(|path| std::fs::canonicalize(path).ok())
        .filter(|browser| {
            std::fs::canonicalize(program).is_ok_and(|program| !same_image(&program, browser))
        });

    unsafe {
        // No kill-on-close, time/memory limits or breakaway flags: this job only
        // observes descendants, including an uninstaller's temporary bootstrap.
        // Windows 10+ supports nesting this inside the helper's existing job.
        let job = CreateJobObjectW(null(), null());
        if job.is_null() {
            return Err(last_error("Cannot create the package process job"));
        }
        let job = Handle(job);
        let streams = [
            duplicate_stream(STD_INPUT_HANDLE)?,
            duplicate_stream(STD_OUTPUT_HANDLE)?,
            duplicate_stream(STD_ERROR_HANDLE)?,
        ];
        let stream_handles = streams.each_ref().map(|handle| handle.0);
        let mut attributes = Attributes::new(&stream_handles)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = stream_handles[0];
        startup.StartupInfo.hStdOutput = stream_handles[1];
        startup.StartupInfo.hStdError = stream_handles[2];
        startup.lpAttributeList = attributes.pointer();
        let mut information = PROCESS_INFORMATION::default();
        // Reuse the helper's AllocConsole. When the caller has no console (e.g.
        // piped tests), do not create a conhost that outlives the bootstrapper.
        let flags = CREATE_SUSPENDED
            | EXTENDED_STARTUPINFO_PRESENT
            | if GetConsoleCP() == 0 {
                CREATE_NO_WINDOW
            } else {
                0
            };
        if CreateProcessW(
            executable.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            1,
            flags,
            null(),
            null(),
            &startup.StartupInfo,
            &mut information,
        ) == 0
        {
            return Err(last_error("Cannot start the package command"));
        }
        let mut child = SuspendedChild {
            process: Handle(information.hProcess),
            thread: Handle(information.hThread),
            suspended: true,
        };
        drop(attributes);
        drop(streams);
        if AssignProcessToJobObject(job.0, child.process.0) == 0 {
            return Err(last_error(
                "Cannot track the package command before execution",
            ));
        }
        if ResumeThread(child.thread.0) == u32::MAX {
            return Err(last_error("Cannot resume the tracked package command"));
        }
        child.suspended = false;
        match WaitForSingleObject(child.process.0, INFINITE) {
            WAIT_OBJECT_0 => {}
            WAIT_FAILED => return Err(last_error("Cannot wait for the package command")),
            result => {
                return Err(io::Error::other(format!(
                    "Unexpected process wait: {result}"
                )))
            }
        }
        let mut code = 0;
        if GetExitCodeProcess(child.process.0, &mut code) == 0 {
            return Err(last_error("Cannot read the package command result"));
        }
        let mut announced = false;
        loop {
            let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            if QueryInformationJobObject(
                job.0,
                JobObjectBasicAccountingInformation,
                (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                null_mut(),
            ) == 0
            {
                return Err(last_error(
                    "Cannot verify the package process tree finished",
                ));
            }
            if accounting.ActiveProcesses == 0
                || ignored_browser.as_ref().is_some_and(|browser| {
                    // An incomplete snapshot stays conservative; only verified
                    // browser images can bypass the strict process count.
                    job_processes(job.0).is_ok_and(|processes| {
                        processes
                            .into_iter()
                            .all(|pid| ignored_job_process(job.0, pid, browser))
                    })
                })
            {
                return Ok(ExitStatus::from_raw(code));
            }
            if !announced {
                println!(
                    "{}",
                    crate::i18n::tr("Attente de la fin du désinstallateur…")
                );
                announced = true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};
    use windows_sys::Win32::Foundation::WAIT_TIMEOUT;

    struct Workspace(PathBuf);

    impl Workspace {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "SuperMenu-process-tree-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn script(&self, name: &str, source: &str) -> PathBuf {
            let path = self.0.join(name);
            // Windows PowerShell recognizes UTF-8 with a BOM, including paths
            // from non-ASCII user profiles and script arguments containing quotes.
            std::fs::write(&path, format!("\u{feff}{source}")).unwrap();
            path
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct WaitingChild {
        child: std::process::Child,
        release: PathBuf,
    }

    impl Drop for WaitingChild {
        fn drop(&mut self) {
            // Keep its script alive even if a parent assertion fails. This only
            // waits for the fixture we created; it never terminates any process.
            let _ = std::fs::write(&self.release, "release");
            let _ = self.child.wait();
        }
    }

    fn powershell() -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe")
    }

    fn literal(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\'', "''"))
    }

    #[test]
    fn root_exit_waits_for_its_delayed_descendant_and_preserves_argument_boundaries() {
        let workspace = Workspace::new();
        let marker = workspace.0.join("child finished 'é'.txt");
        let child = workspace.script(
            "delayed child 'é'.ps1",
            &format!(
                "Start-Sleep -Milliseconds 700; [IO.File]::WriteAllText({}, 'completed'); exit 17",
                literal(&marker)
            ),
        );
        let bootstrap = workspace.script(
            "bootstrap.ps1",
            &format!(
                "$start = [Diagnostics.ProcessStartInfo]::new({}); \
                 $start.UseShellExecute = $false; $start.CreateNoWindow = $true; \
                 $start.Arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"' + {} + '\"'; \
                 $child = [Diagnostics.Process]::Start($start); \
                 if ($child.HasExited) {{ exit 91 }}; exit 0",
                literal(&powershell()),
                literal(&child),
            ),
        );
        let script = bootstrap.to_str().unwrap();
        let result = run(
            &powershell(),
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                script,
            ],
        )
        .unwrap();
        assert_eq!(
            result.code(),
            Some(0),
            "child status must not replace the root status"
        );
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "completed");
    }

    #[test]
    fn preserves_a_nonzero_root_status_without_waiting_for_or_stopping_a_foreign_process() {
        let workspace = Workspace::new();
        let marker = workspace.0.join("foreign-finished.txt");
        let release = workspace.0.join("foreign-release.txt");
        let foreign_script = workspace.script(
            "foreign.ps1",
            &format!(
                "$deadline = [DateTime]::UtcNow.AddSeconds(30); \
                 while (-not [IO.File]::Exists({})) {{ \
                     if ([DateTime]::UtcNow -ge $deadline) {{ exit 81 }}; \
                     Start-Sleep -Milliseconds 10 \
                 }}; [IO.File]::WriteAllText({}, 'untouched'); exit 0",
                literal(&release),
                literal(&marker)
            ),
        );
        let failure_script = workspace.script("failed-command.ps1", "exit 23");
        let mut foreign = WaitingChild {
            child: Command::new(powershell())
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(foreign_script)
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .unwrap(),
            release,
        };
        let result = run(
            &powershell(),
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                failure_script.to_str().unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(result.code(), Some(23));
        assert!(
            foreign.child.try_wait().unwrap().is_none(),
            "an unrelated process is outside the job"
        );
        std::fs::write(&foreign.release, "release").unwrap();
        assert!(foreign.child.wait().unwrap().success());
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "untouched");
    }

    #[test]
    fn rejects_nul_and_relative_executables_before_creating_a_process() {
        for program in [Path::new("cmd.exe"), Path::new("C:\\invalid\0.exe")] {
            assert_eq!(
                run(program, &[]).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert_eq!(
            run(&powershell(), &["argument\0suffix"])
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn winget_execution_alias_accepts_the_native_spawn_path_without_managing_software() {
        let Some(local) = std::env::var_os("LOCALAPPDATA") else {
            return;
        };
        let alias = PathBuf::from(local).join("Microsoft/WindowsApps/winget.exe");
        if !alias.try_exists().unwrap_or(false) {
            // A clean Windows CI runner need not have App Installer provisioned.
            return;
        }
        assert!(run(&alias, &["--version"]).unwrap().success());
    }

    #[test]
    fn ignores_only_the_validated_browser_image_without_stopping_it() {
        let workspace = Workspace::new();
        // A tiny GUI-subsystem fixture acts as a browser. A console executable
        // would add its own conhost and does not model Chrome/Firefox correctly.
        // No installed browser, registry association or user profile is used.
        let browser = workspace.0.join("fixture-browser.exe");
        let source = workspace.0.join("browser-fixture.cs");
        std::fs::write(
            &source,
            r#"
using System;
using System.IO;
using System.Reflection;
using System.Threading;
class BrowserFixture {
    static int Main() {
        string root = Path.GetDirectoryName(Assembly.GetExecutingAssembly().Location);
        File.WriteAllText(Path.Combine(root, "browser-started.txt"), "ready");
        DateTime deadline = DateTime.UtcNow.AddSeconds(30);
        while (!File.Exists(Path.Combine(root, "browser-release.txt"))) {
            if (DateTime.UtcNow >= deadline) return 81;
            Thread.Sleep(10);
        }
        File.WriteAllText(Path.Combine(root, "browser-finished.txt"), "untouched");
        return 0;
    }
}
"#,
        )
        .unwrap();
        let compiler = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("Microsoft.NET/Framework64/v4.0.30319/csc.exe");
        let compiled = Command::new(compiler)
            .args(["/nologo", "/target:winexe"])
            .arg(format!("/out:{}", browser.display()))
            .arg(source)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "Cannot compile GUI fixture: {} {}",
            String::from_utf8_lossy(&compiled.stdout),
            String::from_utf8_lossy(&compiled.stderr)
        );
        let pid_file = workspace.0.join("browser-pid.txt");
        let started = workspace.0.join("browser-started.txt");
        let release = workspace.0.join("browser-release.txt");
        let finished = workspace.0.join("browser-finished.txt");
        let bootstrap = workspace.script(
            "browser-bootstrap.ps1",
            &format!(
                "$ErrorActionPreference = 'Stop'; \
                 $start = [Diagnostics.ProcessStartInfo]::new({}); \
                 $start.UseShellExecute = $false; $start.CreateNoWindow = $true; \
                 $browser = [Diagnostics.Process]::Start($start); \
                 [IO.File]::WriteAllText({}, [string]$browser.Id); \
                 $deadline = [DateTime]::UtcNow.AddSeconds(10); \
                 while (-not [IO.File]::Exists({})) {{ \
                     if ($browser.HasExited -or [DateTime]::UtcNow -ge $deadline) {{ exit 81 }}; \
                     Start-Sleep -Milliseconds 10 \
                 }}; exit 0",
                literal(&browser),
                literal(&pid_file),
                literal(&started),
            ),
        );
        struct ReleaseOnDrop {
            release: PathBuf,
            pid_file: PathBuf,
        }
        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                let _ = std::fs::write(&self.release, "release");
                let pid = std::fs::read_to_string(&self.pid_file)
                    .ok()
                    .and_then(|pid| pid.parse::<u32>().ok());
                if let Some(pid) = pid {
                    unsafe {
                        let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
                        if !process.is_null() {
                            let process = Handle(process);
                            WaitForSingleObject(process.0, 5000);
                        }
                    }
                }
            }
        }
        let release_on_drop = ReleaseOnDrop {
            release,
            pid_file: pid_file.clone(),
        };
        let result = run_with_ignored_browser(
            &powershell(),
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                bootstrap.to_str().unwrap(),
            ],
            Some(&browser),
        )
        .unwrap();
        assert_eq!(result.code(), Some(0));
        let pid: u32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        unsafe {
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            assert!(
                !process.is_null(),
                "the ignored browser was not killed or awaited"
            );
            let process = Handle(process);
            assert_eq!(WaitForSingleObject(process.0, 0), WAIT_TIMEOUT);
            assert!(!finished.exists());
            drop(release_on_drop);
            assert_eq!(WaitForSingleObject(process.0, 5000), WAIT_OBJECT_0);
        }
        assert_eq!(std::fs::read_to_string(finished).unwrap(), "untouched");
    }
}
