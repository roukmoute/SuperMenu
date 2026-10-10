//! A small, read-only system snapshot, collected only when About is opened.
//! It uses Windows APIs and an explicit registry allowlist: no processes, WMI,
//! network requests, account names, serial numbers or persistent background work.

use crate::i18n::{fmt, tr};
use std::collections::HashSet;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::{ERROR_SUCCESS, FILETIME, SYSTEMTIME},
    Graphics::Gdi::{
        EnumDisplayDevicesW, EnumDisplaySettingsW, DEVMODEW, DISPLAY_DEVICEW,
        DISPLAY_DEVICE_MIRRORING_DRIVER, ENUM_CURRENT_SETTINGS,
    },
    Storage::FileSystem::GetDiskFreeSpaceExW,
    System::{
        LibraryLoader::{GetModuleHandleW, GetProcAddress},
        Registry::{
            RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
            RRF_SUBKEY_WOW6464KEY, RRF_ZEROONFAILURE,
        },
        SystemInformation::{
            GetNativeSystemInfo, GetSystemWindowsDirectoryW, GetTickCount64, GlobalMemoryStatusEx,
            IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64, IMAGE_FILE_MACHINE_I386,
            MEMORYSTATUSEX, OSVERSIONINFOEXW, PROCESSOR_ARCHITECTURE_AMD64,
            PROCESSOR_ARCHITECTURE_ARM64, PROCESSOR_ARCHITECTURE_INTEL, SYSTEM_INFO,
        },
        Threading::{GetActiveProcessorCount, GetCurrentProcess, ALL_PROCESSOR_GROUPS},
        Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTimeEx},
    },
};

const UNAVAILABLE: &str = "Indisponible";
const WINDOWS_KEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
const BIOS_KEY: &str = "HARDWARE\\DESCRIPTION\\System\\BIOS";
const CPU_KEY: &str = "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0";
const VALUE_LIMIT: usize = 512;

#[link(name = "ntdll")]
extern "system" {
    fn RtlGetVersion(information: *mut OSVERSIONINFOEXW) -> i32;
}

#[derive(Debug)]
pub struct Row {
    pub label: String,
    pub value: String,
}

#[derive(Debug)]
pub struct Snapshot {
    pub hardware: Vec<Row>,
    pub windows: Vec<Row>,
    pub activity: Vec<Row>,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn clean(text: &str) -> Option<String> {
    let text: String = text
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    let mut characters = text.chars();
    let mut limited: String = characters.by_ref().take(VALUE_LIMIT).collect();
    if characters.next().is_some() {
        limited.pop();
        limited.push('…');
    }
    Some(limited)
}

fn row(label: &str, value: Option<String>) -> Row {
    Row {
        label: tr(label),
        value: value
            .and_then(|value| clean(&value))
            .unwrap_or_else(|| tr(UNAVAILABLE)),
    }
}

fn registry_string(key: &str, name: &str) -> Option<String> {
    let key = wide(key);
    let name = wide(name);
    let flags = RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY | RRF_ZEROONFAILURE;
    unsafe {
        let mut bytes = 0;
        if RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            null_mut(),
            &mut bytes,
        ) != ERROR_SUCCESS
            || !(2..=65_536).contains(&bytes)
            || bytes % 2 != 0
        {
            return None;
        }
        let mut buffer = vec![0u16; bytes as usize / 2];
        if RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        ) != ERROR_SUCCESS
            || bytes as usize > buffer.len() * 2
            || bytes % 2 != 0
        {
            return None;
        }
        clean(&String::from_utf16_lossy(
            buffer[..bytes as usize / 2]
                .split(|character| *character == 0)
                .next()
                .unwrap_or_default(),
        ))
    }
}

