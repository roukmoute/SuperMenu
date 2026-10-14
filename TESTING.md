# Testing Super Menu on Windows

**English** · [Français](TESTING.fr.md)

Tests are versioned with the project. The same command runs on a development PC and in GitHub Actions:

```powershell
.\scripts\test.ps1
```

Requirements: Windows x64, PowerShell 7, Rust MSVC with `rustfmt` and `clippy`, and Visual Studio Build Tools C++ tools. The script checks formatting and Clippy, runs Rust tests, builds the Release binary, then tests the program's actual Win32 controls. An assertion failure makes the command fail.

Pictogram tests also check off-screen rendered pixels: nine home symbols, centered to within one pixel, at Windows scales of 100%, 125%, 150% and 200%. They check asymmetric bounds, invalid rectangles and restoration of GDI settings.

## Coverage

| Suite | Verified behavior |
| --- | --- |
| `cargo test --locked` | Search ranking and normalization, indexing and cache, real monitoring of creation/rename/deletion and shortcut targets, optional Steam configuration, game deduplication and cache exclusion, coverage of the 236 WinUtil entries, distinct editions and architectures, simulated Microsoft Store commands, proof of Chrome's absence despite a remaining registration, targeted reinstallation and executable confirmation, cleanup plans and data preservation, timers, launch state, icons, system information collection and isolated native controls. |
| `pwsh scripts/update-package-catalog.ps1 -Check` | Imported catalogue hash and size, deterministic regeneration of identifiers and sources, without downloading or installing. Also run by testing and distribution scripts. |
| `tests/windows/launcher.ps1` | Deferred file refreshes while the menu is hidden, opening with results immediately available, priority of application notifications and manual requests, search on a fixed index, default applications and explicit files, installation of absent Chrome with or without a prefix, absence of Telegram results in a Steam search, selection preservation after multiple index notifications (scrolled home and search), real WM_PAINT painting and resizing with EDIT text/selection preserved and bounded GDI/USER resources, reset on a new query, menu keyboard, prefix and caret, word/selection deletion with Ctrl+Backspace and undo. |
| `tests/windows/translations.ps1` | English menus and About window, unchanged application and package identities, real French/English selector, cache reuse without rewriting. |
| `tests/windows/about.ps1` | About window opening and content in both themes, version and counters, closing with Escape/cross/button, reopening the same window, resizing, scrolling, accessible buttons and bounded GDI/USER resources. |
| `tests/windows/helper-console.ps1` | Automatic closure after an installation or uninstallation error, preserved exit code, local report, non-waiting diagnostic and absence of blocking QuickEdit selection. Test requests fail before any WinGet call. |
| `tests/windows/updater.ps1` | Actual update helper, validation of SHA-256 digests, size, Windows x64 image, parent process identity and window owner, rejection of foreign jobs and ambiguous arguments, waiting for full parent exit before atomic replacement, bounded deferral if the parent refuses, background restart and data preservation. A locked target preserves and restarts the old binary. No download or user software is used. |
| `tests/windows/setup.ps1` | Real NSIS installation, shortcuts and Windows registration, optional settings, update while the test instance runs, rejection of unknown processes, uninstallation and data preservation. Every installer uses an isolated test name and directory. |

Windows suites drive the built executable, observe its actual windows and results, and fail on a regression. They do not recalculate ranking in the script. They also verify names and results of every `--smoke-test` check; a zero exit code alone is insufficient.

The native notification-icon test checks its actual rendering with `DrawIconEx` at 16, 20, 24 and 32 pixels: transparent background in light and dark themes, centered grid, transparency mask and resource release. To export BMP renders, optionally set `SUPER_MENU_ICON_TEST_DIR` before `cargo test`; this instrumentation remains absent from the distributed binary.

Launch feedback is also drawn into a native bitmap: the test verifies the application icon's centering and transparency in both themes, its ownership independent of the cache, replacement without extending the duration and no reappearance after expiry. `SUPER_MENU_LAUNCH_TEST_DIR` optionally exports these renders during tests.

Uninstallation-tracking tests use dummy PowerShell processes and a fake GUI browser compiled for the fixture: a launcher exits before its delayed child, which must write its marker before the command returns. They also check the launcher's exit code, no interference with an unrelated process and exclusion of only the validated browser, without closing it. The WinGet alias test runs only --version. StreamFab cleanup uses a fixture directory with its dated name, binaries and personal files: only attributable binaries become candidates. These tests launch, stop or uninstall no actual StreamFab installation.

