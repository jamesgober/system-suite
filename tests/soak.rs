//! The soak (0.7): start, reload, and stop cycles for a long time, with no
//! growth in heap, file descriptors, or threads.
//!
//! Ignored by default; run it with
//!
//! ```text
//! SYSTEM_SUITE_SOAK_SECS=28800 cargo test --release --all-features --test soak -- --ignored --nocapture
//! ```
//!
//! (28 800 s is the 8 hours of the roadmap; the default is 60 s.) Every
//! cycle boots a system with a configuration file, a subsystem, a worker,
//! and a timer, reloads it twice (once valid, once refused), and shuts it
//! down. After a warm-up the heap, the open descriptors (Linux), and the
//! threads (Linux) are sampled; the test fails if any grew.

#![cfg(all(feature = "kernel", feature = "sys", feature = "config"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    alloc::{GlobalAlloc, Layout, System as SystemAlloc},
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use system_suite::{
    Member, System,
    lifecycle::{Deadline, Failure, Link, Subsystem},
    runtime::Worker,
};

/// Counts the bytes live on the heap. Test-only: the library itself forbids
/// `unsafe` (D-069).
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

struct Quiet;

impl Subsystem for Quiet {
    fn name(&self) -> &str {
        "soak"
    }
    fn start(&mut self, _link: &Link) -> Result<(), Failure> {
        Ok(())
    }
    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sample {
    heap: usize,
    fds: usize,
    threads: usize,
}

fn count_dir(path: &str) -> usize {
    fs::read_dir(path).map(|d| d.count()).unwrap_or(0)
}

fn sample() -> Sample {
    Sample {
        heap: LIVE.load(Ordering::Relaxed),
        fds: count_dir("/proc/self/fd"),
        threads: count_dir("/proc/self/task"),
    }
}

fn write_private(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn cycle(root: &Path, file: &Path) {
    let system = System::builder("soak")
        .process_wide(false)
        .single_instance(false)
        .disable(Member::Log)
        .portable(root)
        .config_file(file)
        .register(Quiet)
        .start()
        .unwrap();
    let worker = Worker::new("soak-worker")
        .spawn(system.runtime(), |ctx| {
            while !ctx.should_stop() {
                ctx.park(Some(Duration::from_millis(20)));
            }
            Ok(())
        })
        .unwrap();
    let _timer = system
        .runtime()
        .every("soak-timer", Duration::from_millis(5), || {})
        .unwrap();
    write_private(file, "[app]\nlimit = 2\n");
    let _version = system.reload().unwrap();
    write_private(file, "[app\n");
    assert!(system.reload().is_err());
    write_private(file, "[app]\nlimit = 1\n");
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
    drop(worker);
}

#[test]
#[ignore = "long-running: SYSTEM_SUITE_SOAK_SECS=28800 for the roadmap's 8 hours"]
fn soak_start_reload_stop_without_growth() {
    let secs: u64 = env::var("SYSTEM_SUITE_SOAK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let root: PathBuf = env::temp_dir().join(format!("system-suite-soak-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let file = root.join("soak.conf");
    write_private(&file, "[app]\nlimit = 1\n");

    // Warm-up: lazy statics, the allocator's caches, thread-local buffers.
    for _ in 0..200 {
        cycle(&root, &file);
    }
    let baseline = sample();
    let began = Instant::now();
    let mut cycles: u64 = 0;
    let mut worst = baseline;
    let mut next_report = Instant::now();
    while began.elapsed() < Duration::from_secs(secs) {
        cycle(&root, &file);
        cycles += 1;
        let now = sample();
        worst.heap = worst.heap.max(now.heap);
        worst.fds = worst.fds.max(now.fds);
        worst.threads = worst.threads.max(now.threads);
        if Instant::now() >= next_report {
            println!(
                "soak: {cycles} cycles, {:>5} s, heap {} B (start {}), fds {} (start {}), threads {} (start {})",
                began.elapsed().as_secs(),
                now.heap,
                baseline.heap,
                now.fds,
                baseline.fds,
                now.threads,
                baseline.threads
            );
            next_report = Instant::now() + Duration::from_secs(60);
        }
    }
    let end = sample();
    let _ = fs::remove_dir_all(&root);
    println!(
        "soak: done, {cycles} cycles in {} s; end {end:?}, start {baseline:?}, worst {worst:?}",
        began.elapsed().as_secs()
    );
    // Allow the heap 64 KiB of slack for allocator and thread-local noise;
    // descriptors and threads must come back exactly.
    assert!(
        end.heap <= baseline.heap + 64 * 1024,
        "the heap grew: {} -> {} bytes over {cycles} cycles",
        baseline.heap,
        end.heap
    );
    assert_eq!(
        end.fds, baseline.fds,
        "descriptors leaked over {cycles} cycles"
    );
    assert_eq!(
        end.threads, baseline.threads,
        "threads leaked over {cycles} cycles"
    );
}