fn registry_dword(key: &str, name: &str) -> Option<u32> {
    let key = wide(key);
    let name = wide(name);
    let mut value = 0u32;
    let mut bytes = size_of::<u32>() as u32;
    (unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD | RRF_SUBKEY_WOW6464KEY | RRF_ZEROONFAILURE,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut bytes,
        ) == ERROR_SUCCESS
            && bytes == size_of::<u32>() as u32
    })
    .then_some(value)
}

fn firmware_value(name: &str) -> Option<String> {
    let value = registry_string(BIOS_KEY, name)?;
    let lower = value.to_lowercase();
    (![
        "system product name",
        "system manufacturer",
        "to be filled by o.e.m.",
        "to be filled by oem",
        "default string",
        "not applicable",
        "not specified",
        "unknown",
        "none",
    ]
    .contains(&lower.as_str()))
    .then_some(value)
}

fn machine() -> Option<String> {
    let manufacturer =
        firmware_value("SystemManufacturer").or_else(|| firmware_value("BaseBoardManufacturer"));
    let model = firmware_value("SystemProductName").or_else(|| firmware_value("BaseBoardProduct"));
    match (manufacturer, model) {
        (Some(manufacturer), Some(model)) => {
            if model
                .to_lowercase()
                .starts_with(&manufacturer.to_lowercase())
            {
                Some(model)
            } else {
                Some(format!("{manufacturer} · {model}"))
            }
        }
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn cpu() -> Option<String> {
    let name = registry_string(CPU_KEY, "ProcessorNameString");
    let count = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) };
    match (name, count) {
        (Some(name), 0) => Some(name),
        (Some(name), count) => Some(fmt(
            "{name} · {count} processeurs logiques",
            &[("name", name), ("count", count.to_string())],
        )),
        (None, 0) => None,
        (None, count) => Some(fmt(
            "{count} processeurs logiques",
            &[("count", count.to_string())],
        )),
    }
}

fn graphics() -> Option<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for index in 0..64 {
        let mut device: DISPLAY_DEVICEW = unsafe { zeroed() };
        device.cb = size_of::<DISPLAY_DEVICEW>() as u32;
        if unsafe { EnumDisplayDevicesW(null(), index, &mut device, 0) } == 0 {
            break;
        }
        if device.StateFlags & DISPLAY_DEVICE_MIRRORING_DRIVER != 0 {
            continue;
        }
        let text = String::from_utf16_lossy(
            device
                .DeviceString
                .split(|character| *character == 0)
                .next()
                .unwrap_or_default(),
        );
        if let Some(name) = clean(&text) {
            if seen.insert(name.to_lowercase()) {
                names.push(name);
            }
        }
    }
    (!names.is_empty()).then(|| names.join(" / "))
}

fn display() -> Option<String> {
    let mut mode: DEVMODEW = unsafe { zeroed() };
    mode.dmSize = size_of::<DEVMODEW>() as u16;
    if unsafe { EnumDisplaySettingsW(null(), ENUM_CURRENT_SETTINGS, &mut mode) } == 0
        || mode.dmPelsWidth == 0
        || mode.dmPelsHeight == 0
    {
        return None;
    }
    let resolution = format!("{} × {}", mode.dmPelsWidth, mode.dmPelsHeight);
    Some(if mode.dmDisplayFrequency > 1 {
        format!("{resolution} · {} Hz", mode.dmDisplayFrequency)
    } else {
        resolution
    })
}

fn format_bytes(bytes: u64) -> String {
    for (threshold, unit) in [
        (1u64 << 40, "Tio"),
        (1u64 << 30, "Gio"),
        (1u64 << 20, "Mio"),
        (1u64 << 10, "Kio"),
    ] {
        if bytes >= threshold {
            let amount = format!("{:.1}", bytes as f64 / threshold as f64);
            let (whole, fraction) = amount.split_once('.').unwrap_or((&amount, "0"));
            return fmt(
                "{whole},{fraction} {unit}",
                &[
                    ("whole", whole.into()),
                    ("fraction", fraction.into()),
                    ("unit", tr(unit)),
                ],
            );
        }
    }
    fmt(
        if bytes == 1 {
            "{bytes} octet"
        } else {
            "{bytes} octets"
        },
        &[("bytes", bytes.to_string())],
    )
}