## Data and isolation

`tests/fixtures/launcher.json` contains dummy applications and files. `--smoke-test --smoke-fixture <fichier>` restores their search data and replaces PC indexing. This mode uses default settings and an empty launch history. It neither reads nor writes the personal index or configuration, installs no global shortcut and starts no directory watchers. Activation of external software, files, web links, clipboard and system actions is blocked in this mode; tests use internal menu actions.

Every process and temporary directory belongs to its test. The support code verifies every window's PID before controlling it and bounds Windows-message timeouts. It does not close an already running Super Menu instance. No user software is installed or uninstalled. Some Rust tests create disposable files or registry keys and check the Recycle Bin using their own files; they clean up only those resources.

The update suite compiles two small disposable Windows x64 executables with Windows' .NET Framework compiler. They substitute only for the fixture's old and new application; the helper being run comes from the real Super Menu binary. A hidden window receives the validation message, then the test checks that the file remains unchanged while its process is active. Restarting writes a different marker depending on the version actually executed. Every process receives its own temporary `LOCALAPPDATA`; no global mutex, keyboard shortcut, user index or Internet connection is used. Fixture executables are neither distributed nor included in Super Menu.

Native tests require no physical keyboard, desktop focus, particular graphics card or precise number of installed applications. Available system information varies in a virtual machine: an `Indisponible` value is accepted. Total memory and resolutions are not fixed in assertions.

## Reports and diagnostics

JSON reports are in `artifacts/test-results/` and retained as the **SuperMenu-test-results** artifact in GitHub Actions, even after a failure. They detail assertions, observed results and diagnostics. This directory is ignored by Git.

To also export captures of test windows on an interactive Windows desktop:

```powershell
.\scripts\test.ps1 -Capture
```

Captures are diagnostic aids. CI does not compare pixels, fonts or performance against a hardware-dependent threshold. A benchmark report is produced separately by `scripts/build.ps1`.

To reuse a binary built from this checkout without repeating Rust checks:

```powershell
.\scripts\test.ps1 -Executable .\target\release\supermenu.exe -SkipRustChecks
```

A suite can also be run separately:

```powershell
.\tests\windows\launcher.ps1 -Executable .\target\release\supermenu.exe
.\tests\windows\about.ps1 -Executable .\target\release\supermenu.exe
.\tests\windows\translations.ps1 -Executable .\target\release\supermenu.exe
.\tests\windows\helper-console.ps1 -Executable .\target\release\supermenu.exe
.\tests\windows\updater.ps1 -Executable .\target\release\supermenu.exe
.\tests\windows\setup.ps1 -Executable .\target\release\supermenu.exe
```

An older published executable may lack diagnostics added since publication: build the checkout being tested. `--smoke-fixture`, the result-snapshot message and the Ctrl+Backspace diagnostic are reserved for test mode. The keyboard diagnostic modifies only the input queue state of this process's UI thread, then restores it; it does not simulate a global keypress.

An optional read-only probe checks launch search against the current Windows account's real Shell catalog. The Codex desktop application must already be installed for this example; the probe is ignored by default, prints matching names and targets, and launches no application:

```powershell
$env:SUPER_MENU_APP_QUERY = 'codex'
$env:SUPER_MENU_APP_EXPECTED_TARGET = 'shell:AppsFolder\OpenAI.Codex_2p2nqsd0c76g0!App'
cargo test --release --locked installed_shell_application_search_probe -- --ignored --nocapture
```

## Measuring performance

Latency measurements are optional and separate from functional assertions. To compare two executables on the same PC:

```powershell
.\scripts\measure-performance.ps1 -Executable .\target\release\supermenu.exe -Report .\artifacts\performance.json
```

The optional `-WindowPaint` parameter measures the actual parent `WM_PAINT` through `RedrawWindow`, including buffering and copying to the window. The default mode retains the off-screen `WM_PRINTCLIENT` diagnostic. Both modes exclude drawing the child EDIT control and DWM presentation; the report identifies the mode used.

```powershell
.\scripts\measure-performance.ps1 -Executable .\target\release\supermenu.exe -WindowPaint -Theme light -Report .\artifacts\window-paint.json
```

