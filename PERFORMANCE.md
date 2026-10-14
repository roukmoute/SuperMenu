# Super Menu performance

**English** · [Français](PERFORMANCE.fr.md)

## Version 0.6.19

The local stable binary grows from **1 079 808 to 1 080 320 bytes (+512 bytes, +0.05%)**, with the same MSVC toolchain, Release profile and static CRT options. These measurements used the corrected source before changing the version stamp from 0.6.18 to 0.6.19.

Packaged applications retain their display names and gain a prepared product-name alias from their Shell identity; distinct launch targets no longer disappear when their display names coincide. Aliases reuse the existing search cache: no new `Item` field, dependency or system call while typing is added. The desktop Codex application remains separate from the Codex CLI installation suggestion.

### Comparative measurements

Historical eight-item performance fixture, light theme, DPI 96, actual parent `WM_PAINT`, three launches, thirty warmups and three hundred measurements per series. Order: reference / candidate / candidate / reference; shared machine. Median / p95 in milliseconds:

| Series | First internal draw, median | Parent painting | Search then painting |
| --- | ---: | ---: | ---: |
| Reference A | 80.189 | 2.064 / 3.654 | 3.566 / 5.576 |
| Candidate A | 58.717 | 1.558 / 2.123 | 3.696 / 5.858 |
| Candidate B | 70.233 | 1.796 / 2.799 | 3.760 / 5.954 |
| Reference B | 76.392 | 1.785 / 2.306 | 3.821 / 6.318 |

Menu search and painting are comparable across these series; warm private memory overlaps (**2.48–2.52 MiB**) and counters stay at **28 GDI, 26 USER**. Internal first-draw timing excludes process loading, the child EDIT and DWM presentation; Windows caches are uncontrolled. The inert fixture measures menu work, not installed application icons or a complete live catalogue.

The in-memory engine benchmark uses 20 000 entries, top nine results, one hundred warmups and four hundred samples per scenario. Median / p95 in milliseconds:

| Series | Simple index | Mixed index: files | Apps and cached package actions |
| --- | ---: | ---: | ---: |
| Reference A | 0.629 / 3.139 | 0.749 / 3.704 | 0.649 / 1.311 |
| Candidate A | 0.679 / 3.365 | 0.817 / 4.077 | 0.644 / 1.424 |
| Candidate B | 0.674 / 3.312 | 0.949 / 4.530 | 0.685 / 1.571 |
| Reference B | 0.672 / 3.233 | 0.793 / 3.984 | 0.896 / 2.414 |

The files scenario is slower in both initial candidate series; this is not reproduced in a final-version comparison: reference medians **0.509–0.543 ms**, candidate **0.501–0.505 ms** (p95 **2.894–2.959** versus **2.770–2.799 ms**). In that final comparison, app-action medians are **0.534–0.562** versus **0.527–0.528 ms**, while the simple-index candidate remains higher (**0.393–0.461** versus **0.362–0.373 ms**). The shared-machine series contain jitter, do not isolate its cause and prove neither a general speedup nor the absence of every regression. Indexing, external activation and cold startup are excluded from the engine benchmark.

**271 Rust tests** pass; three optional probes are ignored by default. The read-only real-catalogue Codex probe also passes. **1 300 Windows assertions** pass locally: 495 launcher, 278 About, 171 translations, 40 console, 170 updater and 146 NSIS installer assertions. The final version-stamped binary retains the measured size of **1 080 320 bytes**.

SHA-256 of the measured local candidate, before the version-stamp change: `1743652338965541A71A5193A1E34211CE7147362A9C14339E48441203CE02BE`.

## Version 0.6.18

Automatic updating shares the WinHTTP transport already used for translations. No runtime dependency is added. The local stable binary grows from **1 021 440 to 1 079 808 bytes**, an increase of **58 368 bytes (+5.71%, 57 KiB)**. It is about **1.03 MiB**; the “under 1 MiB” description of earlier versions therefore no longer applies to this build. The stable MSVC toolchain, Release profile and static CRT options are identical.

Network checks, version selection, downloading, hashing and language refresh run off the UI thread. The first check is delayed thirty seconds, then limited to once per day per channel and version. The update-application retry timer exists only when an update is ready. Activity started between preparation and shutdown defers replacement; terminal failures stop repeated attempts. The automatic mechanism interrupts no timer or operation in progress.

### Comparative measurements

Eight-item fixture, light theme, DPI 96, actual parent painting, three launches per series, thirty warmups and three hundred measurements. Order: reference / 0.6.18 / 0.6.18 / reference; shared machine. Median / p95 in milliseconds:

