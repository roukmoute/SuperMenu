use super::*;
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

fn counters() -> PROCESS_MEMORY_COUNTERS_EX {
    let mut value: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    value.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let success = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut value as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
            value.cb,
        )
    };
    assert_ne!(success, 0);
    value
}

#[test]
#[ignore = "isolated frozen-index memory experiment"]
fn frozen_index_memory_measurement() {
    // Initialize the same normalization/runtime paths before the baseline.
    std::hint::black_box(Item::new(
        "probe",
        "Probe é",
        "Probe",
        "",
        Kind::File,
        "probe",
    ));
    let before = counters();
    let start = Instant::now();
    let items = read_cache_file(
        Path::new(&std::env::var("SUPER_MENU_MEMORY_FIXTURE").unwrap()),
        "fr",
    );
    let restore_ms = start.elapsed().as_secs_f64() * 1000.0;
    std::hint::black_box(&items);
    let after = counters();
    assert!(
        !items.is_empty(),
        "memory fixture must restore a nonempty cache"
    );
    let owned_capacities: usize = items
        .iter()
        .map(|item| {
            item.id.capacity()
                + item.title.capacity()
                + item.subtitle.capacity()
                + item.glyph.capacity()
                + item.target.capacity()
                + item.arguments.capacity()
        })
        .sum();
    let materialized = serde_json::to_vec(&items).unwrap();
    let items_sha256 = format!("{:x}", Sha256::digest(&materialized));
    let mut ranked = std::collections::BTreeMap::new();
    let usage = std::collections::HashMap::new();
    let mut rank_times = std::collections::BTreeMap::new();
    for query in ["steam", "artisan", "firefox", "settings", "supermenu", "é"] {
        let results = crate::core::rank(&items, query, 9, &usage);
        ranked.insert(
            query,
            results
                .iter()
                .map(|item| item.id.clone())
                .collect::<Vec<_>>(),
        );
        let mut samples = Vec::with_capacity(101);
        for _ in 0..10 {
            std::hint::black_box(crate::core::rank(&items, query, 9, &usage));
        }
        for _ in 0..101 {
            let start = Instant::now();
            std::hint::black_box(crate::core::rank(&items, query, 9, &usage));
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        rank_times.insert(
            query,
            serde_json::json!({ "median_ms": samples[50], "p95_ms": samples[95] }),
        );
    }
    let report = serde_json::json!({
        "items": items.len(), "capacity": items.capacity(), "item_bytes": std::mem::size_of::<Item>(),
        "owned_string_capacities_bytes": owned_capacities, "restore_ms": restore_ms,
        "before_private": before.PrivateUsage, "after_private": after.PrivateUsage,
        "private_delta": after.PrivateUsage as i64 - before.PrivateUsage as i64,
        "before_working_set": before.WorkingSetSize, "after_working_set": after.WorkingSetSize,
        "working_set_delta": after.WorkingSetSize as i64 - before.WorkingSetSize as i64,
        "before_peak_working_set": before.PeakWorkingSetSize, "after_peak_working_set": after.PeakWorkingSetSize,
        "peak_working_set_delta": after.PeakWorkingSetSize as i64 - before.PeakWorkingSetSize as i64,
        "materialized_json_sha256": items_sha256, "ranked_top9": ranked, "rank_times": rank_times
    });
    std::fs::write(
        std::env::var("SUPER_MENU_MEMORY_REPORT").unwrap(),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    std::hint::black_box(&items);
}
