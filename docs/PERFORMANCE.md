<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br><b>system-suite</b><br>
    <sub><sup>PERFORMANCE</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="./API.md" title="API reference"><b>API</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="./ARCHITECTURE.md" title="Architecture"><b>ARCHITECTURE</b></a>
        <span>&nbsp;│&nbsp;</span>
        <span>PERFORMANCE</span>
    </sup>
</div>
<br>

The targets of the plan, measured. Numbers are from `cargo bench --bench boot` (`benches/boot.rs`), version 0.6.0, release profile, on one machine; your numbers depend on your hardware, filesystem, and members.

## Results

Machine: AMD Ryzen 9 9950X3D (16 cores). Linux: Ubuntu under WSL2, kernel 6.6.87.2, Rust 1.99.0. Windows: Windows 11 Pro 26200, Rust 1.95.0.

| Measure | Target | Linux (median / p95) | Windows (median / p95) |
|---|---|---|---|
| boot to ready, default members, local configuration file, log file on | under 5 ms | 0.078 ms / 0.114 ms | 0.31 ms / 0.38 ms |
| stop to exit, nothing in flight | under 50 ms | 0.16 ms / 0.21 ms | 0.20 ms / 0.28 ms |
| idle CPU, a booted system, 5 s | 0 | 0 ticks, 0 context switches | not measured in 0.6 (needs sys-lib 0.4) |
| urgent-line check (`Watch::changed`, nothing changed) | about 1 ns | 0.20 ns | 0.19 ns |
| snapshot read (`Reader::get`, unchanged) | about 1 ns | 0.35 ns | 0.30 ns |
| heap held by a booted system | under 256 KiB | 98 KiB | 99 KiB |

Every target is met. The measured boot is with `process_wide(false)` and no single-instance lock, so it leaves out the signal intake and the lock; both are one system call each and a thread start.

## How it is measured

- **Times** are the median of 200 boots (each followed by a shutdown) and of 21 rounds of 10 million checkpoint reads; p95 is printed beside each median.
- **Heap** is the live bytes a booted system holds, counted by a counting allocator in the benchmark binary (the library itself forbids `unsafe`, D-069), on a second boot so lazy statics from the first do not count.
- **Idle CPU** is the process's user and system ticks (`/proc/self/stat`) and the context switches of all its threads (`/proc/self/task/*/status`) over 5 s of a booted, idle system, after it settled for 0.5 s. Zero switches means no thread of the suite woke at all: the signal intake blocks on the OS, the timer thread waits with no timeout when nothing is due, workers park.
- The harness uses the standard library only (D-068): criterion would bring `serde_json` and `regex`, which are held until first-party crates cover them.

## The regression gate

`benches/baseline/<os>-<arch>.txt` stores the medians. A run with `SYSTEM_SUITE_BENCH_BASELINE=<file>` fails when a time is over 105% of its baseline **and** over the run-to-run noise floor (1 ns for one operation, 20 µs for a boot or a stop): without the floor, a 0.30 ns read measured at 0.31 ns would fail. The gate runs in the local release routine on the reference machine; CI builds and runs the benchmark on every push but does not gate on shared runners, whose noise is larger than 5%.

```bash
cargo bench --bench boot
SYSTEM_SUITE_BENCH_BASELINE=benches/baseline/linux-x86_64.txt cargo bench --bench boot
SYSTEM_SUITE_BENCH_WRITE=benches/baseline/linux-x86_64.txt cargo bench --bench boot
```

## Costs that stay

| Operation | Cost | Where |
|---|---|---|
| a reload | the file read and parse, plus each subsystem's preparation | `System::reload` |
| `AuditLog::record` with `sync = "every"` | one `fdatasync` (milliseconds on rotating disks, tens of microseconds on NVMe) | per record |
| `Secret::with` | key-vault's read path (defragment into a wiped scratch buffer) | per read |
| `System::health` | one `Vec` and the reasons; `status` allocates nothing of its own | per call |
| a log line | log-io's pipeline: formatting and each sink | per line |

The boot timeline of a running system is `System::boot_timeline()` and the `boot step` lines of the log: how long each step took, so a slow start is visible without a profiler.
