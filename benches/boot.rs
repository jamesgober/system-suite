//! The boot benchmark harness (D-068): the targets of `dev/PLAN.md`
//! section 13, measured with the standard library only.
//!
//! ```text
//! cargo bench --bench boot                     # measure and print
//! SYSTEM_SUITE_BENCH_BASELINE=benches/baseline/linux-x86_64.txt cargo bench --bench boot
//!                                              # also fail when a time is over 105% of the baseline
//! SYSTEM_SUITE_BENCH_WRITE=benches/baseline/linux-x86_64.txt cargo bench --bench boot
//!                                              # store the numbers as the new baseline
//! ```
//!
//! Times are the median of many runs (p95 printed beside it). The heap a
//! booted system holds is counted by a counting allocator (this binary
//! only). Idle CPU and wake-ups are read from `/proc/self` on Linux; on
//! other platforms they are not measured until sys-lib's live readings are
//! wired.

use std::{
    collections::BTreeMap,
    env, fs,
    hint::black_box,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Instant,
};
#[cfg(target_os = "linux")]
use std::{thread, time::Duration};

use std::{
    alloc::{GlobalAlloc, Layout, System as SystemAlloc},
    sync::atomic::{AtomicUsize, Ordering},
};

use system_suite::{System, kernel::UrgentLine};