fn format_capacity(used: u64, total: u64) -> Option<String> {
    if total == 0 {
        return None;
    }
    let used = used.min(total);
    let percent = (used as f64 / total as f64 * 100.0).round() as u32;
    Some(format!(
        "{} / {} ({percent} %)",
        format_bytes(used),
        format_bytes(total)
    ))
}

fn memory() -> Option<String> {
    let mut status: MEMORYSTATUSEX = unsafe { zeroed() };
    status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return None;
    }
    format_capacity(
        status.ullTotalPhys.saturating_sub(status.ullAvailPhys),
        status.ullTotalPhys,
    )
}

fn system_disk() -> Option<String> {
    let mut directory = [0u16; 32_768];
    let length =
        unsafe { GetSystemWindowsDirectoryW(directory.as_mut_ptr(), directory.len() as u32) };
    if length == 0 || length as usize >= directory.len() {
        return None;
    }
    let mut total = 0;
    let mut free = 0;
    if unsafe { GetDiskFreeSpaceExW(directory.as_ptr(), null_mut(), &mut total, &mut free) } == 0 {
        return None;
    }
    format_capacity(total.saturating_sub(free), total)
}

fn os_version() -> Option<OSVERSIONINFOEXW> {
    let mut version: OSVERSIONINFOEXW = unsafe { zeroed() };
    version.dwOSVersionInfoSize = size_of::<OSVERSIONINFOEXW>() as u32;
    (unsafe { RtlGetVersion(&mut version) } == 0).then_some(version)
}

fn edition_name(product: Option<String>, build: Option<u32>, workstation: bool) -> Option<String> {
    let product = product?;
    // ProductName remains "Windows 10" on some Windows 11 installations.
    // Server builds also pass 22000, so the product type must be a workstation.
    if workstation && build.is_some_and(|build| build >= 22_000) {
        if let Some(edition) = product.strip_prefix("Windows 10") {
            return Some(format!("Windows 11{edition}"));
        }
    }
    Some(product)
}

fn architecture() -> Option<String> {
    unsafe {
        // GetNativeSystemInfo can describe emulation on ARM64. Query the native
        // machine when this newer API is available, without raising the minimum
        // Windows version through a mandatory import.
        type QueryMachine = unsafe extern "system" fn(
            windows_sys::Win32::Foundation::HANDLE,
            *mut u16,
            *mut u16,
        ) -> i32;
        let library = GetModuleHandleW(wide("kernel32.dll").as_ptr());
        if !library.is_null() {
            if let Some(function) = GetProcAddress(library, c"IsWow64Process2".as_ptr().cast()) {
                let query: QueryMachine = std::mem::transmute(function);
                let mut process = 0;
                let mut native = 0;
                if query(GetCurrentProcess(), &mut process, &mut native) != 0 {
                    if let Some(name) = match native {
                        IMAGE_FILE_MACHINE_AMD64 => Some("x64 · 64 bits"),
                        IMAGE_FILE_MACHINE_ARM64 => Some("ARM64 · 64 bits"),
                        IMAGE_FILE_MACHINE_I386 => Some("x86 · 32 bits"),
                        _ => None,
                    } {
                        return Some(tr(name));
                    }
                }
            }
        }
        let mut system: SYSTEM_INFO = zeroed();
        GetNativeSystemInfo(&mut system);
        match system.Anonymous.Anonymous.wProcessorArchitecture {
            PROCESSOR_ARCHITECTURE_AMD64 => Some(tr("x64 · 64 bits")),
            PROCESSOR_ARCHITECTURE_ARM64 => Some(tr("ARM64 · 64 bits")),
            PROCESSOR_ARCHITECTURE_INTEL => Some(tr("x86 · 32 bits")),
            _ => None,
        }
    }
}

