# system-suite - Roadmap

> Path from scaffold to a stable 1.0. Each phase has hard exit criteria and ends with the release routine (bump, CHANGELOG, release notes, local gate, push, green CI, tag, GitHub release, `cargo doc`, `cargo publish`).
> Design, API item ids (SUI-), requirements (FR-, NFR-), and threats (T1 to T9): [`PLAN.md`](./PLAN.md).
>
> **Anti-deferral rule:** no listed task moves to a later phase unless this file records the move and the reason, with a decision-log entry in `PLAN.md`.
>
> **Member dependencies:** several phases wait for a member release (`PLAN.md` section 12.2). When a phase is blocked, record the blocker here and continue with the unblocked items; never copy a member's code into the suite to get around it.

## v0.1.0 - Scaffold (DONE, 2026-10-22)
Exit: CI green on every job; the crate published to claim the name.
- [x] Repository `jamesgober/system-suite` created and cloned to `suite-collection/system-suite`.
- [x] Manifest (edition 2024, MSRV 1.89, one feature per member declared with no dependencies yet), README, CHANGELOG, REPS, dual license, `deny.toml`, `clippy.toml`, `rustfmt.toml`, `.editorconfig`, `.gitattributes`.
- [x] `src/lib.rs`: crate docs, lint attributes, `VERSION` (SUI-01), one test.
- [x] `dev/PLAN.md`, `dev/DIRECTIVES.md`, `dev/ROADMAP.md`.
- [x] CI: Linux, macOS, Windows on x86_64 and aarch64, stable and 1.89; package; security job (audit, deny).

## v0.2.0 - Design lock (DONE, 2026-10-22)
Exit: James approves `docs/API.md`; every open decision answered.
- [x] `docs/API.md` written for every item in `PLAN.md` section 8 (signatures, errors, examples), each marked with the version it arrives in (2026-10-22; item ids SUI-01 to SUI-30).
- [x] `docs/BOOT-ORDER.md` first version: both orders with the reason for every position (2026-10-22; the boot order was revised, D-012).
- [x] Open decisions OD-1 to OD-6 answered by James or accepted as recommended, each recorded in the decision log: OD-1 (D-033), OD-2 (D-034), OD-3 (D-035) answered by James on 2026-10-22; OD-4 (D-016), OD-5 (D-015), OD-6 (D-014) decided. James also replaced the zero-dependency rule with judgement (D-032).
- [x] Member requests filed in their repositories (`PLAN.md` section 12.2): sys-lib#2, signal-mod#4, log-io#1, log-io#2, path-lib#1, config-lib#4 (cleanup only), audit-trail#3. signal-mod#3 was withdrawn after D-032; no noml-core request is needed after D-034.
- [x] One new defence idea recorded (`PLAN.md` section 18: the `.env` redirect check; the earlier 0.2 idea was adopted as D-020).
- [x] James approves `docs/API.md` (2026-10-22), after the kernel, lifecycle, and runtime were split into three modules (D-037).

