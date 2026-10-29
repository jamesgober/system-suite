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
Requires: sys-lib 0.4.0 (`mem::pressure`) for the memory watermark moved from 0.5 (D-066).
- [ ] Memory watermark raises the urgent line to `Shed` (sys-lib `mem::pressure`), moved from 0.5.
- [ ] Boot benchmark harness; boot timeline per step.
- [ ] `docs/PERFORMANCE.md` published; criterion baselines stored; the 5% regression gate active.

## v0.7.0 - Hardening
Exit: no open finding; every threat has a passing attack test.
- [ ] Attack tests T1 to T9 complete; `docs/SECURITY.md` with the cost of each defence.
- [ ] Soak: 8 hours of start, reload, and stop cycles with no growth in memory, descriptors, or threads.

## v0.8.0 - Alpha
Exit: two real consumers boot on it on every listed environment.
- [ ] Used by WASP's boot and a Hive DB start-up on Windows 11, WSL 2 Ubuntu, a Linux server, macOS on Apple silicon, and a Docker container with limits.
- [ ] API review against real use; changes made now, not after 1.0.

## v0.9.0 - Beta and release candidates
Exit: two weeks without an API change; every document checked against the build; James's review below done.
- [ ] **James's pre-1.0 review**, before the API is frozen: the API, the workflow, and anything he wants added before 1.0. Whatever he adds goes into this roadmap (into 0.9 or a new milestone before 1.0) with its exit criteria; 1.0 waits for it.
- [ ] API frozen; `0.9.x` only fixes.
- [ ] `docs/STABILITY-1.0.md` drafted with the requirement-to-test map.

## v1.0.0 - Stable
Exit: the audit passes, the definition of done in `DIRECTIVES.md` holds, and James approves.
- [ ] Every FR and NFR has a named, passing test; every threat has a passing attack test.
- [ ] Never published without James's explicit go.