The script uses `tests/fixtures/launcher-performance.json`, the eight historical entries and their dummy `fixture://` targets, to compare latency without loading installed application icons. It launches nine successive processes, then measures drawing alone and search followed by drawing in the last process: thirty warmups and three hundred samples for each series. `-StartupRuns`, `-WarmupRounds`, `-Rounds`, `-Theme`, `-Fixture` and `-Queries` reproduce other configurations; a fixture is limited to 256 dummy entries. Default queries are ASCII; `tests/fixtures/launcher-accents.json` adds twelve accented, Greek and Cyrillic titles to this performance fixture, and `-Queries '@ éditeur','@ caméra','@ télé','@ désinst','@ ångs','@ no-match','/ vidéo','@ é'` measures the non-ASCII normalization path. The first control query remains `@ steam`, so the fixture must keep Steam entries. Every HWND is checked against its PID before sending a message. No software is activated, installed or uninstalled, and no language download is requested.

The functional fixture `tests/fixtures/launcher.json` adds the ChatGPT/Codex Shell identities: the native suite verifies aliases and distinct targets without launching an application; it remains separate from the performance fixture.

Series are timed in a C# loop, excluding PowerShell dispatch overhead. `WM_SETTEXT` passes through the actual EDIT control and updates results; `WM_PRINTCLIENT` draws the menu into a reused off-screen bitmap, followed by caller-side `GdiFlush`. A snapshot also verifies the query actually processed. The JSON report contains the binary's size and SHA-256, fixture digest, theme, language, DPI, dimensions, samples, medians and p95, private and resident memory, and GDI/USER counters.

To interpret the report:

- `first_frame_ms` comes from the internal clock started in `ui::run`, up to the first completed `WM_PAINT`. It excludes process creation, initial binary loading and DWM pixel presentation.
- Repeated launches generally use already warm Windows file and DLL caches. Their state is uncontrolled; this is not a cold-PC startup measurement.
- Menu series include window-ownership checks and synchronous inter-process Win32 messages. Off-screen drawing excludes the child EDIT control and on-screen DWM presentation.
- The small fixture measures menu work. The `--benchmark` diagnostic separately measures ranking on a 20 000-item index; it does not measure opening software or disk access.

Compare executables using the same fixture, options, DPI and similar machine conditions. The tools impose no arbitrary performance threshold and are not routinely run by CI.

An additional benchmark measures cache restoration and serialization over 20 000 entries. It compares buffered-reader JSON loading with a read-only file view, then serialization of a deep copy with serialization of borrowed items:

```powershell
cargo test --release --locked cache_performance_comparison -- --ignored --nocapture
```

This `#[ignore]` test uses a temporary file, two warmups and sixteen samples per operation. It is absent from ordinary `cargo test` runs, and its code is not included in the distributed executable. Its figures remain sensitive to the compiler, build profile, machine and Windows caches. It does not replace functional tests for the cache's format, compatibility and release.

An optional probe measures a frozen cache's memory. Copy the same French `index.json` outside the active data directory, then supply absolute paths:

```powershell
$env:SUPER_MENU_MEMORY_FIXTURE = 'C:\mesures\index.json'
$env:SUPER_MENU_MEMORY_REPORT = 'C:\mesures\memoire.json'
cargo test --release --locked frozen_index_memory_measurement -- --ignored --test-threads=1
```

`tests/performance/memory.rs` records `PrivateUsage`, total resident working set, vector capacity, string sizes, restoration time, item hash and the results of six searches. This probe's language is `fr`; a cache in another language would be rejected. It starts no menu or watcher, does not reindex the PC and does not modify the supplied file. Compare the same bytes between versions and alternate launches. The report contains identifiers from the supplied cache: keep this data local when it describes personal files. The test is ignored by default and its code remains excluded from the distributed executable. Committed and resident counters must be interpreted separately; temporary Windows memory and other PC tasks can affect timings.

## Adding a regression

For an engine bug, add a Rust test near the relevant module with the failing case. For an interaction bug, add an assertion in the appropriate Windows suite and fixture data if needed. Verify that the test fails when behavior is deliberately regressed, then restore the code. Keep tests independent of the Windows profile, actual applications and PC execution time.

Translation tests copy the repository's English pack into their temporary cache. They do not contact the Internet. Rust tests also validate the actual JSON pack, text parameters, limits, atomic saving, recovery after errors and absence of downloads when the cache is valid. `--smoke-language` and `--smoke-language-dir` require a fixture; the latter identifies an isolated absolute cache. `--smoke-first-run-language` checks the selector before showing the menu without saving personal settings.

To optionally verify actual pack downloading from the version published on `main` (Internet access required, never run by CI):

```powershell
.\tests\windows\translations.ps1 -Executable .\target\release\supermenu.exe -Download
```