fn format_uptime(milliseconds: u64) -> String {
    let minutes = milliseconds / 60_000;
    let hours = minutes / 60;
    let days = hours / 24;
    if days > 0 {
        fmt(
            "{days} j {hours} h {minutes} min",
            &[
                ("days", days.to_string()),
                ("hours", format!("{:02}", hours % 24)),
                ("minutes", format!("{:02}", minutes % 60)),
            ],
        )
    } else if hours > 0 {
        fmt(
            "{hours} h {minutes} min",
            &[
                ("hours", hours.to_string()),
                ("minutes", format!("{:02}", minutes % 60)),
            ],
        )
    } else if minutes > 0 {
        fmt("{minutes} min", &[("minutes", minutes.to_string())])
    } else {
        tr("Moins d’une minute")
    }
}

fn format_date(date: &SYSTEMTIME) -> Option<String> {
    let months = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];
    let month = months.get(usize::from(date.wMonth).checked_sub(1)?)?;
    if !(1..=31).contains(&date.wDay) || date.wHour > 23 || date.wMinute > 59 {
        return None;
    }
    Some(fmt(
        "{day} {month} {year} · {hour} h {minute}",
        &[
            ("day", date.wDay.to_string()),
            ("month", tr(month)),
            ("year", date.wYear.to_string()),
            ("hour", format!("{:02}", date.wHour)),
            ("minute", format!("{:02}", date.wMinute)),
        ],
    ))
}

fn installation_date(seconds: u32) -> Option<String> {
    if seconds == 0 {
        return None;
    }
    // InstallDate is Unix seconds; FILETIME uses 100 ns since 1601.
    let ticks = (u64::from(seconds) + 11_644_473_600) * 10_000_000;
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc: SYSTEMTIME = unsafe { zeroed() };
    let mut local: SYSTEMTIME = unsafe { zeroed() };
    if unsafe { FileTimeToSystemTime(&file_time, &mut utc) } == 0 {
        return None;
    }
    if unsafe { SystemTimeToTzSpecificLocalTimeEx(null(), &utc, &mut local) } == 0 {
        return None;
    }
    format_date(&local)
}