## v0.3.0 - Core: the first boot path (DONE, 2026-10-23)
API: SUI-02, SUI-03, SUI-04 (build, instance, guard report, paths, env, log, clock, ids, kernel), SUI-05, SUI-06, SUI-10 (builder basics), SUI-16, SUI-20, SUI-23, SUI-24 for the wired members, SUI-26, SUI-27.
Required member releases, all published 2026-10-23: sys-lib 0.3.0 (boot kit), path-lib 1.1.0 (owner-only directories, path-lib#1), log-io 2.1.0 (recent lines and file mode, log-io#1 and #2). signal-mod 2.0 is used as published (its dependencies accepted, D-032).
Exit: a service boots with `start!`, logs, parks in `run`, and stops in order on SIGTERM and Ctrl+C on all three OSes; every boot step has a failure-injection test.
- [x] Boot steps 1 to 4, 6, 7, 9 to 11, 15, 17 (`docs/BOOT-ORDER.md`) and the kernel's lifecycle, signal translation (signal-mod), and shutdown with deadlines.
- [x] Members wired: sys-lib, path-lib, env-lib, log-io, clock-lib, id-forge, signal-mod.
- [x] `Error`, `ErrorKind`, exit codes, `Error::report`; `process_wide(false)` for tests.
- [x] Crash capture (panic hook, flush, crash report with the last 64 log lines) with T5 and T9 tests.
- [x] Attack tests T1, T3 (Unix modes; Windows access control lists), T4 (every step that can fail, with real triggers, plus the unwinding driver), T8 (a stop at every step boundary).
- [x] Stop signals: SIGTERM and SIGINT on Linux and macOS, Ctrl+Break on Windows (Ctrl+C cannot be sent to another process group; both arrive through the same console handler), each in a child process, in CI on all three OSes.
- [x] README quick start (Tier 1 first).
- Moved to 0.4 (D-044): the subsystem-level T6 attack test (a subsystem whose `stop` never returns). There are no subsystems before 0.4; the deadline runner they will use is tested in 0.3 (a step that never returns is abandoned, exit code 79).
- Moved to 0.4 (D-040): the `log-bridge` and `log-tracing` pass-through features, which wait for `log`, `tracing-core`, and `tracing-subscriber` to be reviewed into the dependency register.

## v0.4.0 - Extended: configuration, metrics, kernel services
API: SUI-04 (config, metrics), SUI-11, SUI-12, SUI-13, SUI-14, SUI-15 (without `Secret`), SUI-21, SUI-22, SUI-25.
Requires: nothing unpublished: config-lib 1.1 is the backend as published (D-034); [config-lib#4](https://github.com/jamesgober/config-lib/issues/4) is a cleanup that can land at any time.
Exit: configuration loads, validates, and reloads (T7); subsystems register and stop in declared order; workers and timers run supervised with zero idle CPU.
- [x] Configuration (section 9): finding, permission check on the opened file (D-020, D-049), the `.env` file checked before it is read and its redirect honoured (section 18, D-055), sections with unknown-key refusal, `FromConfig`, two-phase reload (D-019) on SIGHUP and `System::reload`, with T7 tests. A secret reference is refused in 0.4 (fail closed).
- Moved to 0.5 (D-031): resolving secret references (FR-16) and the T2 tests. Resolution goes through key-vault (D-025), which this roadmap already wires in 0.5; resolving references in 0.4 would mean pulling key-vault forward or resolving them without it.
- [x] metrics-lib wired (global registry in process-wide mode, D-053); kernel metrics (boot time, urgent level, worker restarts and failures, subsystem errors, reloads); pass-throughs `metrics-histogram`, `metrics-meter`, `metrics-statsd`, `metrics-http`.
- [x] Urgent line (T0, T1); `Subsystem` trait, `Position`, `Link`, `Mailbox`, `Failure`, `Commit`; `Boot`; `Snapshot` and `Reader`.
- [x] `loom` model tests of the urgent line's word and the worker heartbeat, in their own CI job (D-056, D-058).
- [x] Workers with restart policies, backoff, budgets, criticality, heartbeats; timers (a min-heap, D-046).
- [x] T6 at the subsystem level: a subsystem whose `stop` never returns is abandoned at the deadline, exit code 79, and the subsystem started before it is still stopped (D-044, D-047).
- [x] `log-bridge`, `log-tracing`, `metrics-tracing`, `config-toml`, `config-noml` (accepted by James, D-056; the bridge installed at step 10, D-057; the format by extension, D-059).
- Held by James (D-056): `config-json`, `config-xml`, `config-validation`, `metrics-json`, `metrics-otlp` wait on first-party crates that cover `serde_json`, `quick-xml`, and `regex`; `metrics-system` (with `[metrics] process`) stays out because `sysinfo` was not accepted.
- [x] Health aggregation; `sd_notify` readiness, status, and the watchdog keep-alive (`WATCHDOG_USEC`).

## v0.5.0 - Feature complete
API: every remaining item in `PLAN.md` section 8 (SUI-15 `Secret`, SUI-28, SUI-29, SUI-30); audit, registry, globals, rand, secrets wired.
Requires: audit-trail 1.1.0 (released for it: verified resume, owner-only files, sync, a monotonic clock; [audit-trail#3](https://github.com/jamesgober/audit-trail/issues/3)); signal-mod 2.1.0 (released for it: Windows service control; [signal-mod#4](https://github.com/jamesgober/signal-mod/issues/4)).
Exit: every FR implemented and tested on Linux, macOS, and Windows; `docs/ARCHITECTURE.md`, `docs/CONFIGURATION.md`, `docs/PLATFORM-NOTES.md` complete.
- [x] audit-trail, registry-io, global-lib, mod-rand, key-vault wired behind their features; secret references with T2 tests (D-060 to D-064).
- [x] Windows service control through signal-mod 2.1 (OD-6, D-014, D-065).
- Moved to 0.6 (D-066): the memory watermark that raises the urgent line to `Shed`. It needs sys-lib 0.4.0's `mem::pressure()`, which another session is building and which is not published; 0.5 does not wait for it.
- [x] Feature-combination test matrix: every member alone on Linux, macOS, and Windows, plus default, all, and none (D-067).
- [x] Error-suite and security-signals hooks exposed: `Link::report_error` (logged, counted, and delivered to the error suite when it publishes) and `UrgentLine::set` with `Shed` and `Lockdown` (the security package's entry point, audited). Wiring waits for those packages.
- [x] `docs/ARCHITECTURE.md`, `docs/CONFIGURATION.md`, `docs/PLATFORM-NOTES.md`.

## v0.6.0 - Optimisation
Exit: every target in `PLAN.md` section 13 measured and published.
- Moved on (D-071): the memory watermark that raises the urgent line to `Shed`. It ships in the release after sys-lib 0.4.0 (`mem::pressure`) is published. Done in 0.7.1.
- [x] Boot benchmark harness (standard library only, D-068); boot timeline per step (`System::boot_timeline`, D-070).
- [x] `docs/PERFORMANCE.md` published (every target met on Linux and Windows); baselines stored in `benches/baseline/`; the 5% regression gate with a noise floor active in the release routine (D-068). Idle CPU and wake-ups on macOS and Windows wait for sys-lib 0.4's live readings.

## v0.7.0 - Hardening
Exit: no open finding; every threat has a passing attack test.
- [x] Attack tests T1 to T9 complete; `docs/SECURITY.md` with the cost of each defence. New in 0.7: T3 for the audit chain and its anchor, T5 for a panicking worker, T8 with a real SIGTERM while a subsystem starts.
- [x] Security review of the whole suite: every finding fixed (D-072 to D-078), the rest recorded below as documented limits.
- [x] Soak: 8 hours of start, reload, and stop cycles with no growth in memory, descriptors, or threads. Done 2026-10-29 on Linux (WSL 2): 251,225 cycles; heap, descriptors, and threads the same at the end as at the start (`tests/soak.rs`).
- Documented limit (D-079): `.env` values (secrets referenced as `env:NAME` among them) stay in env-lib's map in ordinary memory for the life of the process. Reason: the same exposure as the process environment, which holds `env:` values in plain memory and which the suite never changes (`std::env::remove_var` is unsafe); a secret that must be wiped belongs in a `file:` or `vault:` reference.
- Documented limit (D-079, narrowed by the pre-1.0 pass): on Unix the owner check needs the current user's id, read through sys-lib; without the `sys` feature only the mode check (no group or other write bit) applies. With `sys`, a user with no entry in the user database is reported by number (sys-lib 0.4, Linux), so the check applies; the container test runs as one. Reason: reading the id needs an OS call, which belongs in sys-lib (`#![forbid(unsafe_code)]`); sys-lib 0.4 is being built in another session, and a `process::uid()` that never consults the user database is a request for it. The directories' own owner-only permissions still apply.
- Documented limit (D-079): the crash hook, removed at shutdown, puts back the hook that was installed before boot; a panic hook the application installed after boot is replaced by that earlier one. Reason: the standard library keeps one process-wide panic hook and gives no way to tell whose it is; an application that installs its own hook after boot is told so in `docs/API.md`.
- Documented limit (D-079): process-wide members set once (log-io's global logger, metrics-lib's global registry) stay installed after a boot that failed after setting them. Reason: the members allow setting them once only; the process exits after a failed boot in every documented use, and a test uses `process_wide(false)`.
- Documented limit (D-079): event handlers run on the thread that tears down, outside the shutdown deadline. Reason: they are the application's code, documented to return quickly; bounding each one would cost a thread per event.

## v0.7.1 - The memory watermark
Requires: sys-lib 0.4.0 (published 2026-10-30).
- [x] The urgent line's memory watermark, opt-in through `[kernel] memory_watch` (D-081), shipped outside 0.8 (D-082).
- [x] Idle CPU on Windows and macOS through sys-lib's `process::cpu_time`; the benchmark runs on all three OSes in CI. A wake-up reading on Windows and macOS is not in sys-lib; kept for the pre-1.0 polish pass.

## v0.8.0 - Alpha
Exit: two real consumers boot on it on every listed environment.
0.8.0 released on 2026-11-06 so a consumer can depend on it (D-091); the exit items below stay open on the 0.8 line.
- [ ] Used by WASP's boot and a Hive DB start-up on Windows 11, WSL 2 Ubuntu, a Linux server, macOS on Apple silicon, and a Docker container with limits. **Blocked (2026-11-02):** neither consumer depends on the suite yet: no `Cargo.toml` outside this repository names it. The suite cannot meet this exit by itself; a consumer has to adopt it. **WASP (2026-11-11):** WASP's gateway boots on the suite (0.8; `System::builder`, the staged `Boot`, `run`, reload); Hive DB does not yet.
- [x] The environments the suite can reach by itself, with `examples/service.rs` standing in until a consumer does: Windows, Linux, and macOS in CI, and a container limited to 256 MiB and 1 CPU, run as a user with no entry in the user database (`.github/scripts/container-test.sh`, D-083).
- [ ] API review against real use; changes made now, not after 1.0. Done against the planned consumers' specification (2026-11-03): the breaking changes are made (D-084 to D-086), the additive ones listed for a consumer to ask for (D-087). Still to do against real use, once a consumer runs on it.

## v0.9.0 - Beta and release candidates
Exit: two weeks without an API change; every document checked against the build; James's review below done.
0.9.0 released on 2026-11-11 with the simplicity pass, because it breaks the API (D-096); 0.8's open exit items carry into 0.9, and the two weeks count from 0.9.0.
- [x] The simplicity pass (James, 2026-11-11): one-line Tier 1 entry points (`run!()`, and `start!()?` with a readable error), plain names, one way to do each thing, `docs/API.md` as a task-oriented tour run as doctests, and `docs/MIGRATION-0.9.md` (D-096).
- [ ] **James's pre-1.0 review**, before the API is frozen: the API, the workflow, and anything he wants added before 1.0. Whatever he adds goes into this roadmap (into 0.9 or a new milestone before 1.0) with its exit criteria; 1.0 waits for it.
- [ ] API frozen; `0.9.x` only fixes.
- [ ] `docs/STABILITY-1.0.md` drafted with the requirement-to-test map.

## v0.9.x - Pre-1.0 polish (before any 1.0 release candidate)
James's standing rule (2026-10-29, D-080): go back and fix every issue and tune the performance before getting close to release.
Exit: nothing below is open without a recorded, hard reason; what is still open is listed in the 1.0 readiness note for James.
First pass run on 2026-11-03, while 0.8 waits for a consumer; the pass is run again before the first 1.0 release candidate, after real use.
- [ ] Every accepted gap, deferred item, known issue, and blocker revisited. Second pass (0.8.1): sys-lib 0.5 adopted; signal-mod's service ordering confirmed by its real-service CI job. First pass: the memory watermark shipped (0.7.1); the owner-check limit narrowed (sys-lib 0.4 names an unknown user by number); `config-json`, `metrics-json`, and `metrics-otlp` shipped on jsonx-lib (config-lib 1.2.0, metrics-lib 2.1.1, D-090; the writer faster than serde_json's, D-092); `config-xml` and `config-validation` still wait for first-party XML and regex crates; `metrics-system` stays out (sysinfo not accepted); wake-up readings on Windows and macOS need a sys-lib API (a request, not filed); the other limits of D-079 stand with their reasons; every "Moved" line is closed.
- [ ] A fresh performance tuning pass. First pass (D-088): boot and stop profiled step by step on Linux; the timer stop's poll removed (a stop with a timer: 1.3 to 2.0 ms to 0.2 ms); the rest kept with reasons in `docs/PERFORMANCE.md`. Windows and macOS were benchmarked, not profiled step by step. Second pass (D-093): Windows profiled step by step; the reload wait's thread is started only while a reload runs (a Windows stop about 450 to 300 µs); the configuration step's file calls and logging's thread kept with reasons.
- [ ] The security and edge-case review run again over the whole suite. First pass (D-089): eleven findings, none high, all fixed (the urgent line's ordering, reason length and same-level floods, a handler's recursion, the unbounded stop record, a reload racing the shutdown, signal floods, an overflowing backoff, a low watermark that never cleared, names ending in '.', the example's first look). Second pass (D-094), an independent review: ten findings, one high (TOML and NOML `include` and `env()` skipped the file checks, fixed in config-lib 1.3), all fixed or recorded as limits. The 0.8.1 Linux gate then found the stop's audit record synced at shutdown step 1, which on a slow disk spent the subsystems' share; it is synced at step 5 (D-095).
- [ ] The same pass for each member release made for the suite. First pass (D-090): signal-mod 2.1.1 (six findings fixed) and audit-trail 1.2.0 (field ambiguity, reader allocation, two writers, links and modes, writes after a torn record, crash tails), both released and adopted; config-lib 1.2.0 and metrics-lib 2.1.1 gained their JSON backends on jsonx-lib 0.5.1 (D-092). Second pass: config-lib 1.3.0 (strict TOML and NOML, byte positions and linear time in `conf`, INI, and Properties, bracket lists of unquoted items) and 1.3.1 (INI quotes and escapes, a CONF section named like a key, CONF parsed with half the allocations).
- [x] The 1.0 readiness note for James: `dev/READINESS-1.0.md`, kept current with each pass.

## v1.0.0 - Stable
Exit: the audit passes, the definition of done in `DIRECTIVES.md` holds, and James approves.
- [ ] Every FR and NFR has a named, passing test; every threat has a passing attack test.
- [ ] Never published without James's explicit go.