| Series | First internal draw, median | Parent painting | Search then painting |
| --- | ---: | ---: | ---: |
| Reference A | 89.688 | 2.276 / 3.090 | 4.888 / 7.026 |
| 0.6.18 A | 167.165 | 3.357 / 4.558 | 4.989 / 7.120 |
| 0.6.18 B | 106.390 | 2.553 / 3.442 | 5.237 / 7.745 |
| Reference B | 110.751 | 2.368 / 3.269 | 5.110 / 7.703 |

Search followed by painting remains close across these series. The first new series has slower first drawing and painting; the second does not reproduce this startup difference. These measurements prove neither a speedup nor the absence of every difference. Ranking and rendering are unchanged; this fixture prohibits networking and updating, so it compares the menu rather than a concurrent connection. Private memory overlaps (**2.54–2.61 MiB**) and counters are identical: **28 GDI, 26 USER**. First-draw timing is internal, with uncontrolled Windows caches, excluding initial process loading, the EDIT control and DWM presentation.

### Verification

Formatting and Clippy (`-D warnings`) are clean; **267 Rust tests**, two ignored optional probes and **1 275 Windows assertions** pass: 470 launcher, 278 About, 171 translations, 40 console, 170 updater and 146 installer. The real helper replaces two distinct x64 fixture executables, waits for a process that deliberately delays exit, actually restarts the new program, rejects invalid digests and identities, and preserves/restarts the old program when its target is locked. NSIS tests separately verify actual silent installation and preservation of settings/shortcuts. They use only test programs, directories and registrations.

The NSIS suite was rerun after a transient error when registering a fixture shortcut; all 146 assertions then passed without changing the installer.

SHA-256 of the measured local binary: `B697755B8797D92AA23F5AA523BFB85C69C4AA680426282E7FF90354E08CAAB7`.

## Version 0.6.17

### Background work and allocations

File or directory name changes are coalesced while the menu is hidden. They no longer repeatedly rebuild a 20 000-item index. On the next opening, the menu immediately shows its in-memory index, then starts an update on its existing worker. Start menu shortcut changes, installations/uninstallations and explicit refresh requests retain priority. Requests received during indexing or a modal dialog are retained.

File-root monitoring no longer requests content-write events: those writes do not change searched names. The Start menu keeps this monitoring for shortcut targets. Notification sources are merged when roots coincide. Rust `target` directories are excluded from the catalogue like `node_modules`, `.git`, `bin` and `obj`. Windows still reports their creation/deletion; these events can still trigger rebuilding while the menu remains visible.

Files are built directly into the next index. The separate intermediate array could reserve 32 768 structures of 216 bytes, adding **7 077 888 bytes** at peak. The next index's reserve is estimated from the previous one, then compacted after traversal to also cover a first launch or a growing catalogue. The deserialized cache is compacted before preparing its keys. Old strings are released on the worker after releasing the index lock, to avoid holding up concurrent search during their destruction.

Persistent identifiers, search strings, ranking and cache format are preserved. No additional thread, timer, dependency, DLL or focus mechanism is added.

### Memory: distinguishing counters

Task Manager's RAM figure represents resident memory. `PrivateUsage` measures committed private memory, which may be paged. These counters therefore describe different things ([Microsoft documentation](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters_ex)). A simultaneous observation on the development PC before the fix gave:

| Process | Private working set | Committed private memory |
| --- | ---: | ---: |
| Super Menu 0.6.16 | 30.55 MiB | 34.09 MiB |
| UltraEdit | 27.97 MiB | 110.63 MiB |