/// Counts the bytes live on the heap, to measure what a running system
/// holds. Benchmark-only: the library itself forbids `unsafe` (D-069).
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards to the system allocator with the same
// arguments and only adds a relaxed counter update, so the allocator's
// contract (size, alignment, ownership) is exactly the system one's.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged; the caller upholds `alloc`'s contract.
        let ptr = unsafe { SystemAlloc.alloc(layout) };
        if !ptr.is_null() {
            let _ = LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged; `ptr` came from this allocator.
        unsafe { SystemAlloc.dealloc(ptr, layout) };
        let _ = LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forwarded unchanged; the caller upholds `realloc`'s contract.
        let new = unsafe { SystemAlloc.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            let _ = LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            let _ = LIVE.fetch_add(new_size, Ordering::Relaxed);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The heap a booted system holds (every platform): live bytes after a
/// second boot, minus before it (the first warms up lazy statics).
fn heap_added() -> Measure {
    let dir = root("heap");
    write_config(&dir);
    let _warm = builder(&dir).start().expect("boot").shutdown();
    let before = LIVE.load(Ordering::Relaxed);
    let system = builder(&dir).start().expect("boot");
    let after = LIVE.load(Ordering::Relaxed);
    let _code = system.shutdown();
    let _ = fs::remove_dir_all(&dir);
    let kib = after.saturating_sub(before) as f64 / 1024.0;
    Measure {
        name: "heap held by a booted system",
        median: kib,
        p95: kib,
        unit: "KiB",
        gated: false,
    }
}

/// One measured value: a time in nanoseconds, or a plain count.
struct Measure {
    name: &'static str,
    median: f64,
    p95: f64,
    unit: &'static str,
    /// Whether the regression gate applies (times, not counts).
    gated: bool,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn summarise(name: &'static str, mut samples: Vec<f64>, unit: &'static str) -> Measure {
    samples.sort_by(f64::total_cmp);
    Measure {
        name,
        median: percentile(&samples, 0.5),
        p95: percentile(&samples, 0.95),
        unit,
        gated: true,
    }
}

/// A private root for one system's directories.
fn root(label: &str) -> PathBuf {
    let dir = env::temp_dir().join(format!("system-suite-bench-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("config")).expect("bench root");
    dir
}

fn write_config(root: &Path) {
    let file = root.join("config").join("bench.conf");
    fs::write(
        &file,
        "[log]\nlevel = \"info\"\nstderr = false\n[app]\nlimit = 1\n",
    )
    .expect("config");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).expect("mode");
    }
}

fn builder(root: &Path) -> system_suite::Builder {
    System::builder("bench")
        .process_wide(false)
        .single_instance(false)
        .portable(root)
}

/// Boot to ready and stop to exit, default members, a local file.
fn boot_and_stop(runs: usize) -> (Measure, Measure) {
    let dir = root("boot");
    write_config(&dir);
    let mut boots = Vec::with_capacity(runs);
    let mut stops = Vec::with_capacity(runs);
    for _ in 0..runs {
        let began = Instant::now();
        let system = builder(&dir).start().expect("boot");
        boots.push(began.elapsed().as_nanos() as f64);
        let began = Instant::now();
        let _code = system.shutdown();
        stops.push(began.elapsed().as_nanos() as f64);
    }
    let _ = fs::remove_dir_all(&dir);
    (
        summarise("boot to ready", boots, "ns"),
        summarise("stop to exit", stops, "ns"),
    )
}

/// The cost of one checkpoint read on the urgent line when nothing changed.
fn urgent_check() -> Measure {
    let line = UrgentLine::detached();
    let mut watch = line.watch();
    let mut samples = Vec::new();
    const ITERS: u32 = 10_000_000;
    for _ in 0..21 {
        let began = Instant::now();
        for _ in 0..ITERS {
            black_box(watch.changed());
        }
        samples.push(began.elapsed().as_nanos() as f64 / f64::from(ITERS));
    }
    summarise("urgent-line check", samples, "ns")
}

/// The cost of `Reader::get` on an unchanged configuration snapshot.
fn reader_get() -> Measure {
    let snapshot = system_suite::Snapshot::new(42_u64);
    let mut reader = snapshot.reader();
    let mut samples = Vec::new();
    const ITERS: u32 = 10_000_000;
    for _ in 0..21 {
        let began = Instant::now();
        for _ in 0..ITERS {
            black_box(*reader.get());
        }
        samples.push(began.elapsed().as_nanos() as f64 / f64::from(ITERS));
    }
    summarise("snapshot read, unchanged", samples, "ns")
}

/// CPU ticks and context switches of the whole process over an idle
/// period, and the resident memory a booted system adds (Linux).
#[cfg(target_os = "linux")]
fn idle_and_memory() -> Vec<Measure> {
    fn cpu_ticks() -> u64 {
        let stat = fs::read_to_string("/proc/self/stat").unwrap_or_default();
        // Fields after the command name (which may hold spaces) are
        // separated by single spaces; utime and stime are fields 14 and 15.
        let after = stat.rsplit_once(") ").map_or("", |(_, rest)| rest);
        let fields: Vec<&str> = after.split(' ').collect();
        let get = |i: usize| {
            fields
                .get(i)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
        };
        get(11) + get(12)
    }
    fn switches() -> u64 {
        let mut total = 0;
        if let Ok(tasks) = fs::read_dir("/proc/self/task") {
            for task in tasks.flatten() {
                let status = fs::read_to_string(task.path().join("status")).unwrap_or_default();
                for line in status.lines() {
                    if let Some(v) = line
                        .strip_prefix("voluntary_ctxt_switches:")
                        .or_else(|| line.strip_prefix("nonvoluntary_ctxt_switches:"))
                    {
                        total += v.trim().parse::<u64>().unwrap_or(0);
                    }
                }
            }
        }
        total
    }

    let dir = root("idle");
    write_config(&dir);
    let system = builder(&dir).start().expect("boot");
    // Let start-up settle, then measure a quiet period.
    thread::sleep(Duration::from_millis(500));
    let (ticks, wakes) = (cpu_ticks(), switches());
    let idle = Duration::from_secs(5);
    let main_wakes_from_sleep = 1; // this thread's own sleep below
    thread::sleep(idle);
    let ticks = cpu_ticks().saturating_sub(ticks);
    let wakes = switches()
        .saturating_sub(wakes)
        .saturating_sub(main_wakes_from_sleep);
    let _code = system.shutdown();
    let _ = fs::remove_dir_all(&dir);
    vec![
        Measure {
            name: "idle CPU ticks in 5 s",
            median: ticks as f64,
            p95: ticks as f64,
            unit: "ticks",
            gated: false,
        },
        Measure {
            name: "idle wake-ups in 5 s",
            median: wakes as f64,
            p95: wakes as f64,
            unit: "switches",
            gated: false,
        },
        Measure {
            name: "memory added by a booted system",
            median: after.saturating_sub(before) as f64,
            p95: after.saturating_sub(before) as f64,
            unit: "KiB",
            gated: false,
        },
    ]
}

#[cfg(not(target_os = "linux"))]
fn idle_and_memory() -> Vec<Measure> {
    Vec::new()
}

fn read_baseline(path: &Path) -> BTreeMap<String, f64> {
    let text = fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (name, value) = l.rsplit_once('\t')?;
            Some((name.to_owned(), value.trim().parse().ok()?))
        })
        .collect()
}

fn main() -> ExitCode {
    // `cargo bench` passes `--bench`; nothing else is accepted.
    // Heap and idle first, before the repeated boots.
    let mut first = vec![heap_added()];
    first.extend(idle_and_memory());
    let runs = 200;
    let (boot, stop) = boot_and_stop(runs);
    let mut measures = vec![boot, stop, urgent_check(), reader_get()];
    measures.extend(first);

    println!("{:<34} {:>14} {:>14}  unit", "measure", "median", "p95");
    for m in &measures {
        println!(
            "{:<34} {:>14.1} {:>14.1}  {}",
            m.name, m.median, m.p95, m.unit
        );
    }

    if let Ok(path) = env::var("SYSTEM_SUITE_BENCH_WRITE") {
        let mut out = String::from("# system-suite boot benchmark baseline: measure<TAB>median\n");
        for m in &measures {
            out.push_str(&format!("{}\t{:.1}\n", m.name, m.median));
        }
        fs::write(&path, out).expect("write baseline");
        println!("baseline written to {path}");
    }

    let mut failed = false;
    if let Ok(path) = env::var("SYSTEM_SUITE_BENCH_BASELINE") {
        let baseline = read_baseline(Path::new(&path));
        for m in measures.iter().filter(|m| m.gated) {
            if let Some(base) = baseline.get(m.name) {
                let limit = base * 1.05;
                if m.median > limit {
                    println!(
                        "REGRESSION: {} median {:.1} {} > 105% of baseline {:.1}",
                        m.name, m.median, m.unit, base
                    );
                    failed = true;
                }
            }
        }
        if !failed {
            println!("no regression against {path}");
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
