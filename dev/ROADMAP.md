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

## v0.3.0 - Core: the first boot path
API: SUI-02, SUI-03, SUI-04 (build, instance, guard report, paths, env, log, clock, ids, kernel), SUI-05, SUI-06, SUI-10 (builder basics), SUI-16, SUI-20, SUI-23, SUI-24 for the wired members, SUI-26, SUI-27.
Requires (blockers, recorded 2026-10-22):
- sys-lib 0.3 (boot kit; [sys-lib#2](https://github.com/jamesgober/sys-lib/issues/2)). sys-lib's own 0.2 design lock is waiting for James.
- log-io with a recent-lines sink ([log-io#1](https://github.com/jamesgober/log-io/issues/1)) and a file mode option ([log-io#2](https://github.com/jamesgober/log-io/issues/2)): crash capture and T3.
- path-lib with owner-only directories on Windows ([path-lib#1](https://github.com/jamesgober/path-lib/issues/1)): the T3 attack test on Windows.
Exit: a service boots with `start!`, logs, parks in `run`, and stops in order on SIGTERM and Ctrl+C on all three OSes; every boot step has a failure-injection test.
- [ ] Boot steps 1 to 4, 6, 7, 9 to 11, 15, 17 (`docs/BOOT-ORDER.md`) and the kernel's lifecycle, signal translation (signal-mod), and shutdown with deadlines.
- [ ] Members wired: sys-lib, path-lib, env-lib, log-io, clock-lib, id-forge, signal-mod.
- [ ] `Error`, `ErrorKind`, exit codes, `Error::report`; `process_wide(false)` for tests.
- [ ] Crash capture (panic hook, flush, crash report) with T5 and T9 tests.
- [ ] Attack tests T1, T3, T4, T6, T8.
- [ ] README quick start (Tier 1 first).

## v0.4.0 - Extended: configuration, metrics, kernel services
API: SUI-04 (config, metrics), SUI-11, SUI-12, SUI-13, SUI-14, SUI-15 (without `Secret`), SUI-21, SUI-22, SUI-25.
Requires: nothing unpublished: config-lib 1.1 is the backend as published (D-034); [config-lib#4](https://github.com/jamesgober/config-lib/issues/4) is a cleanup that can land at any time.
Exit: configuration loads, validates, and reloads (T7); subsystems register and stop in declared order; workers and timers run supervised with zero idle CPU.
- [ ] Configuration (section 9): finding, permission check (D-020), the `.env` redirect check (section 18), sections, `FromConfig`, two-phase reload (D-019), with T7 tests. A secret reference is refused in 0.4 (fail closed).
- Moved to 0.5 (D-031): resolving secret references (FR-16) and the T2 tests. Resolution goes through key-vault (D-025), which this roadmap already wires in 0.5; resolving references in 0.4 would mean pulling key-vault forward or resolving them without it.
- [ ] metrics-lib wired; kernel metrics (boot timeline, worker restarts).
- [ ] Urgent line (T0, T1) with `loom` tests; `Subsystem` trait, `Position`, `Link`, `Mailbox`; `Boot`.
- [ ] Workers with restart policies; timer wheel.
- [ ] Health aggregation; `sd_notify` readiness; watchdog heartbeat.

## v0.5.0 - Feature complete
API: every remaining item in `PLAN.md` section 8 (SUI-15 `Secret`, SUI-28, SUI-29, SUI-30); audit, registry, globals, rand, secrets wired.
Requires: audit-trail with verified resume, owner-only files, sync, and a monotonic clock ([audit-trail#3](https://github.com/jamesgober/audit-trail/issues/3)); signal-mod Windows service control ([signal-mod#4](https://github.com/jamesgober/signal-mod/issues/4)); sys-lib 0.5 memory pressure for the urgent line's `Shed` watermark.
Exit: every FR implemented and tested on Linux, macOS, and Windows; `docs/ARCHITECTURE.md`, `docs/CONFIGURATION.md`, `docs/PLATFORM-NOTES.md` complete.
- [ ] audit-trail, registry-io, global-lib, mod-rand, key-vault wired behind their features; secret references with T2 tests.
- [ ] Windows service control through signal-mod (OD-6, D-014).
- [ ] Memory watermark raises the urgent line to `Shed` (sys-lib `mem::pressure`).
- [ ] Feature-combination test matrix (default, full, none, each member alone).
- [ ] Error-suite and security-signals hooks exposed (wired when those packages publish).

## v0.6.0 - Optimisation
Exit: every target in `PLAN.md` section 13 measured and published.
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