These are snapshots rather than a general comparison of two products. UltraEdit notably uses chunked reading for large files ([UltraEdit documentation](https://wiki.ultraedit.com/Settings:File_handling:Temporary_files)). Super Menu keeps its catalogue paths and keys in memory to respond to typing without disk access. No forced working-set purge or heap compaction is used to artificially lower a counter.

### Frozen-index comparison

A local copy of **20 318 items**, 8 547 083 bytes of JSON, serves both variants; its SHA-256 is `E17B5E72977A2E2A73595CD03B9C6612D87BFDF7D893B49EEE9AD3EBB0D36B22`. Personal data is not versioned. The optional Rust probe loads exactly this copy, measures before/after counters, then verifies materialized items and the first nine results of six searches. The item hash and results remain identical. Three launches per variant alternate, with the same stable toolchain and Release options.

| Median index-load measurement | Reference 0.6.16 | Vector compaction, unchanged strings |
| --- | ---: | ---: |
| Capacity / item count | 32 768 / 20 318 | 20 318 / 20 318 |
| Additional committed private memory | 18 804 736 bytes | 16 130 048 bytes |
| Additional resident working set | 16 048 128 bytes | 16 027 648 bytes |

The saving of **2 674 688 committed bytes** corresponds to unused vector capacity; this probe's resident reduction is only **20 480 bytes**. This is not a 2.7 MB reduction in Task Manager's displayed RAM.

Initial inter-process timings were affected by heavy external CPU load: their differences cannot attribute a slowdown to compaction. An additional measurement within a single process isolates `shrink_to_fit`: two warmup pairs, then eight pairs with/without compaction, alternating AB/BA order. Median cost **0.0226 ms**, maximum **0.0284 ms**; full restoration median **90.573 → 89.490 ms**, maxima **109.284 → 112.008 ms**. This series establishes no significant cost in total loading.

A `Box<str>` experiment for normalized keys saved about another 0.2 MB of resident memory, with timings too variable to conclude. It was rejected; `src/core.rs` is unchanged.

### Menu responsiveness

Same eight-item fixture, light theme, DPI 96, three launches per series, thirty warmups and three hundred actual parent-paint measurements. Order: reference / 0.6.17 / 0.6.17 / reference, after stopping this audit's builds. The machine remains shared with other tasks. Median / p95 in milliseconds:

| Series | First internal draw, median | Parent painting | Search then painting |
| --- | ---: | ---: | ---: |
| Reference A | 114.766 | 3.273 / 5.153 | 8.699 / 12.634 |
| 0.6.17 A | 110.886 | 3.699 / 6.081 | 9.696 / 16.201 |
| 0.6.17 B | 101.321 | 2.840 / 4.311 | 8.973 / 13.314 |
| Reference B | 124.126 | 3.058 / 4.674 | 8.809 / 12.619 |

Searches in the first new series are slower; the second approaches the references. These samples prove neither a speedup nor the absence of every latency difference, and ranking while typing has not changed. Counters remain at **28 GDI objects and 24 USER**, and private memory for this small fixture overlaps (2.48–2.51 MB versus 2.43–2.49 MB). First draws are internal, with uncontrolled Windows caches; DWM and the child EDIT control remain excluded.

### Size and verification

The local stable binary is **1 021 440 bytes**, versus **1 019 904 bytes** for 0.6.16, an increase of **1 536 bytes (+0.15%)**. The stable MSVC toolchain, Release profile and dependencies are identical. Probe and native-test code is excluded from the distributed binary.

Formatting and Clippy (`-D warnings`) are clean; **245 Rust tests**, two ignored optional measurements and **1 105 Windows assertions** pass (470 launcher, 278 About, 171 translations, 40 console, 146 installer). Tests monitor real temporary directories and windows belonging to their own processes. They verify source priority, creation/rename/deletion, absence of file-content notifications and opening after deferred changes.

SHA-256 of the measured local binary: `478A207028CAE8E8509F9311F8EE4CEAF120256895E454EA03B49BBFF85C133E`.

## Version 0.6.16

Steam games are an option disabled by default, available under **Settings → Installed Steam games**. Registry traversal already used for presence and uninstallation also supplies launch entries when enabled. When disabled, the branch returns before parsing Steam identifiers or allocating games; names needed for uninstallation stay in the shared inventory. No additional SDK, Steam library file, network, thread, timer or dependency is used.

Games retain a stable `steam:<AppID>` identifier and a `steam://run/<AppID>` target. Duplicates across registry views and existing shortcuts are avoided. The icon is read from the registry only on request for a visible result, on the existing icon worker. No field is added to the roughly 20 000 file items. A shared boolean guards ongoing publications: disabling the option removes games and purges icons without allowing an old worker to reintroduce them.

Generated entries are never saved in the main cache. The application and file cache remains compatible; games return during the application-indexing phase after enabling, without blocking menu opening. This avoids deserializing games at startup while the option is disabled.

The local stable binary is **1 019 904 bytes**, versus **1 015 808 bytes** for the 0.6.15 copy retained on this machine: **4 096 additional bytes (+0.40%)**, or 0.97 MiB. The Release profile, stable MSVC toolchain and dependencies are unchanged. The option's code remains in the EXE even while disabled; enabling determines the additional work and in-memory data.

### Menu measurements with the option disabled

Same eight-item fixture, light theme, DPI 96 and actual parent `WM_PAINT`. Each series has three launches, thirty warmups and three hundred measurements per operation. An initial reference / 0.6.16 / 0.6.16 / reference sequence was followed by the reverse 0.6.16 / reference / reference / 0.6.16 sequence to check order effects. Median / p95 in milliseconds:

| Series | First draw, internal median | Parent painting | Search then painting |
| --- | ---: | ---: | ---: |
| Reference A | 115.501 | 1.900 / 3.419 | 4.528 / 6.708 |
| 0.6.16 A | 90.561 | 2.097 / 3.111 | 5.598 / 9.084 |
| 0.6.16 B | 121.136 | 2.369 / 3.381 | 5.469 / 8.086 |
| Reference B | 104.566 | 2.041 / 2.764 | 5.099 / 7.981 |
| 0.6.16 C | 119.341 | 2.543 / 4.831 | 4.749 / 7.396 |
| Reference C | 118.231 | 3.032 / 4.307 | 5.192 / 7.872 |
| Reference D | 114.921 | 2.323 / 3.669 | 5.014 / 8.060 |
| 0.6.16 D | 113.411 | 2.278 / 3.556 | 5.299 / 7.833 |

The first comparison showed slower searches for the new version; the reverse sequence shows overlapping medians and a new series faster than the references. The PC was under heavy load from other builds. These data show no consistent shift and guarantee no particular latency. The ranking path on every keystroke has not changed. First draws are internal measurements with uncontrolled Windows caches; DWM presentation and child EDIT drawing remain excluded.

Counters remain at **28 GDI objects and 26 USER**. Fixture private memory overlaps: 2.58–2.68 MB for the reference and 2.60–2.64 MB for the new version. These fixtures check the menu with the option disabled; they do not measure the cost of a complete Steam library.

### Verification

**241 Rust tests**, one ignored benchmark and **1 093 Windows assertions** pass, including the installer. Eleven added tests cover the setting's optional nature, compatibility and saving, no allocation of the disabled game vector, bounded identifiers, Unicode titles, duplicates, icon paths, entry removal and cache exclusion.

An additional check with temporary data and read-only access to the real registry verifies both states of the native dialog and the absence of games in the cache. Artisan TD appears with its own icon and “Steam game” label only after enabling. The dialog was cancelled; no game was launched or uninstalled, and no personal setting was saved.

SHA-256 of the measured local binary:

```text
043F2343ECDC384437D96BCA9AEC3F862C9620F5625A6083E829422472B1A375
```

## Version 0.6.15

The menu prepares its complete image in a reusable GDI bitmap before a single copy to the window. Clipping the destination DC protects the EDIT control and its caret. The buffer is allocated on the first draw, reused at constant dimensions, replaced on resize and freed on destruction. Allocation or copying failure preserves functional drawing. No additional thread, timer, dependency or DLL is added.

The local stable binary is **1 015 808 bytes**, versus **1 014 784 bytes** for 0.6.14: **1 024 additional bytes (+0.10%)**. PE sections are aligned to 512 bytes. Shared handling of initial allocation and resizing was simplified; final size remains identical. Tests are excluded from the distributed binary. A 720 × 680-pixel surface at 32 bits represents **1 958 400 bytes**, about 1.96 MB of graphics memory managed by Windows.

### Actual menu painting

The optional `scripts/measure-performance.ps1 -WindowPaint` parameter uses the actual parent `WM_PAINT` through `RedrawWindow`, including the buffer and copy. The historical `WM_PRINTCLIENT` mode remains available by default. Both modes exclude child EDIT drawing and DWM pixel presentation. Clocks include inter-process Win32 messages and Windows work; first draws are timed in `ui::run` and remain repeated launches with uncontrolled caches.

Same stable toolchain, same Release profile, eight-item fixture, light theme, DPI 96, 720 × 680 window. Three launches, thirty warmups and three hundred measurements per operation in each series, in **0.6.15 / reference / reference / 0.6.15** order. Each cell gives median / p95 in milliseconds.

| Measurement | 0.6.15 A | Reference A | Reference B | 0.6.15 B |
| --- | ---: | ---: | ---: | ---: |
| Parent painting | 2.533 / 3.532 | 2.641 / 3.542 | 3.998 / 5.059 | 1.946 / 2.645 |
| Search then parent painting | 4.913 / 6.979 | 5.342 / 7.503 | 5.161 / 7.611 | 4.581 / 7.413 |
| First draw, internal median | 107.313 | 93.413 | 92.653 | 85.946 |

These final series show no painting slowdown. Windows variability is substantial: an initial exploratory comparison gave 6–9 ms for search followed by painting, versus 4.6–5.3 ms in the final series. These measurements therefore prove neither a systematic speedup nor guaranteed latency. The fix aims to remove visible intermediate images.

After warmup, GDI counters rise from **26 to 28**, while USER stays at **26**. Observed private memory is around 2.5 MB; it does not necessarily account for graphics surfaces allocated by Windows. Resize tests check that resources do not accumulate.

### Verification

Four native GDI tests verify that buffer drawing stays invisible before presentation, copied pixels are complete, clipping protects a child-control area, the buffer is reused and its handles are actually freed. The release test checks `GetObjectW`, because `GetObjectType` here retains the encoded type of an already deleted handle. Invalid parameters are rejected without allocation.

The launcher suite requires twenty-four actual paints with simulated indexing, icon notifications, navigation and six size changes; it checks text, EDIT selection, the selected result and GDI/USER counters. Validation: **230 Rust tests**, one ignored optional benchmark and **1 093 Windows assertions** (458 launcher, 278 About, 171 translations, 40 console, 146 installer). The launcher is also verified in the reported GIF's light theme.

SHA-256 of the measured final local binary:

```text
3D1DAC4EB45DF0A8B9B871CEE08A94850CE446E8FB74DB412AED0429646583B1
```

## Version 0.6.14

Navigation fix: automatic updates restore the selected result's identity before requesting visible icons. If the result disappears, its previous position is clamped to the new list; an actual query change always selects the first row. The typing path clones no identifier and does not scan results to preserve selection. Refreshes use a single identifier and traversal bounded by result count. No additional thread, timer, persistent state, import or dependency is added.

With the same stable toolchain and profile as the local 0.6.13 measurement, the binary is **1 014 784 bytes**, versus **1 014 272 bytes**: **512 additional bytes (+0.05%)**. The 0.6.13 profile and optimizations are retained; the nightly variant remains optional and experimental. This size comparison is not a new latency measurement.

The native test fails on the official 0.6.13 EXE at the first refresh and passes with the fix. It checks selection near the bottom of the home screen, a selected second application, three successive index notifications in each case, then returning to the first row when the query changes. Three Rust tests cover reordering, disappearing or limited results, and an empty list. Local validation: **226 Rust tests**, one ignored optional benchmark and **1 017 Windows assertions** (382 launcher, 278 About, 171 translations, 40 console, 146 installer).

SHA-256 of the measured local binary:

```text
9052964F0D8A194F03A2BADB0DFAB6B4EC7BC97090DD4D1D358A8656272420F4
```

## Version 0.6.13

The measured binary is **1 014 272 bytes (0.97 MiB)**, versus **1 210 368 bytes** for 0.6.12 rebuilt with the same toolchain: **196 096 bytes saved, or 16.2%**. Both versions use Rust 1.94.1, MSVC, the static C runtime and the unchanged Release profile (`opt-level = "z"`, full LTO, one codegen unit, stripped symbols, `panic = "abort"`). The reference is commit [`143f449`](https://github.com/roukmoute/SuperMenu/commit/143f449).

SHA-256 of the measured 0.6.13 binary:

```text
A213D90ABDD4599CB9682BDD36142CCEAE9BD8CB06C32A58B7547235B8CCC5B4
```

### Method

On an unstripped Release build, `cargo bloat` attributes 333 KiB of `.text` to Super Menu code, 311 KiB to the standard library and less than 45 KiB to dependencies; the 363 KiB of `.rdata` (tables and strings) cannot be attributed by symbol after LTO. Each component's marginal cost was therefore measured with small executables built using the same profile, against an empty 203 776-byte executable:

| Component | Isolated cost | Decision |
| --- | ---: | --- |
| `unicode-normalization` (NFD tables) | 126 KiB | replaced by Win32 |
| `std::process::Command` | 90 KiB | replaced by `CreateProcessW` |
| Float parsing and display | 36 KiB | retained: calculator |
| `str::to_uppercase` | 17 KiB | retained: persistent identifiers |
| `serde_json::Value` and `json!` | 11 KiB | replaced by typed structures |
| `sha2` | 9 KiB | retained: SHA-NI path |
| `windows` crate (COM) | 5 KiB | replaced by vtables on `windows-sys` |

### Retained changes

Each change was built and measured separately on version 0.6.12, then reviewed in three independent reviews (behavior parity, startup and typing latency, Win32/COM robustness) before integration.

| Change | Bytes saved |
| --- | ---: |
| Unicode decomposition through `NormalizeString` and `GetStringTypeW` instead of the `unicode-normalization` crate | 122 880 |
| Helper console and WinGet launched through `CreateProcessW` instead of `std::process::Command` | 54 272 |
| JSON reports serialized through typed structures instead of `json!` | 10 752 |
| `Debug` derived only for tests, nongeneric body for `Item::new`, unstable sorts on indistinguishable items | 3 072 |
| Manual COM bindings on `windows-sys` instead of the `windows` crate | 3 072 |

The integrated total (196 096 bytes) slightly exceeds the sum of isolated measurements: LTO removes more shared code once the `windows-core`, `windows-strings`, `windows-result` and `unicode-normalization` crates leave the dependency graph. Runtime dependencies are now limited to `serde`, `serde_json`, `sha2` and `windows-sys`; `unicode-normalization` remains a test dependency for parity checking.

Win32 normalization matches the crate for every character in the Latin-1 Supplement, Latin Extended A and B, Greek, Cyrillic and Latin Extended Additional blocks, and for fixture titles. Remaining differences are documented by tests: Indian and Burmese spacing marks that Windows classifies as neither diacritics nor vowels are retained, as are combining marks added after Unicode 5.1 and marks outside the Basic Multilingual Plane; three characters reclassified by Unicode are still removed. Noncharacters, accepted by NTFS but rejected by `NormalizeString`, are replaced with U+FFFD before the call. The ASCII path, predominant for application and file names, performs neither allocation nor a Win32 call. For an accented title, the Win32 version costs **1.3 µs** versus 0.95 µs for the crate (20 000 accented titles: 27 ms versus 19 ms, on the indexing thread); for an accented query, **0.7 µs** versus 0.2 µs, compared with 3 ms for search followed by drawing. The import table contains no new library: `NormalizeString` is resolved by `kernel32.dll`, and `oleaut32.dll` plus the `ntdll.dll` named-pipe calls used by `std::process` are no longer imported.

### Rejected tradeoffs

- A pure Rust SHA-256 implementation (−5 120 bytes) was implemented, reviewed and rejected: `sha2` uses SHA-NI instructions on recent processors, and `Usage::snapshot_for` hashes the index's 20 000 identifiers on the UI thread after each index load and launch. On this machine without SHA-NI, the implementations tie (29 ms for 20 000 identifiers); on a processor with SHA-NI, the software version would add around 15 ms.
- Replacing `to_uppercase` with `to_lowercase` (−16 896 bytes) would change the directory deduplication key from which persistent file identifiers and therefore `usage.json` keys are derived; existing usage counters would have been lost.
- The C runtime and standard library's mathematical and formatting tables (`pow`, `sin`, `log`, float display and parsing, about 40 KiB) serve the calculator and remain.
- Linker options (`/OPT:REF,ICF`) are already enabled; reducing section alignment or disabling dynamic relocation would break ASLR or security-tool compatibility, and no executable compression is added.

### Search engine

Same machine, **20 000-item** index, nine results, one hundred warmups then four hundred samples per scenario, in **reference / 0.6.13 / 0.6.13 / reference** order. Each cell gives **median / p95** in milliseconds.

| Scenario | Reference A | 0.6.13 A | 0.6.13 B | Reference B |
| --- | ---: | ---: | ---: | ---: |
| Generic index | 0.332 / 2.329 | 0.342 / 2.325 | 0.339 / 2.240 | 0.312 / 2.176 |
| Files in the mixed index | 0.621 / 2.968 | 0.467 / 3.083 | 0.497 / 2.772 | 0.390 / 2.683 |
| Cached applications and management actions | 0.519 / 0.990 | 0.501 / 0.973 | 0.500 / 0.906 | 0.493 / 0.895 |

Ranking has not changed: all four series overlap and differences stay within series-to-series variability. Win32 normalization applies only to non-ASCII queries and titles; this fixture contains none.

### Native menu

Three series per version, in alternating order (reference, 0.6.13, 0.6.13, reference, 0.6.13, reference), each with nine launches, then thirty warmups and three hundred measurements per operation. Isolated eight-item fixture, French, dark theme, DPI 96, 720 × 680 window, Windows 11 build 26200. Ranges cover medians from the three series.

| Measurement | Reference | 0.6.13 |
| --- | ---: | ---: |
| First draw, internal clock | 64.9–78.4 ms | 63.2–75.9 ms |
| Drawing only | 0.056–0.062 ms | 0.063–0.070 ms |
| Search then drawing | 3.15–3.32 ms | 3.05–3.78 ms |

The first 0.6.13 series is the slowest of all six (search followed by drawing 3.78 ms, p95 6.13 ms), and the next two are the fastest (3.05 and 3.09 ms): neither version differs beyond measurement noise. Private memory (2.70 MB), working set (15.9 MB) and GDI and USER counters (26 each) are identical. As with version 0.6.1, these figures do not indicate guaranteed latency and exclude cold startup.

### Full startup and accented queries

`first_frame_ms` excludes process creation and DLL loading. Here, an external clock measures the delay between `CreateProcess` and the first visible `SuperMenuWindow`, in fixture mode, fifteen alternating launches per version, warm Windows caches:

| External startup | Reference | 0.6.13 |
| --- | ---: | ---: |
| Median | 66.3 ms | 64.0 ms |
| p95 | 145.2 ms | 90.6 ms |
| Minimum | 61.1 ms | 54.7 ms |

The previous benchmarks use only ASCII queries, which do not go through the replaced normalization. The same menu measurement was therefore repeated with `tests/fixtures/launcher-accents.json` (twenty entries including twelve accented, Greek and Cyrillic titles) and accented queries (`@ éditeur`, `@ caméra`, `@ télé`, `@ désinst`, `@ ångs`, `@ no-match`, `/ vidéo`, `@ é`), two series per version in reference / 0.6.13 / 0.6.13 / reference order:

| Measurement | Reference | 0.6.13 |
| --- | ---: | ---: |
| First draw, internal clock | 65.8–76.4 ms | 72.7–75.4 ms |
| Drawing only | 0.062–0.063 ms | 0.066–0.070 ms |
| Accented search then drawing, median | 2.33–2.78 ms | 2.39–2.51 ms |
| Accented search then drawing, p95 | 5.47–5.67 ms | 6.10–6.21 ms |

Medians overlap; the 0.6.13 p95 values are about 0.5 ms higher in these two series, whereas earlier ASCII series placed them sometimes above and sometimes below the reference. The measured Win32 normalization cost (an extra 0.5 µs per accented query) cannot produce such a difference; it comes from the machine's scheduling noise, and these figures guarantee no particular latency.

### Optional nightly build

A substantial part of the binary remains the precompiled standard library and panic machinery, which a console-less executable cannot display. Rust nightly allows rebuilding the standard library with the project's size profile and aborting panics without formatting a message. Measurements on version 0.6.12, same machine:

| Build | Size |
| --- | ---: |
| Stable, project Release profile | 1 210 368 bytes |
| Nightly, `-Zbuild-std` with `optimize_for_size` | 999 936 bytes |
| Nightly, `-Zbuild-std` and `-Cpanic=immediate-abort` | 966 656 bytes |
| Nightly, `-Zbuild-std` with `optimize_for_size` and `-Cpanic=immediate-abort` | 937 984 bytes |

On version 0.6.13, the same recipe produces a **780 288-byte (0.74 MiB)** executable, 35.5% smaller than stable 0.6.12. It is available with `.\scripts\build.ps1 -Nightly` (the `nightly` toolchain and `rust-src` component installed by rustup) and is not used by continuous integration: it depends on unstable options, a panic no longer writes a diagnostic to the helper console, and the standard library's `optimize_for_size` variant chooses more compact, sometimes slower algorithms. The only benchmark recorded on this build, run by the script immediately after compilation, gives a 0.503 ms median on the generic index versus 0.312–0.342 ms for the stable build: an isolated, noisy measurement that nevertheless requires comparing the engine and menu with the above protocol before publishing with this recipe. Windows tests run on the executable produced this way.

### Reproducing and verifying

Commands and their limitations are described in [TESTING.md](TESTING.md#measuring-performance). To compare version 0.6.12 (`143f449`) and version 0.6.13 with the same toolchain:

```powershell
cargo build --release --locked
.\scripts\measure-performance.ps1 -Executable .\target\release\supermenu.exe -Report .\artifacts\performance.json
.\target\release\supermenu.exe --benchmark --report .\artifacts\benchmark.txt
```

Functional validation: **223 Rust tests**, including Unicode normalization parity, actual `.lnk` shortcut and Recycle Bin tests. On the binary measured here, **851 Windows assertions** were rerun locally (362 launcher, 278 About, 171 translations, 40 helper console). The **146 installer assertions** require an actual NSIS installation and were not rerun on this workstation; they were executed by continuous integration on the published commit ([run 38255429297](https://github.com/roukmoute/SuperMenu/actions/runs/38255429297)), bringing the validated total to 997. Warning-free Clippy and `rustfmt` are enforced.

## Version 0.6.1

The measured local binary is **1 154 560 bytes (1.10 MiB)**, versus **1 599 488 bytes** for version 0.6.0: **444 928 bytes saved, or 27.8%**. Both versions were rebuilt with Rust 1.94, MSVC and the same static C runtime options. The reference is commit [`1d68820`](https://github.com/roukmoute/SuperMenu/commit/1d68820f2915c35ba8f84ffb39d034b6b86755df), rather than an archive compiled elsewhere.

SHA-256 of the measured 0.6.1 binary:

```text
298FBD2A13A63367B347B6B9404E2E20F4A4048ADF57A12E92E6ACBF3D29A8D7
```

### Retained changes

The Release profile uses `opt-level = "z"`, full LTO, one codegen unit, stripped symbols and `panic = "abort"`. Profiles `3`, `s` and `z` were compared. Only the two ranking helpers `score` and `match_token` retain `inline(always)` to limit the size-oriented profile's cost on the search path. The executable is not compressed and adds no dependency.

The engine avoids unnecessary scans and allocations during ranking. The interface retains its fixed providers, translated text, icon keys and prepared UTF-16 arrays; drawing uses Windows GDI resources without recreating a brush for every rectangle. Translated formatting borrows its template instead of copying it. Cache restoration parses a read-only file view, and saving serializes borrowed items without copying the entire index.

### Search engine

Same machine, **20 000-item** index, nine results, one hundred warmups then four hundred samples per scenario. Compared runs use **reference / 0.6.1 / 0.6.1 / reference** order. Each cell gives **median / p95** in milliseconds; all four series are shown without selecting only the best run.

| Scenario | Reference A | 0.6.1 A | 0.6.1 B | Reference B |
| --- | ---: | ---: | ---: | ---: |
| Generic index | 0.391 / 2.872 | 0.518 / 3.869 | 0.391 / 2.394 | 0.447 / 3.005 |
| Files in the mixed index | 0.536 / 3.699 | 0.487 / 2.778 | 0.493 / 2.770 | 0.566 / 3.818 |
| Cached applications and management actions | 0.695 / 1.310 | 0.292 / 0.604 | 0.286 / 0.610 | 0.663 / 0.791 |

The mean of the two medians falls from **0.679 to 0.289 ms** for applications (−57.4%), and **0.551 to 0.490 ms** for files (−11%). The generic index rises from **0.419 to 0.455 ms** (+8.5%): the size/speed tradeoff therefore does not accelerate every case. Exploratory trials also showed greater variability; these figures guarantee no absolute latency. The diagnostic measures only in-memory ranking, excluding indexing, disk access, drawing and software activation.

### Cache

Release benchmark over **20 000 entries**, **4 417 969 bytes of JSON**, two warmups and sixteen samples per operation. Data and format remain identical.

| Operation | Median | p95 |
| --- | ---: | ---: |
| Restore with a buffered reader | 81.586 ms | 92.541 ms |
| Restore with a read-only file view | 47.410 ms | 65.791 ms |
| Serialize after a deep copy | 34.300 ms | 51.359 ms |
| Serialize borrowed items | 17.636 ms | 23.471 ms |

In this test, restoration reduces its median by **41.9%**, serialization by **48.6%**. The second comparison measures JSON preparation, excluding disk writing and synchronization. The mapped file avoids an additional heap copy of JSON; saving avoids temporarily doubling index data.

### Native menu

Two adjacent series per version, three launches each, then thirty warmups and three hundred measurements per operation. Isolated eight-item fixture, French, dark theme, DPI 96, 720 × 680 window, Windows 11 build 26200. The ranges below cover medians from the two series.

| Measurement | Reference | 0.6.1 |
| --- | ---: | ---: |
| First draw, internal clock | 116.6–157.2 ms | 71.8–96.7 ms |
| Drawing only | 0.171–0.172 ms | 0.119–0.149 ms |
| Search then drawing | 7.114–9.645 ms | 5.206–8.006 ms |

The p95 values varied more: one 0.6.1 run has a drawing p95 of **2.511 ms**, versus **0.537–0.652 ms** for the reference. Inferring a systematic display-latency improvement would be incorrect. First draws were generally faster in these series, without a cold-start guarantee.

`first_frame_ms` starts in `ui::run`: it excludes process creation, initial binary loading and DWM pixel presentation. Repeated launches generally use already warm Windows caches. Series include window-ownership checks, synchronous inter-process Win32 messages and caller-side `GdiFlush`; `WM_PRINTCLIENT` draws off-screen and excludes the child EDIT control. This small fixture's memory does not represent a real index's memory: no comparison is made with earlier full-PC measurements.

### Reproducing and verifying

Commands and tool limitations are described in [TESTING.md](TESTING.md#measuring-performance). Build both versions with the same toolchain, then retain JSON reports separately:

```powershell
.\scripts\measure-performance.ps1 -Executable .\target\release\supermenu.exe -Report .\artifacts\performance.json
cargo test --release --locked cache_performance_comparison -- --ignored --nocapture
```

The native report retains binary size and digest, fixture, settings and every sample. `--benchmark` produces engine measurements. Comparisons depend on the machine, compiler, DPI and load; no fixed hardware threshold is imposed in CI.

Functional validation: **174 Rust tests**, **one optional cache benchmark** and **755 Windows assertions** (306 menu, 278 About, 171 translations) pass. Tests use their own isolated files, registry entries and windows; no personal application was installed or uninstalled for these measurements.
