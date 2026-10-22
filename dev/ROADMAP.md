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

## v0.2.0 - Design lock
Exit: James approves `docs/API.md`; every open decision answered.
- [ ] `docs/API.md` written for every item in `PLAN.md` section 8 (signatures, errors, examples), each marked with the version it arrives in.
- [ ] `docs/BOOT-ORDER.md` first version: both orders with the reason for every position.
- [ ] Open decisions OD-1 to OD-6 answered by James or accepted as recommended, each recorded in the decision log.
- [ ] Member requests filed in their repositories (`PLAN.md` section 12.2).
- [ ] One new defence idea recorded (`PLAN.md` section 18).

## v0.3.0 - Core: the first boot path
API: SUI-02, SUI-03, SUI-04 (build, instance, paths, env, log, clock, ids), SUI-10 (builder basics), SUI-20, SUI-23, SUI-24 for the wired members.
Requires: sys-lib 0.3 (boot kit).
Exit: a service boots with `start`, logs, parks in `run`, and stops in order on SIGTERM and Ctrl+C on all three OSes; every boot step has a failure-injection test.
- [ ] Boot steps 1 to 8 and the kernel's lifecycle, signal translation (signal-mod), and shutdown with deadlines.
- [ ] Members wired: sys-lib, path-lib, env-lib, log-io, clock-lib, id-forge, signal-mod.
- [ ] Crash capture (panic hook, flush, crash report) with T5 and T9 tests.
- [ ] Attack tests T1, T3, T4, T6, T8.
- [ ] README quick start (Tier 1 first).

## v0.4.0 - Extended: configuration, metrics, kernel services
API: SUI-04 (config, metrics), SUI-11, SUI-12, SUI-13, SUI-14, SUI-21, SUI-22.
Requires: the configuration backend from OD-2 published.
Exit: configuration loads, validates, and reloads (T7); subsystems register and stop in declared order; workers and timers run supervised with zero idle CPU.
- [ ] Configuration (section 9) with secret references (FR-16) and T2 tests.
- [ ] metrics-lib wired; kernel metrics (boot timeline, worker restarts).
- [ ] Urgent line (T0, T1) with `loom` tests; `Subsystem` trait and registration.
- [ ] Workers with restart policies; timer wheel.
- [ ] Health aggregation; `sd_notify` readiness; watchdog heartbeat.

## v0.5.0 - Feature complete
API: every remaining item in `PLAN.md` section 8; audit, registry, globals, rand, secrets wired.
Exit: every FR implemented and tested on Linux, macOS, and Windows; `docs/ARCHITECTURE.md`, `docs/CONFIGURATION.md`, `docs/PLATFORM-NOTES.md` complete.
- [ ] audit-trail, registry-io, global-lib, mod-rand, key-vault wired behind their features.
- [ ] Windows service control through sys-lib (OD-6).
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
Exit: two weeks without an API change; every document checked against the build.
- [ ] API frozen; `0.9.x` only fixes.
- [ ] `docs/STABILITY-1.0.md` drafted with the requirement-to-test map.

## v1.0.0 - Stable
Exit: the audit passes, the definition of done in `DIRECTIVES.md` holds, and James approves.
- [ ] Every FR and NFR has a named, passing test; every threat has a passing attack test.
- [ ] Never published without James's explicit go.