pub fn collect() -> Snapshot {
    let version = os_version();
    let build = version
        .as_ref()
        .map(|version| version.dwBuildNumber)
        .or_else(|| {
            registry_string(WINDOWS_KEY, "CurrentBuildNumber")?
                .parse()
                .ok()
        });
    let product = edition_name(
        registry_string(WINDOWS_KEY, "ProductName"),
        build,
        version
            .as_ref()
            .is_some_and(|version| version.wProductType == 1),
    );
    let detailed_build = version.as_ref().map(|version| {
        let base = format!(
            "{}.{}.{}",
            version.dwMajorVersion, version.dwMinorVersion, version.dwBuildNumber
        );
        registry_dword(WINDOWS_KEY, "UBR")
            .map_or(base.clone(), |revision| format!("{base}.{revision}"))
    });
    Snapshot {
        hardware: vec![
            row("Machine", machine()),
            row("Processeur", cpu()),
            row("Graphiques", graphics()),
            row("Écran", display()),
            row("Mémoire", memory()),
            row("Disque système", system_disk()),
        ],
        windows: vec![
            row("Édition", product),
            row(
                "Version",
                registry_string(WINDOWS_KEY, "DisplayVersion")
                    .or_else(|| registry_string(WINDOWS_KEY, "ReleaseId")),
            ),
            row("Build", detailed_build),
            row("Architecture", architecture()),
        ],
        activity: vec![
            row(
                "Allumé depuis",
                Some(format_uptime(unsafe { GetTickCount64() })),
            ),
            row(
                "Windows installé le",
                registry_dword(WINDOWS_KEY, "InstallDate").and_then(installation_date),
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn french_sizes_and_usage_are_readable_and_bounded() {
        assert_eq!(format_bytes(1), "1 octet");
        assert_eq!(format_bytes(1024), "1,0 Kio");
        assert_eq!(format_bytes(3 * (1 << 30) / 2), "1,5 Gio");
        assert_eq!(
            format_capacity(8 << 30, 16 << 30).as_deref(),
            Some("8,0 Gio / 16,0 Gio (50 %)")
        );
        assert_eq!(
            format_capacity(200, 100).as_deref(),
            Some("100 octets / 100 octets (100 %)")
        );
        assert!(format_capacity(0, 0).is_none());
        assert!(format_capacity(u64::MAX, u64::MAX)
            .unwrap()
            .contains("100 %"));
    }

    #[test]
    fn uptime_and_calendar_use_french_units() {
        assert_eq!(format_uptime(59_999), "Moins d’une minute");
        assert_eq!(format_uptime(120_000), "2 min");
        assert_eq!(
            format_uptime((3 * 24 * 60 + 2 * 60 + 5) * 60_000),
            "3 j 02 h 05 min"
        );
        let date = SYSTEMTIME {
            wYear: 2026,
            wMonth: 10,
            wDay: 11,
            wHour: 9,
            wMinute: 7,
            ..unsafe { zeroed() }
        };
        assert_eq!(
            format_date(&date).as_deref(),
            Some("11 octobre 2026 · 09 h 07")
        );
        assert!(format_date(&SYSTEMTIME { wMonth: 13, ..date }).is_none());
        assert!(installation_date(0).is_none());
        assert!(installation_date(1_700_000_000).is_some());
    }

    #[test]
    fn windows_11_fix_does_not_rename_server_or_old_workstation() {
        assert_eq!(
            edition_name(Some("Windows 10 Pro".into()), Some(26_100), true).as_deref(),
            Some("Windows 11 Pro")
        );
        assert_eq!(
            edition_name(Some("Windows 10 Pro".into()), Some(19_045), true).as_deref(),
            Some("Windows 10 Pro")
        );
        assert_eq!(
            edition_name(
                Some("Windows Server 2025 Standard".into()),
                Some(26_100),
                false
            )
            .as_deref(),
            Some("Windows Server 2025 Standard")
        );
        assert_eq!(
            edition_name(Some("Windows 10 Pro".into()), Some(26_100), false).as_deref(),
            Some("Windows 10 Pro")
        );
    }

    #[test]
    fn metadata_is_single_line_limited_and_missing_values_stay_explicit() {
        assert_eq!(
            clean("  CPU\r\n\t  modèle  ").as_deref(),
            Some("CPU modèle")
        );
        assert_eq!(
            clean(&"é".repeat(600)).unwrap().chars().count(),
            VALUE_LIMIT
        );
        assert!(clean("\0\n\t").is_none());
        assert_eq!(row("Test", None).value, UNAVAILABLE);
    }

    #[test]
    fn native_read_only_collection_has_expected_fields_without_personal_identifiers() {
        let snapshot = collect();
        assert_eq!(snapshot.hardware.len(), 6);
        assert_eq!(snapshot.windows.len(), 4);
        assert_eq!(snapshot.activity.len(), 2);
        for row in snapshot
            .hardware
            .iter()
            .chain(&snapshot.windows)
            .chain(&snapshot.activity)
        {
            assert!(!row.value.is_empty());
            assert!(row.value.chars().count() <= VALUE_LIMIT);
            assert!(!row.value.chars().any(char::is_control));
            assert!(!["Utilisateur", "Nom du PC", "Numéro de série"].contains(&row.label.as_str()));
        }
        assert_ne!(
            snapshot.windows[2].value, UNAVAILABLE,
            "RtlGetVersion should be available on supported Windows"
        );
        assert_ne!(
            snapshot.hardware[4].value, UNAVAILABLE,
            "Physical memory should be readable"
        );
        assert_ne!(
            snapshot.windows[3].value, UNAVAILABLE,
            "Native architecture should be readable"
        );
    }
}
