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
| idle CPU, a booted system, 5 s | 0 | 0 ticks, 0 context switches | 0.0 ms of CPU time (0.7.1); wake-ups have no reading |
| urgent-line check (`Watch::changed`, nothing changed) | about 1 ns | 0.20 ns | 0.19 ns |
| snapshot read (`Reader::get`, unchanged) | about 1 ns | 0.35 ns | 0.30 ns |
| heap held by a booted system | under 256 KiB | 98 KiB | 99 KiB |

Measured again for 0.7.1 on a busy machine: 0.6.0 and 0.7.1, run alternately, could not be told apart, so the baselines stand. macOS has no reference machine; CI runs the benchmark there on every push, without a gate. On a shared macOS runner (0.7.1): boot 0.12 ms, stop 0.17 ms, 86 KiB of heap, and 0.1 ms of CPU time over 5 idle seconds, one wake of the benchmark's own sleep at that resolution. Every target is met. The measured boot is with `process_wide(false)` and no single-instance lock, so it leaves out the signal intake and the lock; both are one system call each and a thread start.

## How it is measured

- **Times** are the median of 200 boots (each followed by a shutdown) and of 21 rounds of 10 million checkpoint reads; p95 is printed beside each median.
- **Heap** is the live bytes a booted system holds, counted by a counting allocator in the benchmark binary (the library itself forbids `unsafe`, D-069), on a second boot so lazy statics from the first do not count.
- **Idle CPU** on Linux is the process's user and system ticks (`/proc/self/stat`) and the context switches of all its threads (`/proc/self/task/*/status`) over 5 s of a booted, idle system, after it settled for 0.5 s. On Windows and macOS it is the process's CPU time over the same period (sys-lib's `process::cpu_time`); it shows no CPU used, but its resolution is coarse (often 15.6 ms on Windows), and neither OS has a wake-up reading in sys-lib. Zero switches means no thread of the suite woke at all: the signal intake blocks on the OS, the timer thread waits with no timeout when nothing is due, workers park.
- The harness uses the standard library only (D-068): criterion would bring `serde_json` and `regex`, which are held until first-party crates cover them.

## The regression gate

`benches/baseline/<os>-<arch>.txt` stores the medians. A run with `SYSTEM_SUITE_BENCH_BASELINE=<file>` fails when a time is over 105% of its baseline **and** over the run-to-run noise floor (1 ns for one operation, 20 µs for a boot or a stop): without the floor, a 0.30 ns read measured at 0.31 ns would fail. The gate runs in the local release routine on the reference machine; CI builds and runs the benchmark on every push but does not gate on shared runners, whose noise is larger than 5%.

```bash
cargo bench --bench boot
SYSTEM_SUITE_BENCH_BASELINE=benches/baseline/linux-x86_64.txt cargo bench --bench boot
SYSTEM_SUITE_BENCH_WRITE=benches/baseline/linux-x86_64.txt cargo bench --bench boot
```

## The pre-1.0 tuning pass

Where a boot and a stop spend their time, measured step by step (`System::boot_timeline`, and the teardown timed in a copy of the code), Linux, release profile, 300 boots after 20 to warm up (medians):

| Boot step | Time | | Shutdown step | Time |
|---|---|---|---|---|
| 3 paths | 5.5 µs | | 1 draining | 2 µs |
| 4 environment | 4.7 µs | | 2 subsystems (none) | 0 µs |
| 5 configuration | 41.9 µs | | 3 workers and timers | 1 µs |
| 6 guard | 30.5 µs | | 9 logging | 107 µs |
| 10 logging | 25.9 µs | | every other step | under 1 µs |
| every other step | under 1.5 µs each | | | |

- **Fixed:** with any timer armed (the memory watermark, `[audit] sync = "flush"`, the systemd keep-alive), stopping the timer thread polled with 1 ms sleeps, so a stop took 1.3 to 2.0 ms instead of about 0.2 ms. It now waits on a channel the thread disconnects when it ends: 0.15 to 0.25 ms (median, same machine, alternating runs).
- **Kept:** closing the log is most of a stop because it runs on its own stop thread, which is what keeps a blocked sink (a full pipe on standard error) from holding the process past its deadline (threat T6). The thread start is the cost, about 0.1 ms once per process; there is no `fsync`. Configuration, guard, and logging are the members' own file work (finding, checking, and reading the file; the umask and limits; creating the log file).
- **Nothing to gain on the hot path:** the urgent-line check and an unchanged snapshot read are a single atomic load each (0.2 and 0.35 ns).

The same profile on Windows (0.8.1, D-093), release profile, 300 boots after 20 to warm up, on a reference machine running other work (medians of three runs; compare within a run):

| Boot step | Time | | Shutdown step | Time |
|---|---|---|---|---|
| 3 paths | 10 µs | | 1 draining | 3 µs |
| 4 environment | 35 to 50 µs | | 2 subsystems (none) | 0.1 µs (was 126 µs) |
| 5 configuration | 140 to 200 µs | | 3 workers and timers | 5 µs |
| 9 directories | 30 to 48 µs | | 9 logging | 156 to 210 µs |
| 10 logging | 125 to 190 µs | | every other step | under 1 µs |
| every other step | under 2 µs each | | | |

- **Fixed:** shutdown step 2 started a thread to wait for a reload on every stop, even with none running (the reload and shutdown ordering of D-089, added after the Linux profile above). Starting a thread costs 110 to 130 µs on Windows. It is started only while a reload runs: a stop went from about 450 µs to 250 to 330 µs. On Linux a thread start is cheap next to the noise of a loaded machine: three alternating runs of the benchmark could not tell the two apart.
- **Kept:** the configuration step is file calls at 30 to 40 µs each on Windows (the metadata check, the open, the read; the parse of a small file is about 20 µs), which is the file system and its filter drivers. The metadata check before the open is what keeps a FIFO from blocking the boot on Unix. Logging, at boot and at stop, is mostly a thread start: the stop's is the T6 bound (D-088).

## Costs that stay

| Operation | Cost | Where |
|---|---|---|
| a reload | the file read and parse, plus each subsystem's preparation | `System::reload` |
| one memory watermark reading | 10 µs on Linux, 8 µs on macOS, 1.9 µs on Windows (medians); at `memory_watch = "5s"`, about 2 parts per million of one core and 0.2 wake-ups per second | only with `[kernel] memory_watch` set |
| `AuditLog::record` with `sync = "every"` | one `fdatasync` (milliseconds on rotating disks, tens of microseconds on NVMe) | per record |
| `Secret::with` | key-vault's read path (defragment into a wiped scratch buffer) | per read |
| `System::health` | one `Vec` and the reasons; `status` allocates nothing of its own | per call |
| a log line | log-io's pipeline: formatting and each sink | per line |

The boot timeline of a running system is `System::boot_timeline()` and the `boot step` lines of the log: how long each step took, so a slow start is visible without a profiler.
