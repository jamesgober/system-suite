# system-suite - Project Plan

> The design, strategy, and decision record for `system-suite`. A living document: every design decision, idea, or open question about the suite is written here or in [`ROADMAP.md`](./ROADMAP.md) before it is acted on.
>
> **Read order:** [`../REPS.md`](../REPS.md), then [`DIRECTIVES.md`](./DIRECTIVES.md), then **this file**, then [`ROADMAP.md`](./ROADMAP.md).
>
> **Status:** `0.1.0` published. The `0.2.0` design lock is written ([`../docs/API.md`](../docs/API.md), [`../docs/BOOT-ORDER.md`](../docs/BOOT-ORDER.md)); `0.3.0`, the first boot path, is published. `0.4.0` (configuration, metrics, subsystems, workers, timers) is next.

---

## Contents

1. [Purpose](#1-purpose)
2. [Where it sits](#2-where-it-sits)
3. [Scope](#3-scope)
4. [Members](#4-members)
5. [Requirements](#5-requirements)
6. [The kernel](#6-the-kernel)
7. [Boot and shutdown order](#7-boot-and-shutdown-order)
8. [API outline](#8-api-outline)
9. [Configuration](#9-configuration)
10. [Threats](#10-threats)
11. [Architecture](#11-architecture)
12. [Ecosystem and dependencies](#12-ecosystem-and-dependencies)
13. [Performance](#13-performance)
14. [Testing](#14-testing)
15. [Documentation deliverables](#15-documentation-deliverables)
16. [Versioning a suite](#16-versioning-a-suite)
17. [Open decisions](#17-open-decisions)
18. [Ideas](#18-ideas)
19. [Decision log](#19-decision-log)

---

## 1. Purpose

`system-suite` is the foundation every Rust service in the portfolio starts from. A project adds one dependency and calls one function, and gets, already wired and started in the right order:

- **system facts and the boot kit** (OS, CPU, memory, build info, instance info, a single-instance process guard),
- **paths** (application and OS directories, hardened joining),
- **environment** (variables and `.env` files),
- **configuration** (load, validate, reload),
- **logging**, **metrics**, and **audit**,
- **signals** (stop, reload, dump),
- **time** (monotonic and wall clock, mockable) and **ids** (request, instance, entity),
- optional **registry**, **globals**, **randomness**, and **secret storage**,
- and a **kernel** that boots all of it, supervises background work, reports health and readiness, captures crashes, and shuts everything down in reverse order.

It exists because every project needs the same ten libraries, and adding, configuring, and wiring them by hand each time is slow, repetitive, and the step most often skipped. Projects stall when the hard parts are done and the foundation is missing. The suite removes that step.

A suite is a complete package: a project using `system-suite` should need no other foundation library. Anything a member should do is added to that member in its own pull request; the suite adds wiring, not features (D-002).

### 1.1 What it is not

- Not a framework: it has no networking, routing, or request handling. Networked frameworks build on top of it (WASP, service frameworks with a network pipeline).
- Not an async runtime. It never depends on one; background work runs on the kernel's own supervised threads.
- Not a database layer: providers (databases, caches, key-value stores) are registered by the projects that use them.
- Not the error or security system itself: those are separate planned packages (error suite and security signals, section 12.3) that plug into the kernel when published.

---

## 2. Where it sits

### 2.1 Who uses it

| Consumer | What it uses |
|---|---|
| WASP (proprietary platform) | boot phase 0 to 2: foundation handles, kernel, urgent line, shutdown order, crash capture |
| Hive DB products | the same foundation without WASP: guard beside the data directory, config, logs, metrics, kernel |
| Service frameworks (planned: daemon and CLI, then a network pipeline with an auth trait) | everything; they add their own layer on top |
| Command-line tools | paths, env, config, logging, signals; kernel optional |

### 2.2 Layering

```
 application / framework (WASP core, Hive DB, a service framework, a CLI)
   │  registers its own subsystems with the kernel
   ▼
 system-suite
   kernel  ── lifecycle, boot and shutdown order, urgent line, workers, timers, health, crash capture
   handles ── typed access to every member (no lookups by name at runtime)
   ▼
 members (each its own published crate, each behind a feature)
   sys-lib · path-lib · env-lib · config · log-io · metrics-lib · signal-mod · clock-lib · id-forge
   audit-trail · registry-io · global-lib · mod-rand · key-vault
```

---

## 3. Scope

### 3.1 In scope

- One start call that boots every enabled member in a fixed, documented order and fails closed if any step fails.
- Typed handles to every member, resolved once at boot.
- The kernel (section 6).
- Configuration loading with a single source of truth for every member's settings.
- Feature flags so unused members are never compiled.
- Re-exports of each member as a module (`system_suite::log`, `system_suite::config`, ...), so a project imports one crate.
- Tests that run the whole boot and shutdown on Linux, macOS, and Windows.

### 3.2 Out of scope, and where it goes

| Not here | Where it lives |
|---|---|
| Networking, request pipelines, auth | WASP gateway-io, net-io, serve-net, gatekeeper |
| Error definitions, codes, reports, the error service | the error suite (planned); the kernel exposes the hook it plugs into |
| Security signals, scoring, enforcement | the security signals library (planned); the kernel exposes the shared threat state it plugs into |
| Job queues, retries, cron | task-io |
| Databases, caches | the project's providers |
| An async runtime | none, by design |
| Watchdog process, socket hand-off restarts | the planned out-of-process watchdog (tower); the kernel reports heartbeats to it |

---

## 4. Members

Versions checked on crates.io on 2026-10-22; public APIs read from source at the design lock (D-026 records what changed from the 0.1.0 table).

| Member | Feature | Crate | Published | Third-party runtime deps (as the suite uses it) | Status for the suite |
|---|---|---|---|---|---|
| System facts, boot kit | `sys` | sys-lib | 0.3.0 | none | wired in suite 0.3 |
| Paths | `paths` | path-lib | 1.1.0 | none | wired in suite 0.3; `ensure_private` (1.1) makes the log and lock directories owner-only on every OS |
| Environment | `env` | env-lib | 1.0.0 | none (uses path-lib) | ready; the suite never calls `Env::export` (it is `unsafe`) |
| Configuration | `config` | config-lib (OD-2, D-034) | 1.1.0 | thiserror, serde (unused), dashmap, arc-swap; `notify` is off (the suite reloads by signal) | ready; [config-lib#4](https://github.com/jamesgober/config-lib/issues/4) is a cleanup, not a blocker |
| Logging | `log` | log-io | 2.1.0 | none | wired in suite 0.3; `RecentLines` (2.1) feeds crash reports, `mode` (2.1) makes log files owner-only |
| Metrics | `metrics` | metrics-lib | 2.0.0 | none with its integrations off | ready |
| Signals | `signals` | signal-mod | 2.0.0 | signal-hook, libc (Unix), windows-sys (Windows), with `default-features = false, features = ["std", "agnostic"]` (no tokio) | ready (dependencies accepted, D-032); Windows service control for 0.5 |
| Time | `time` | clock-lib | 1.1.0 (1.1.1 in progress) | none | ready |
| Ids | `ids` | id-forge | 1.0.0 | none | ready; the suite calls only `try_next_id` (`next_id` panics on clock skew) |
| Audit | `audit` | audit-trail | 1.0.1 | sha2 (blake3 through `audit-blake3`) | opt-in (OD-3, D-035); needs resume-and-verify from a file, owner-only files, `sync`, a clock that never goes back (requested; blocker for 0.5) |
| Registry | `registry` | registry-io | 1.1.0 | none | ready |
| Globals | `globals` | global-lib | 1.0.0 | none | ready |
| Randomness | `rand` | mod-rand | 1.0.0 (Apache-2.0 only) | none | ready |
| Secrets | `secrets` | key-vault | 1.1.0 | subtle, getrandom, blake3, zeroize, arc-swap | ready; opt-in because of its dependencies |
| Kernel | `kernel` | this crate (OD-1, D-033) | - | none | built here |

log-io and mod-rand are licensed Apache-2.0 only; the suite's own code stays `Apache-2.0 OR MIT`, and a binary that includes those members follows Apache-2.0 for them (both licences are on `deny.toml`'s allow list).

`service-kernel` (local, unpublished 0.1.0): its workers, shutdown coordinator, drain, and signal handling all require tokio; every message goes through lang-lib; mod-events, error-forge, metrics-lib, fsys, and proc-daemon are declared but unused. Its runtime-free shapes informed the kernel here (the lifecycle transition table, restart and backoff policies, criticality, the push-based health aggregate); its code is not reused (OD-1).

---

## 5. Requirements

### 5.1 Functional

| Id | Requirement |
|---|---|
| FR-1 | `system_suite::start!()` (or `start!(name)`; a macro, D-008) boots every enabled member in the order of section 7 and returns a `System` only when every step succeeded. |
| FR-2 | Any boot failure stops the boot, shuts down what already started in reverse order, and returns an error that names the step and says what to do. Nothing is left half-started. |
| FR-3 | `System` gives typed handles to every enabled member (`system.log()`, `system.config()`, ...). No member is looked up by name at runtime. |
| FR-4 | `System::run()` parks the calling thread with zero CPU until a stop is requested (signal, `request_stop`, or the watchdog), then runs the shutdown order of section 7 and returns the exit code. |
| FR-5 | Signals go to the kernel only. The kernel maps them to lifecycle decisions (stop, reload, dump); nothing else receives raw signals. |
| FR-6 | The urgent line (section 6.3) changes the process-wide level in one atomic write and wakes every registered subsystem directly. |
| FR-7 | Projects register their own subsystems (`Subsystem` trait) with a name, a shutdown position, and start, stop, and health functions; the kernel starts them after the members and stops them before the members. |
| FR-8 | The shutdown order is declared, not coded: each subsystem states its position; the kernel runs them in reverse within a deadline and forces the rest at the deadline. |
| FR-9 | Background work runs on supervised kernel workers with a restart policy; a worker that panics is restarted per its policy and reported. |
| FR-10 | Timers (periodic and one-shot) run without a thread per timer and without waking when nothing is due. |
| FR-11 | Health aggregates every member and subsystem; readiness is reported to systemd (`sd_notify`) and to the watchdog when present. |
| FR-12 | A panic hook records the panic, flushes logs, metrics, and audit, writes a crash report (build info, instance info, panic, recent errors), and then lets the process end. |
| FR-13 | Configuration reload (SIGHUP or a call) builds the new configuration, validates it, and swaps it in one step; an invalid configuration is refused and the old one stays. |
| FR-14 | Every member's settings come from one configuration source, with a documented section per member and secure defaults when a section is absent. |
| FR-15 | Each member can be disabled at compile time (feature) and, where it makes sense, at run time (configuration). |
| FR-16 | Secrets in configuration are references (`vault:name`, `env:NAME`, `file:path`), resolved at boot through key-vault or the named source; plain-text secrets in configuration files are refused unless explicitly allowed. |

### 5.2 Non-functional

| Id | Requirement |
|---|---|
| NFR-1 | Every third-party runtime dependency, default or optional, is a judged and recorded choice in the dependency register (section 12.4): who maintains it, how actively, its `unsafe` and soundness record, its transitive tree, its MSRV, and its licence. `cargo audit` and `cargo deny` gate it in CI. No async runtime (D-005). (Replaces the 0.1.0 zero-dependency rule, D-032.) |
| NFR-2 | Zero CPU when idle: no polling threads, no timers firing for nothing. |
| NFR-3 | Boot to ready with the default members and a local configuration file in under 5 ms [target, measured in 0.6]. |
| NFR-4 | Stop request to exit with no work in flight in under 50 ms [target]. |
| NFR-5 | Urgent-line read at a checkpoint: about 1 ns; propagation to every core: under 100 ns [est.]. |
| NFR-6 | Memory added by the suite itself (excluding members' own buffers): under 256 KiB [target]. |
| NFR-7 | No panics in library code; every error variant actionable. |
| NFR-8 | Linux, macOS, and Windows on x86_64 and aarch64, stable and MSRV 1.89. |
| NFR-9 | `forbid(unsafe_code)`: every OS call happens inside a member. |

---

## 6. The kernel

### 6.1 What it is

The kernel is the suite's controller: it owns the lifecycle, the order, and the background work. It is built as three modules of this crate, all behind the `kernel` feature (D-037): `kernel` (the controller: signals to decisions, urgent line, boot and shutdown orders, health, readiness, crash capture, events), `lifecycle` (the phases and the `Subsystem` contract), and `runtime` (supervised workers and timers). It is the "system core" of the cores architecture: every subsystem registers with it, and registration is the subscription (nothing can miss a stop or an urgent change).

### 6.2 Lifecycle

```
 Booting ─▶ Ready ─▶ Draining ─▶ Stopping ─▶ Stopped
    │                    ▲
    └── boot failure ────┘ (reverse shutdown of what started, then Stopped with the error)
```

Reload does not change the lifecycle state; it swaps configuration snapshots (FR-13).

### 6.3 The urgent line

One cache-line-aligned atomic level and epoch that every worker reads at its checkpoints, plus a mailbox per subsystem poked directly and unparked. Levels: `Normal`, `Shed`, `Lockdown`, `Draining`, `Stopping`. Raised by the kernel (signals, watchdog commands, memory watermark from sys-lib) and by the security package when present. The registry (if enabled) records what happened; it never delivers urgent events.

| Tier | Mechanism | Cost [est.] |
|---|---|---|
| T0 | atomic level and epoch, read at checkpoints | read about 1 ns; visible on other cores in 40 to 100 ns |
| T1 | per-subsystem mailbox and unpark | store ns; waking a parked thread µs |
| T2 | events to logs, metrics, audit | µs, buffered |

### 6.4 Signals

| Signal | Kernel decision |
|---|---|
| SIGTERM, SIGINT, Ctrl+C, Windows service stop | stop: `Draining`, then `Stopping` at the deadline |
| SIGHUP | configuration reload |
| SIGUSR1 (Unix) | dump health and state to the log |
| watchdog "memory critical" | urgent level `Shed` |

On Windows, signal-mod reports logoff and system shutdown as its `Hangup` signal, so the kernel maps `Hangup` to stop on Windows and to reload on Unix. Windows reload and dump come from the service control manager (`PARAMCHANGE`, user control code 128) once signal-mod supports service control (D-014).

### 6.5 Workers and timers

- Workers are named threads with a restart policy (never, on failure, always), a backoff with jitter, a restart budget, and a criticality; their health is a heartbeat word written only by the worker (D-023).
- Timers run on one kernel timer thread, ordered in a min-heap of due times (D-046); with nothing due, the thread waits with no timeout.
- Long-running work belongs to task-io; the kernel's workers are for the services' own housekeeping (log flushing, metrics export, cache sweeps).

### 6.6 Health and readiness

- `system.health()` aggregates members, subsystems, workers, and the timer thread into a `HealthReport` (status `Healthy`, `Degraded`, or `Unhealthy`, with each component's reason); `system.status()` returns the aggregate alone without allocating (D-022).
- Readiness: `READY=1` at ready, `STOPPING=1` at the start of shutdown, `STATUS=` on changes, and `WATCHDOG=1` keep-alives while not `Unhealthy`, to systemd when `NOTIFY_SOCKET` is set (std Unix datagram, no FFI; D-030); heartbeats to the watchdog (tower) when it exists.

### 6.7 Crash capture

The panic hook (installed at boot unless disabled) reports the panic, flushes every member that buffers (logs, metrics, audit), writes `crash-<instance>-<time>.txt` to the application's log directory (build info, instance info, the panic message and location, the last N log lines held in a small ring), and returns so the process ends as configured (`panic = "abort"` in release). Error-suite integration (section 12.3) replaces the plain report with coded events when available.

---

## 7. Boot and shutdown order

The orders, with the reason for every position, are in [`../docs/BOOT-ORDER.md`](../docs/BOOT-ORDER.md) (the source of truth; revised at the design lock, D-012). Summary:

### 7.1 Boot (listeners and other outward-facing work belong to the application and come last)

```
 1 build info         compiled into the application by start! (sys-lib)
 2 instance info      id, start time, host, pid, user; OS random source checked (sys-lib, mod-rand)
 3 paths              resolved, nothing created (path-lib)
 4 environment        .env read, process environment never changed (env-lib)
 5 configuration      found, permission-checked, parsed, validated; secret references parsed (config)
 6 process guard      single-instance lock, pid file, umask, open-file limit (sys-lib)   fail: exit, nothing created
 7 signals            the kernel's intake installed; acted on at step boundaries (signal-mod)
 8 secrets            references resolved into the vault (key-vault)
 9 directories        created owner-only (path-lib)
10 logging            sinks opened; build, instance, guard warnings, steps 1 to 9 timed (log-io)
11 crash capture      panic hook with the recent-lines ring
12 metrics            registry (metrics-lib)
13 audit              chain opened, verified, resumed (audit-trail)
14 registry           kernel events (registry-io)
15 kernel             urgent line, timer thread, worker supervision, health
16 subsystems         the application's, in Position order
17 ready              on_ready, READY=1 to systemd; `start` returns
```

### 7.2 Shutdown (reverse; the kernel goes last)

```
 1 Draining           phase and urgent line; STOPPING=1 to systemd
 2 subsystems         reverse of their actual start order, each within the deadline minus the member reserve
 3 Stopping           urgent line; workers joined, timer thread stopped
 4 registry           handlers cleared and waited for
 5 audit              flushed and synced
 6 metrics            final export when configured
 7 secrets            vault dropped (wiped)
 8 signals            intake uninstalled
 9 logging            shutdown report, exit code, flush
10 Stopped            run returns; the guard is dropped with System, releasing the lock
```

A subsystem past the deadline is abandoned (threads cannot be killed safely), reported, and the process exits with code 79 (D-017).

---

## 8. API outline

Item ids are used by the roadmap and the tests. Signatures, errors, and examples are settled in [`../docs/API.md`](../docs/API.md) (the contract from the 0.2.0 design lock; its item index is the full list).

### 8.1 Tier 1

| Id | Item | Version |
|---|---|---|
| SUI-01 | `pub const VERSION: &str` | 0.1 |
| SUI-02 | `start!()` / `start!(name)`: boot with defaults, the application's build info, and the configuration file found by name (a macro, D-008) | 0.3 |
| SUI-03 | `System::run(self) -> ExitCode`: park until stop, shut down in order | 0.3 |
| SUI-04 | Typed handles on `System` and `Foundation`: `build`, `instance`, `guard_report`, `paths`, `env`, `config`, `log`, `metrics`, `clock`, `ids`, `audit`, `events`, `vault`, `kernel` (each behind its feature) | 0.3 to 0.5 |
| SUI-05 | `Error::report(&self) -> ExitCode`: the boot error to stderr, the exit code back (D-009) | 0.3 |
| SUI-06 | `build::emit()` for the application's `build.rs`; `build_info!()` | 0.3 |

```rust
fn main() -> std::process::ExitCode {
    let system = match system_suite::start!() {
        Ok(system) => system,
        Err(error) => return error.report(),   // what failed, at which step, and what to do
    };
    system_suite::log::info!(system.log(), "ready");
    system.run()
}
```

### 8.2 Tier 2

| Id | Item | Version |
|---|---|---|
| SUI-10 | `Builder`: `build_info`, `scope`, `portable`, `env_file`, `config_file`, `config_str`, `clock`, `disable(member)`, `process_wide`, `crash_capture`, `single_instance`, `shutdown_deadline`, `member_reserve`, `register(subsystem)`, `on_ready`, `on_vault`, `start`, `boot` | 0.3 to 0.5 |
| SUI-11 | `System::request_stop(&self, reason) -> bool`; `System::reload(&self) -> Result<u64, Error>` | 0.4 |
| SUI-12 | `System::health(&self) -> HealthReport`, `System::status(&self) -> Status` (D-022) | 0.4 |
| SUI-13 | `lifecycle::Subsystem` trait (`name`, `position`, `start(&Link)`, `stop(Deadline)`, `health`, `reload -> Commit`), `Position`, `Link` with its `Mailbox` and error hook, `Failure`, `Commit` | 0.4 |
| SUI-14 | `runtime::Runtime`, `Worker` (restart policy, backoff, budget, criticality, heartbeat), `WorkerContext`, `WorkerHandle`; timers `Runtime::every`, `Runtime::after` | 0.4 |
| SUI-15 | `config::Config` (backend-neutral view, D-018), `FromConfig`, `Secret` | 0.4 (`Secret` 0.5) |
| SUI-16 | `exit` codes (D-017) | 0.3 |

### 8.3 Tier 3

| Id | Item | Version |
|---|---|---|
| SUI-20 | `Foundation`: the typed handles, for frameworks that embed the suite | 0.3 |
| SUI-21 | `kernel::UrgentLine`, `Level`, `Watch`, `Notice` | 0.4 |
| SUI-22 | `Snapshot<T>` and per-worker `Reader<T>` for reloadable tables | 0.4 |
| SUI-23 | `Error` (`#[non_exhaustive]`), `ErrorKind`, `Member`, `Step`: the failing step, the member, and the "do" sentence | 0.3 |
| SUI-24 | Re-exported member modules: `sys`, `path`, `env`, `config::backend`, `log`, `metrics`, `signal`, `clock`, `id`, `audit`, `registry`, `global`, `rand`, `vault` | as each member is wired |
| SUI-25 | `Builder::boot` and `Boot`: staged start for frameworks (D-013) | 0.4 |
| SUI-26 | `System::wait_for_stop`, `System::shutdown` | 0.3 |
| SUI-27 | `kernel::Kernel` control handle, `lifecycle::{Phase, StopReason, Deadline}` | 0.3 |
| SUI-28 | `kernel::Event` and `Foundation::events` | 0.5 |
| SUI-29 | `AuditLog` | 0.5 |
| SUI-30 | `system_suite::foundation()` (feature `globals`) | 0.5 |

---

## 9. Configuration

- One file per application (`<name>.conf`, config-lib's `conf` format, D-034), found by the order: explicit path, `<NAME>_CONFIG` environment variable, the application's configuration directory (path-lib), the working directory (only when not running privileged, D-020).
- The file's owner and permissions are checked before it is parsed (D-020).
- One section per member (`[config]`, `[guard]`, `[log]`, `[crash]`, `[metrics]`, `[kernel]`, `[ids]`, `[audit]`, `[secrets]`), each optional with secure defaults; summary in `docs/API.md`, every key in `docs/CONFIGURATION.md` (0.5).
- Secret values are references only (FR-16): `env:NAME`, `file:/path`, `vault:name` (D-025).
- No implicit environment overrides of individual keys (D-021).
- The loaded configuration is immutable and read through a `Snapshot`; reload prepares every change, then swaps the whole snapshot and commits, or changes nothing (D-019).

---

## 10. Threats

| Id | Threat | Defence | Test | Cost [est.] |
|---|---|---|---|---|
| T1 | a second copy of the service corrupts shared data | process guard before any data file is opened | start two copies; the second fails with `AlreadyRunning` | one lock at boot |
| T2 | secrets in configuration files or logs | references only; plain text refused by default; redaction in log sinks | a config with a plain password is refused; a logged secret appears as `[redacted]` | resolution at boot only |
| T3 | log, audit, or crash files readable by other users | umask and owner-only directories before any file is created | created files have mode 0640 or stricter on Unix and an owner-only ACL on Windows | none |
| T4 | a half-started process serving traffic | boot completes before `start` returns; outward-facing work is the application's last step | inject a failure at each boot step; nothing stays started | none |
| T5 | data loss on crash (buffered logs and audit) | crash capture flushes before the process ends | panic in a worker; the last log line and the crash report exist | flush time at crash only |
| T6 | a stuck subsystem prevents shutdown forever | per-step deadlines; abandonment with a distinct exit code | a subsystem that never returns from `stop` | none |
| T7 | an invalid configuration reload takes the service down | validate, then swap; refuse and keep the old snapshot | reload with a broken file; the old values stay | reload time only |
| T8 | signal races during boot (a stop during step 6) | signals hooked early, acted on at the next step boundary | send SIGTERM at each boot step | none |
| T9 | crash reports leaking secrets | the report contains only build and instance info, the panic message and location, and redacted log lines | panic with a secret in scope; the report does not contain it | none |

---

## 11. Architecture

```
src/
  lib.rs          crate docs, start!, re-exports, VERSION, Member, Step
  system.rs       System, Foundation
  builder.rs      Builder, Boot
  boot.rs         the boot order and its failure unwinding
  shutdown.rs     the shutdown order and deadlines
  error.rs        Error, ErrorKind (#[non_exhaustive]), exit codes
  snapshot.rs     Snapshot, Reader
  config.rs       configuration finding, permission check, sections, FromConfig, secret references
  audit.rs        AuditLog
  kernel/
    mod.rs        Kernel control handle, stop decisions
    urgent.rs     UrgentLine (T0, T1)
    signals.rs    signal translation (signal-mod)
    health.rs     aggregation, readiness, sd_notify, watchdog heartbeat
    crash.rs      panic hook, crash report
    events.rs     Event, registry publication
  lifecycle/
    mod.rs        Phase, StopReason, Deadline, the transition table
    subsystem.rs  Subsystem, Position, Link, Mailbox, Failure, Commit
  runtime/
    mod.rs        Runtime handle
    worker.rs     supervised workers, Restart, Backoff
    timer.rs      timers (a min-heap, D-046)
```

---

## 12. Ecosystem and dependencies

### 12.1 Uses (by published version, never `path`)

Section 4. Each member is added in the version that first needs it (ROADMAP).

### 12.2 Needs from members (requests to file in their repositories)

Filed on 2026-10-22, after reading every member's public API at the design lock.

| Member | Need | Needed by | Request |
|---|---|---|---|
| sys-lib | 0.3 boot kit (build info, instance info, guard, `is_elevated`, `Error`) as the suite uses it; `keep_open_files()` turned out not to be needed (a target and minimum of 0 change nothing) | suite 0.3 | [sys-lib#2](https://github.com/jamesgober/sys-lib/issues/2) (done: sys-lib 0.3.0) |
| signal-mod | ~~a runtime-free back-end with no third-party dependencies~~: withdrawn, its dependencies are accepted (D-032) | - | [signal-mod#3](https://github.com/jamesgober/signal-mod/issues/3) (closed) |
| signal-mod | Windows service control: stop, shutdown, preshutdown, paramchange, a user code for the dump, status reporting (D-014, answers OD-6) | suite 0.5 | [signal-mod#4](https://github.com/jamesgober/signal-mod/issues/4) |
| log-io | a bounded recent-lines sink for crash reports | suite 0.3 | [log-io#1](https://github.com/jamesgober/log-io/issues/1) (done: log-io 2.1.0, `sink::RecentLines`) |
| log-io | a file mode option for `FileSink` and `RotatingFile` (T3) | suite 0.3 | [log-io#2](https://github.com/jamesgober/log-io/issues/2) (done: log-io 2.1.0, `mode`) |
| path-lib | owner-only directories on Windows (T3) | suite 0.3 | [path-lib#1](https://github.com/jamesgober/path-lib/issues/1) (done: path-lib 1.1.0, `ensure_private` / `create_private`) |
| config-lib | remove the unused `serde` and trim the rest where it is cheap; a cleanup, not a blocker (D-034) | when convenient | [config-lib#4](https://github.com/jamesgober/config-lib/issues/4) |
| audit-trail | resume a verified chain from a file, owner-only files, `sync`, a clock that never goes back | suite 0.5 | [audit-trail#3](https://github.com/jamesgober/audit-trail/issues/3) |
| noml-core | none: OD-2 picked config-lib (D-034); NOML/TOML can come later through config-lib | - | not filed |

signal-mod's coverage was confirmed from its 2.0.0 source (the 0.1.0 request "confirm it exposes the signals of section 6.4"): SIGTERM, SIGINT, SIGQUIT, SIGHUP, SIGUSR1 on Unix and Ctrl+C, Ctrl+Break, console close, logoff, and shutdown on Windows, all without a runtime; Windows service control and the dependency-free back-end are missing and requested above.

Observed and not requested (the suite works around them without copying code): id-forge's `Snowflake::next_id` panics on clock skew (the suite calls `try_next_id`); env-lib's `Env::export` is `unsafe` (the suite never changes the process environment); metrics-lib's global registry is set once (the suite installs it only in process-wide mode, D-010).

### 12.3 Planned packages that plug in

- **Error suite** (planned): the kernel exposes an error hook in `Link` (`Link::report_error`); when the error suite is published, a feature `errors` wires its service, and crash capture reports coded events.
- **Security signals** (planned): the kernel exposes the shared threat state (urgent level `Lockdown`, revocation epochs); a feature `security` wires the bridge when published.

### 12.4 Dependency register

Every third-party crate the suite brings, and why it is acceptable (D-032). Reviewed 2026-10-22 on crates.io; each is re-checked when its version changes, and `cargo audit` and `cargo deny` run on every CI build.

| Crate | Brought by | Feature | Maintainer and status | Why accepted | Watch |
|---|---|---|---|---|---|
| `libc` | signal-mod (Unix) | `signals` (default) | rust-lang; released monthly; among the most used crates | the standard Unix bindings; writing our own declarations adds risk, not safety | none |
| `windows-sys` 0.61 | signal-mod (Windows) | `signals` (default) | Microsoft; generated from Windows metadata | the official Windows bindings; essential for console and service events | key-vault pins 0.59: two versions in a `full` build (cargo-deny warns) |
| `signal-hook`, `signal-hook-registry` | signal-mod (Unix) | `signals` (default) | long-maintained (0.4.5, October 2026); used by most of the ecosystem's signal handling | async-signal-safe handler registration is subtle; this is the reviewed, widely tested implementation | none |
| `thiserror` (+ `proc-macro2`, `quote`, `syn` at build time) | config-lib | `config` (default) | dtolnay; very widely used | derive only; no run-time code of its own | config-lib#4 may remove it |
| `serde` (+ `serde_derive` at build time) | config-lib | `config` (default) | dtolnay | unused by config-lib's code; compile time only cost | config-lib#4 removes it |
| `dashmap` 6 | config-lib | `config` (default) | maintained; 6.x has no open advisory (an old 5.0 soundness advisory was fixed in 5.1) | config-lib's path cache | brings `hashbrown`, `lock_api`, `parking_lot_core`, `crossbeam-utils` |
| `arc-swap` | config-lib, key-vault | `config` (default), `secrets` | vorner (as signal-hook); maintained | lock-free pointer swap | none |
| `sha2` (+ `digest`, `block-buffer`, `crypto-common`, `cpufeatures`, `cfg-if`) | audit-trail | `audit` | RustCrypto; audited, constant-time where it matters | the audit chain's hash; never written in-house | none |
| `blake3` | audit-trail, key-vault | `audit-blake3`, `secrets` | the BLAKE3 authors | alternative audit hash; key-vault's normalisation | none |
| `subtle`, `zeroize`, `getrandom` 0.2 | key-vault | `secrets` | dalek-cryptography, RustCrypto, rust-random | constant-time comparison, wiping, OS randomness for the vault | getrandom 0.2 is the older line |
| `log` 0.4 | log-io | `log-bridge` (opt-in) | rust-lang; the ecosystem's logging facade | interop: records from crates that log through the facade reach the suite's logger. Accepted by James, 2026-10-28 (D-056) | none |
| `tracing-core`, `tracing-subscriber` 0.3 (default features off, `std` only; + `once_cell`) | log-io | `log-tracing` (opt-in) | tokio-rs; the ecosystem's tracing core | interop: `TracingLayer` writes `tracing` events through log-io. Accepted by James, 2026-10-28 (D-056) | `tracing-subscriber`'s `registry` feature is not enabled (D-057) |
| `tracing` 0.1 (default features off, `std`; + `pin-project-lite`) | metrics-lib | `metrics-tracing` (opt-in) | tokio-rs | interop: span-wrapped timing. Accepted by James, 2026-10-28 (D-056) | none |
| `noml` (pinned `=0.9.2` by config-lib) + `indexmap` 2 (+ `equivalent`, `hashbrown` 0.17, `thiserror` 2 at build time) | config-lib | `config-toml`, `config-noml` (opt-in) | James Gober (noml) and the indexmap maintainers | James's own crate; TOML and NOML configuration files. Accepted by James, 2026-10-28 (D-056) | a second `hashbrown` and `thiserror` version in a build with both (cargo-deny warns) |
| `loom` 0.7 | the suite's tests | `cfg(loom)` development builds only | tokio-rs; the standard permutation tester | model tests of the urgent line and worker heartbeats; never in a build of the crate. Accepted by James, 2026-10-28 (D-056) | its own tree (`generator`, `scoped-tls`, `tracing`) exists only in `--cfg loom` test builds |

Not accepted: any async runtime (tokio, async-std) in any feature, including signal-mod's default `tokio` feature and metrics-lib's `async` (D-005); `notify` (config-lib's default `hot-reload`; the suite reloads by signal or call, so it would be an idle watcher thread for nothing).

Held by James on 2026-10-28 (D-056): these pass-throughs wait on first-party crates that cover the same ground, and are not added until then.

| Crate | Would be brought by | Pass-through |
|---|---|---|
| `serde_json` | config-lib, metrics-lib | `config-json`, `metrics-otlp`, `metrics-json` |
| `quick-xml` | config-lib | `config-xml` |
| `regex` | config-lib | `config-validation` |

Not accepted in the 2026-10-28 decision (only the crates above the line were): `sysinfo` (the BSDs only), which metrics-lib's `system` feature brings, so `metrics-system` and `[metrics] process` stay out (D-056).

### 12.5 Member features passed through

A member's optional features are reachable without depending on the member directly: the suite declares `<suite feature>-<member feature>` features that turn on the member's feature (D-036). The application can also add the member crate itself with the feature it wants; Cargo merges features of the same crate version, so the suite's re-exported module gets it too. Pass-through features are added with each member's wiring and listed in `docs/API.md`; every one is checked against the register before it is added.

---

## 13. Performance

| Measure | Target |
|---|---|
| boot to ready, default members, local file | under 5 ms [target] |
| stop to exit, nothing in flight | under 50 ms [target] |
| idle CPU | 0 (no wake-ups) |
| urgent-line check | about 1 ns [est.] |
| memory added by the suite | under 256 KiB [target] |

Measured in 0.6 with criterion and a boot benchmark harness; published in `docs/PERFORMANCE.md`.

---

## 14. Testing

- Unit tests for every function and every error path.
- Boot tests: every member combination the features allow (default, full, none, and each member alone) boots and shuts down cleanly on Linux, macOS, and Windows.
- Failure injection at every boot step (T4) and every shutdown step (T6).
- Signal tests at every lifecycle state (T8).
- Crash tests: panic in the main thread, in a worker, in a subsystem's `stop` (T5, T9).
- Reload tests (T7).
- Soak: 8 hours of start, reload, and stop cycles with no growth in memory, descriptors, or threads.
- Counting-allocator tests for the urgent-line checks and health reads (no allocation).
- `loom` for the urgent line and worker heartbeats.

---

## 15. Documentation deliverables

README (Tier 1 first), `docs/API.md` (metrics-lib format), `docs/ARCHITECTURE.md`, `docs/BOOT-ORDER.md` (the two orders with the reasons), `docs/CONFIGURATION.md` (every section and default), `docs/PERFORMANCE.md`, `docs/SECURITY.md`, `docs/PLATFORM-NOTES.md`, `docs/STABILITY-1.0.md`, CHANGELOG, release notes per version.

---

## 16. Versioning a suite

- A member's patch release needs no suite release.
- A member's minor release that changes something the suite re-exports is a suite minor release.
- A member's major release is a suite minor release before 1.0 and a suite major release after 1.0, unless the suite hides the change completely.
- The suite's MSRV is the highest MSRV among its enabled-by-default members; raising it is a major change after 1.0.

---

## 17. Open decisions

| Id | Question | Status | Recommendation or answer |
|---|---|---|---|
| OD-1 | Where does the kernel live: built here as a module, or service-kernel cleaned (no tokio, current first-party versions) and published as its own crate? | **decided by James, 2026-10-22 (D-033)** | **Built here** (module `kernel`, feature `kernel`), from the design in sections 6 and 7; split into its own crate only when a second consumer needs the kernel without the suite (James's split rule). Reading service-kernel at the design lock strengthened this: its workers, shutdown, drain, and signals all need tokio and its messages all go through lang-lib, so "cleaning" it means rewriting its runtime half; its runtime-free shapes were adopted here instead. An earlier note leaned towards service-kernel; that was before its code was read. |
| OD-2 | Configuration backend: noml-core (zero dependencies, unpublished) or config-lib 1.1 (eight formats, four third-party dependencies)? | **decided by James, 2026-10-22 (D-034)** | **config-lib**, as published, with its built-in `conf` format (`[section]`, `key = value`). Its dependencies are accepted (section 12.4); [config-lib#4](https://github.com/jamesgober/config-lib/issues/4) is a later cleanup. config-lib and the suite are updated together when config-lib grows (NOML/TOML through noml-core, for example); the suite's API does not change (D-018). |
| OD-3 | Is audit a default member? audit-trail brings sha2 or blake3. | **decided by James, 2026-10-22 (D-035)** | **Opt-in.** Not because of sha2 (accepted, section 12.4) but because an audit chain is a file every run of a CLI tool or small service would create and verify for nothing; WASP and Hive DB enable it. Revisit at 0.8. |
| OD-4 | Does the suite install the panic hook by default? | **decided (D-016)** | Yes, with `crash_capture(false)` to opt out, and no hook in `process_wide(false)` mode. |
| OD-5 | Re-export names: `system_suite::log` shadows the `log` crate name in user code that imports both. | **decided (D-015)** | Keep `log` (and `rand`); document `use system_suite::log as slog;`. |
| OD-6 | Windows service control (start, stop, pause from the service manager) | **decided (D-014)** | Through **signal-mod**, not sys-lib: sys-lib's charter is read-only system facts and its spec places signal handling in signal-mod; service control codes are signal-like events for the same coordinator. Requested as [signal-mod#4](https://github.com/jamesgober/signal-mod/issues/4); suite 0.5. |

---

## 18. Ideas

- **Boot timeline:** record the duration of each boot step and expose it in health and the first log lines, so slow starts are visible without a profiler.
- **Shutdown report:** at exit, one log line per subsystem with its stop duration and whether it met the deadline.
- **Defence idea (0.2):** hypothesis: a configuration file that became world-writable since the last boot is a sign of tampering; method: the loader checks the file's permissions and owner before reading and refuses a group- or world-writable file unless explicitly allowed; expected cost: one `stat` at boot. **Adopted into the 0.4 design (D-020, `ErrorKind::ConfigInsecure`).**
- **Defence idea (0.3):** hypothesis: someone with write access to the parent of the log or runtime directory can swap the directory for one they control between two boots, or loosen its permissions, and the service would keep writing its logs and lock there; method: at boot, after `ensure_private`, check that the existing log and runtime directories are still private (Unix: no group or other bits and owned by the process's user; Windows: a protected access control list naming only the user, `SYSTEM`, and `Administrators`) and refuse when they are not, unless allowed; expected cost: one `stat` or one access-control-list read per directory at boot, none at run time. Needs a read-back API in path-lib (a `check_private`).
- **Defence idea (0.2, design lock):** hypothesis: whoever can write the application's `.env` file can redirect the whole configuration by setting `<NAME>_CONFIG` there, to a file they own; method: a `<NAME>_CONFIG` that came from the `.env` file (not from the process environment) must pass the same owner and permission check as the configuration file, the `.env` file itself is checked the same way before it is read, and a redirect is logged with where it came from; expected cost: two `stat` calls at boot, none at run time. Test: a group-writable `.env` that sets `<NAME>_CONFIG` is refused. **Adopted in 0.4** (the `.env` file is checked before it is read; the redirect is logged as "named by the .env file").
- **Defence idea (0.4):** hypothesis: a permission check on a path and the read that follows are two system calls, so whoever can write the directory can swap the checked file for another between them (a time-of-check to time-of-use race); method: open the configuration file first and check the owner and mode of the opened handle (`fstat`), so the file checked is the file read; expected cost: none measurable (one `fstat` instead of one `stat`). **Adopted in 0.4 (D-049).** The `.env` file is still checked by path, because env-lib opens it itself; the residual window needs write access to the configuration directory, which the directory's own permissions guard.
- **Defence idea (0.5):** hypothesis: whoever can write the audit file can replace it with a shorter chain, or with an empty one, that still verifies (verification proves the records present are linked, not that none were removed from the end); method: keep an anchor (the last record's id and hash) outside the chain file, in the state directory, owner-only, updated at every sync and at shutdown, and refuse a chain whose tail is behind the anchor (`AuditTampered`); expected cost: one small write per sync (at most one per second with `sync = "flush"`), one read at boot. Not adopted in 0.5; candidate for 0.7 (hardening).

---

## 19. Decision log

| Id | Date | Decision | Reason |
|---|---|---|---|
| D-001 | 2026-10-22 | Name `system-suite`, in a new `suite-collection` folder | James's choice: the portfolio's `-suite` suffix marks a complete package; this is the complete foundation |
| D-002 | 2026-10-22 | The suite adds wiring, not features: a missing capability is added to the member that owns it, in its own pull request | members stay single-purpose and reusable without the suite |
| D-003 | 2026-10-22 | Every member behind its own feature; members added by published version only | compile only what is used; each repository builds alone in CI |
| D-004 | 2026-10-22 | MSRV 1.89 (sys-lib's), edition 2024 | the highest MSRV among default members |
| D-005 | 2026-10-22 | No async runtime; `forbid(unsafe_code)` | portfolio rule; OS calls live inside members |
| D-006 | 2026-10-22 | Signals go to the kernel only; the registry declares, the urgent line delivers | a single owner for process-wide decisions; urgent delivery must not depend on an event loop that can stall |
| D-007 | 2026-10-22 | Boot completes before `start` returns; shutdown runs in reverse with deadlines; the kernel stops last | no request or job can reach a half-started or stopping process (fail closed) |
| D-008 | 2026-10-22 | The Tier 1 entry point is the macro `start!()` / `start!(name)`, not `fn start(name)` | sys-lib's `build_info!()` must expand in the application crate to see the application's build values; a function inside the suite would record the suite's own name and version. The macro also defaults the name to the package name. Everything it does is available through `Builder`. |
| D-009 | 2026-10-22 | One crate-wide `Error` (kind, step, member, subsystem, action, member error as `source()`); `Error::report()` writes it to stderr and returns the exit code | a boot failure before logging opens has nowhere else to go; `start` itself never prints, so tests and embedders stay quiet |
| D-010 | 2026-10-22 | `Builder::process_wide(bool)`, default true: false installs no signal intake, panic hook, umask, limit change, or member global | services want the globals (member macros work without a handle); tests and embedding hosts need several systems per process; metrics-lib and log-io globals are set once |
| D-011 | 2026-10-22 | Subsystem order: a declared `Position` (u16) with registration order for ties; the stop order is the exact reverse of the actual start order | FR-8 (declared, not coded); a total order is explicit, cheap, and cannot form a cycle. service-kernel's dependency-name sorting was considered; it can be added later as a default trait method without breaking anything. |
| D-012 | 2026-10-22 | Boot order revised (docs/BOOT-ORDER.md): paths, environment, and configuration are read before the guard; the guard comes before anything is created and before the first thread; signals are hooked right after the guard; secrets and directories follow; `STOPPING=1` is sent at the start of shutdown | the guard's settings live in the configuration; signal-mod starts a thread and the umask is process-wide; T8 needs signals hooked early; systemd expects `STOPPING=1` when stopping begins |
| D-013 | 2026-10-22 | `Builder::boot()` returns a `Boot` (foundation and kernel up, subsystems started one by one, `ready()` finishes) | frameworks such as WASP build later phases from earlier phases' results; a pre-registered list cannot express that |
| D-014 | 2026-10-22 | signal-mod must provide a runtime-free back-end with no third-party dependencies before the suite's default `signals` feature ships (a 0.3 blocker), and Windows service control (0.5). The kernel maps `Hangup` per OS (Windows: stop; Unix: reload). Answers OD-6. | the zero-third-party rule for default features (signal-mod's agnostic back-end brings signal-hook, libc, windows-sys); the suite forbids `unsafe`, so service control belongs in the member that owns OS events; sys-lib's charter excludes it |
| D-015 | 2026-10-22 | OD-5: keep the re-export names `log` and `rand`; document `use system_suite::log as slog;` | the names match the plan's module list and the members' purpose; the clash only matters to projects that also use those crates, and an alias solves it |
| D-016 | 2026-10-22 | OD-4: the panic hook is installed by default (`crash_capture(false)` and `process_wide(false)` opt out); at most `[crash] max_reports` (8) reports per process | losing the last log lines at a crash is the failure this package exists to prevent; the cap stops a panic loop from filling the disk |
| D-017 | 2026-10-22 | Exit codes follow BSD `sysexits.h` (64, 65, 69, 70, 73, 74, 75, 77, 78), plus 79 for a missed shutdown deadline; a boot interrupted by a stop signal exits 0 | supervisors and scripts can tell configuration errors, a second copy, and crashes apart; systemd treats only 0 as a clean stop |
| D-018 | 2026-10-22 | `config::Config` is a backend-neutral view (typed `get`/`require`/`get_or` by dotted path, a sealed `FromConfig`); the backend is re-exported as `config::backend` | OD-2 is open, and a later second format (NOML/TOML) must not break callers |
| D-019 | 2026-10-22 | Reload is two-phase: members and subsystems prepare and return a `Commit`; the snapshot is published and the commits run only if every preparation succeeded | the invariant "a reload swaps the whole configuration or changes nothing" across independent subsystems |
| D-020 | 2026-10-22 | The configuration file's owner and permissions are checked before parsing (`ConfigInsecure`); the working directory is not searched when running elevated | adopts the 0.2 defence idea; a privileged service should never pick up a file from a directory an attacker may control |
| D-021 | 2026-10-22 | No implicit environment overrides of individual configuration keys; values that come from the environment say so with an `env:` reference | one visible source of truth per value (FR-14); implicit overrides let anything that can set a variable change any setting silently |
| D-022 | 2026-10-22 | Health: `Status` (Copy), `Health` per component, `HealthReport` with `ready()` and `live()`; `status()` never allocates; a non-critical worker can make the system at most `Degraded` | the counting-allocator requirement for health reads; readiness and liveness probes need different answers; a housekeeping thread should not fail a readiness probe |
| D-023 | 2026-10-22 | Workers: `Restart::{Never, OnFailure, Always}`, exponential backoff with jitter (100 ms to 30 s), a budget (5 in 60 s), `critical` to stop the process when the budget is exhausted, an optional heartbeat; the body is `FnMut` so it can run again | an `Err` return is a failure just as a panic is ("on panic" was too narrow); jitter prevents synchronised restarts; adopted from service-kernel's runtime-free policy types |
| D-024 | 2026-10-22 | Default shutdown deadline 25 s, member reserve 2 s | below Kubernetes' default 30 s grace and systemd's 90 s, so the suite reports what was late before it is killed; the reserve protects the audit sync and the log flush from a slow subsystem |
| D-025 | 2026-10-22 | Secret references (`env:`, `file:`, `vault:`) need the `secrets` feature; without it any reference is refused. Keys whose last segment marks a secret must hold a reference unless `[secrets] allow_plain = true` | fail closed: a reference must never be mistaken for a literal; the key list is log-io's common-secret list, so logs and configuration agree on what is secret |
| D-026 | 2026-10-22 | Member facts corrected at the design lock: signal-mod's runtime-free back-end has three third-party dependencies; config-lib's are listed with what each does; noml-core is an empty scaffold; log-io and mod-rand are Apache-2.0 only; clock-lib 1.1.1 is in progress | the 0.1.0 table was written before the members' code was read |
| D-027 | 2026-10-22 | A member disabled at run time keeps its accessor (a logger with no sinks, defaults, an empty environment); `Audit`, `Registry`, and `Secrets` handles then fail with `Disabled` | the build decides which methods exist; run-time switches must not change the API |
| D-028 | 2026-10-22 | `Foundation::ids()` is a Snowflake generator configured from `[ids] worker`; the suite never calls `next_id` | entity ids need a per-instance worker id, which is configuration; `next_id` panics on clock skew |
| D-029 | 2026-10-22 | Kernel deadlines and timers always use the OS monotonic clock; `Builder::clock` changes only what `Foundation::clock` returns | a test clock must never stall a shutdown |
| D-030 | 2026-10-22 | systemd notification through a std Unix datagram socket: `READY=1`, `STATUS=`, `WATCHDOG=1` only while not `Unhealthy`, `STOPPING=1` at the start of shutdown | no FFI needed; a wedged process stops sending keep-alives and systemd restarts it |
| D-031 | 2026-10-22 | Secret-reference resolution (FR-16) and its T2 tests move from 0.4 to 0.5; in 0.4 any reference is refused | resolution goes through key-vault (D-025), which the roadmap wires in 0.5; refusing is fail-closed, so nothing insecure ships in between |
| D-032 | 2026-10-22 | James: the zero-third-party-dependency rule is replaced, for this project, by judgement. Each dependency is reviewed (maintainer, maintenance, `unsafe` and soundness record, transitive tree, MSRV, licence) and recorded in the dependency register (section 12.4); no async runtime stays (D-005). signal-mod's signal-hook, libc, and windows-sys are accepted, which withdraws the first half of D-014 (signal-mod#3 closed) and removes that 0.3 blocker | the members were built zero-dependency, but some dependencies are necessities (OS bindings, audited cryptography, async-signal-safe handling); a suite that shuts features off to follow a count serves no one |
| D-033 | 2026-10-22 | OD-1 (James): the kernel is built here, as the `kernel` module | as recommended; a split into its own crate waits for a second consumer that needs the kernel without the suite |
| D-034 | 2026-10-22 | OD-2 (James): config-lib is the configuration backend, as published, with the `conf` format; `hot-reload` (notify) off | config-lib was meant for this; its dependencies are accepted under D-032; config-lib and the suite are updated together later |
| D-035 | 2026-10-22 | OD-3 (James): audit is opt-in; sha2 is the default hash, blake3 through `audit-blake3` | sha2 is not the concern (D-032); a chain file per run is, for tools and small services |
| D-036 | 2026-10-22 | Member features are passed through as `<suite feature>-<member feature>` features (for example `metrics-http`, `log-tracing`, `config-json`), each checked against the dependency register; no pass-through brings an async runtime | a project gets a member's optional feature without a second dependency line; adding the member directly still works through Cargo's feature merging |
| D-037 | 2026-10-22 | James: lifecycle and runtime are built here too, as the `lifecycle` and `runtime` modules beside `kernel` (one `kernel` feature for all three); `Runtime` is its own handle (`Foundation::runtime()`), and workers and timers moved off `Kernel` onto it | his foundation plan names kernel, runtime, and lifecycles as separate parts; one module per responsibility; separate crates wait for a consumer that needs them without the suite (the same rule as D-033); one feature because the kernel cannot run without the other two |
| D-038 | 2026-10-23 | `Foundation::build()` returns `Option<&'static sys::BuildInfo>`: `None` when the builder was not given the build info | sys-lib's `BuildInfo` can only be built by `build_info!()` in the application crate; a stand-in built inside the suite would carry the suite's own name and version and mislabel every log line. `start!` always sets it |
| D-039 | 2026-10-23 | `Builder::crash_capture` belongs to the `kernel` feature, not `log` | the hook is the kernel's and writes its report without a logger; with `log` it also logs and flushes the panic |
| D-040 | 2026-10-23 | The `log-bridge` and `log-tracing` pass-through features wait: they bring `log`, `tracing-core`, and `tracing-subscriber`, which are not yet in the dependency register; `time-serde` (serde, in the register) ships in 0.3 | no third-party crate is added before its review is recorded and accepted (D-032) |
| D-041 | 2026-10-23 | Logging before configuration exists (0.3): level info, standard error in human form on a terminal and JSON otherwise, JSON lines in `<logs>/<name>.log` rotated daily with 14 files kept, common secret keys redacted; `[log]` settings arrive with configuration in 0.4 | secure, useful defaults; one file per day for two weeks is a common retention |
| D-042 | 2026-10-23 | SIGHUP and SIGUSR1 are received from 0.3 on: SIGHUP logs "nothing to reload" until configuration exists, SIGUSR1 logs the phase and the instance | the OS default for both is to kill the process; a service built on 0.3 must survive a `systemctl reload` or a stray dump request |
| D-043 | 2026-10-23 | Boot and attack tests run each scenario in a child process (the test binary runs itself again), with every application directory pointed into a private temporary root | signals, the panic hook, the umask, and the lock are process-wide; a scenario must not see another's state or touch the user's own directories |
| D-044 | 2026-10-23 | Threat T6 is tested in 0.3 at the mechanism level: the deadline runner abandons a step that never returns (used for the signal intake and the log flush). The subsystem-level attack test (a subsystem whose `stop` never returns) moves to 0.4 with subsystems | there is no subsystem to get stuck before 0.4; the mechanism the subsystems will use is the one tested now |
| D-045 | 2026-10-23 | The member releases 0.3 needed were made in the members, as additive minor releases: path-lib 1.1.0 adds `ensure_private` and `create_private` (owner-only on every OS; Windows through a protected access control list set at creation) instead of changing `ensure`; log-io 2.1.0 adds `sink::RecentLines` and an opt-in `mode` on its file sinks, with the default unchanged | wiring, not features (D-002); changing what `ensure` does on Windows, or log-io's default file mode, would be a behavioural change, which those crates' stability contracts reserve for a major release |
| D-046 | 2026-10-26 | Timers are a min-heap of due times on one lazily started thread, not a hierarchical timing wheel: arming and firing O(log n), cancelling one compare-and-swap (the entry is dropped when it comes due), and a wait with no timeout when nothing is due | a service has tens of timers, not millions; the heap has no fixed tick, so an idle process wakes for nothing (NFR-2) and a 30-day timer costs the same as a 1 ms one; the API is unchanged, so a wheel can replace it if a measurement in 0.6 asks for one |
| D-047 | 2026-10-26 | Each subsystem's `stop` runs on its own stop thread; once the subsystems' part of the shutdown deadline is spent (one was abandoned), each later subsystem is still asked to stop and given 100 ms from the member reserve, never past the whole deadline | "the next subsystem is stopped" (T6) must hold even after a stuck one used all of its time; 100 ms lets a well-behaved `stop` finish while the reserve still covers the log flush |
| D-048 | 2026-10-26 | No switch accepts a configuration or `.env` file that another user can change (the `[config] allow_insecure_file` key of the design lock is dropped before it shipped); a parse error names the line and column only, never the parser's message | fail closed (DIRECTIVES): the fix (`chmod 640`, `chown`) is always available to the operator and the error says so, while a switch in the file it protects would be set by the attacker it protects against; a parser message can quote the text, and the text can hold a secret |
| D-049 | 2026-10-26 | The configuration file's owner and mode are checked on the opened file (`fstat`), not on the path before opening (the 0.4 defence idea, section 18) | closes the window between the check and the read; same cost |
| D-050 | 2026-10-26 | 0.4 ships the pass-throughs that bring no third-party crate (`metrics-histogram`, `metrics-meter`, `metrics-statsd`, `metrics-http`); the ones that bring `serde_json`, `noml`/`indexmap`, `quick-xml`, `regex`, `tracing`, or `sysinfo` wait for their review (section 12.4), and `[metrics] process` waits with `metrics-system` (metrics-lib's `system` feature brings `sysinfo` on the BSDs) | no third-party crate is added before its review is recorded and accepted (D-032); the review was asked for and is not answered yet |
| D-051 | 2026-10-26 | The `loom` model tests of the urgent line and worker heartbeats wait for `loom`'s review; 0.4 tests both with std threads under contention (concurrent `set` and `raise` never undo a stop; mailboxes and watchers see every change) | `loom` is a development dependency but still a third-party crate (D-032); the std-thread tests cover the same invariants without exhaustive interleavings |
| D-052 | 2026-10-26 | With `[kernel] reload_signal` or `dump_signal` off, SIGHUP and SIGUSR1 are still received, and logged as ignored | the OS default for both ends the process (as in D-042); "off" means "do nothing", not "die" |
| D-053 | 2026-10-26 | The metrics registry is metrics-lib's global one when the system may touch process-wide state; if another part of the process set it up first, that one is used and a warning logged; otherwise (and with `Member::Metrics` disabled or `[metrics] enabled = false`) the system has its own registry, which nothing exports. The kernel's metrics are named `system_suite_*` (listed in the `kernel::metrics` module documentation) | metrics-lib's global is set once (D-010); refusing to boot because a library touched it first would punish the application for a dependency's choice, and the warning says which settings were kept; a disabled member still has its accessor (API) |
| D-054 | 2026-10-26 | In 0.4 a reload changes only the `[kernel]` deadlines live; every other suite section is logged as "takes effect at restart". The new configuration is read again from the same source (`config_str` included) | the log sinks, guard, ids, and metrics limits are built once at boot by their members, which have no live-change API yet; reporting them is honest, failing the reload for them would block every other change |
| D-055 | 2026-10-26 | The `.env` file's default place is `<config dir>/.env` through path-lib's hardened join (as in 0.3), checked like the configuration file before env-lib opens it; a `CONFIG` key in it (with or without the `<NAME>_` prefix) names the configuration file only when the process environment does not | the 0.2 design-lock defence idea; the process environment wins because it is set by whoever started the process |
| D-056 | 2026-10-28 | **James:** "accept only: loom (dev-only), noml + indexmap (my own crate), and the log / tracing-core + tracing-subscriber / tracing bridges (opt-in interop). Hold serde_json, quick-xml and regex until my own crates cover them." So 0.4 adds `log-bridge`, `log-tracing`, `metrics-tracing`, `config-toml`, `config-noml` (each opt-in) and the `loom` model tests (`cfg(loom)`, development only); `config-json`, `config-xml`, `config-validation`, `metrics-json`, and `metrics-otlp` wait on first-party crates; `sysinfo` was not accepted, so `metrics-system` stays out | James's call on the dependency register (D-032); the accepted crates are opt-in interop or his own, and none is in a default build |
| D-057 | 2026-10-28 | With `log-bridge`, boot step 10 installs log-io's `LogBridge` as the `log` crate's logger when the system may touch process-wide state; one already installed is kept and a warning logged. With `log-tracing`, the suite installs no `tracing` subscriber: `system_suite::log::compat::TracingLayer::new(foundation.log().clone())` goes into the application's own subscriber stack | the `log` logger is set once, like log-io's global (D-010); a `tracing` layer needs a span-storing subscriber (`tracing-subscriber`'s `registry`, which brings crates outside D-056), and the subscriber stack is the application's to compose |
| D-058 | 2026-10-28 | The loom model tests run with `RUSTFLAGS="--cfg loom" cargo test --lib --release --no-default-features --features kernel loom_`, in their own CI job; the atomics they check come from `crate::sync` (std's, or loom's under `cfg(all(loom, test))`), so the tests check the shipped code. Only the `kernel` feature is built because clock-lib also reads `cfg(loom)` and does not build as a dependency under it | the urgent line's word and the heartbeat are the only lock-free parts; a mutation (relaxed epoch orderings) was confirmed to fail the reader test, so the tests are not vacuous |
| D-059 | 2026-10-28 | The configuration format follows the file's extension: `.toml` and `.noml` with their feature, `conf` for anything else; `config_str` is always `conf`. A `.toml` or `.noml` file without its feature is `ConfigInvalid` naming the feature. Finding two candidates (`site.conf` and `site.toml`) in one place is `ConfigInvalid` naming both | as designed in D-018; an extension is explicit, guessing between two files is not |
| D-060 | 2026-10-28 | Audit (step 13) uses audit-trail 1.1.0, released for it as an additive minor version (verified `open_file`, owner-only files, `sync`, `MonotonicClock`). A chain that does not verify (changed, removed, reordered, or undecodable records) stops the boot with `AuditTampered`; a record cut short by a crash stops it with `Audit` unless `[audit] repair_tail` keeps the cut bytes aside. `AuditLog::path` and `next_id` return `Option` (a disabled log has neither) and `is_enabled` is added. The kernel records reloads, lockdowns, and stops; the stop at the start of the teardown, not in `request_stop`, which never blocks | the record of privileged actions must be trustworthy before the first new record; a crash is not tampering but must not be silently repaired |
| D-061 | 2026-10-28 | Kernel events (step 14, registry-io's `SyncRegistry`): phase changes, urgent changes, reloads and refusals, worker restarts and failures, subsystem stops. With `Member::Registry` disabled the registry exists and is never notified. Shutdown step 4 clears the handlers and waits for running ones, bounded by the deadline, so the last phase change (`Stopped`) is not delivered | a handler must not run while the member it writes to is closing; a disabled member keeps its accessor (API) |
| D-062 | 2026-10-28 | `system_suite::foundation()` is a global-lib `Global<Foundation>`, published at step 17 in process-wide mode (unless `Member::Globals` is disabled) and withdrawn at shutdown step 1 | for code that cannot be handed a `Foundation`; never present while the system is not serving |
| D-063 | 2026-10-28 | With `rand` (and so with `secrets`) step 2 reads 16 bytes from the OS random source and stops the boot with `Entropy` when it fails; backoff jitter comes from mod-rand's process-unique values | a blocked random source surfaces at the step that names it, not later in a token generator or the vault |
| D-064 | 2026-10-28 | Secrets (step 8): a key-vault vault built with BLAKE3 normalization off (the original bytes come back); `file:` references are read by the suite, not key-vault's `FileFetch` (it keeps a trailing newline, has no size limit, and does no checks on Windows): checked on the opened file (no group or other bit, owner the service user or root), at most 64 KiB, one trailing newline removed. Each configuration version stores its values under its own names (`config:v<N>:<key>`), so a reload resolves the new version before subsystems prepare (they can read the new secrets) without changing what the version in force reads; a refusal removes them again. The version in force and the one before keep their values; older ones and, at shutdown step 7, all are removed (wiped). A plain value under a key whose last segment marks a secret is refused (`PlainSecret`) unless `[secrets] allow_plain` | references, never secrets, in configuration (FR-16, T2); a `Secret` taken before a reload keeps working through one reload |
| D-065 | 2026-10-28 | Windows service control through signal-mod 2.1.0 (released for it, additive, feature `service`, no new crate): `System::run` on Windows in process-wide mode offers the main thread to the service control manager, reports `RUNNING`, `STOP_PENDING` with the shutdown deadline as the wait hint, and `STOPPED` with the exit code; a process not started by the manager runs in the foreground as before. Stop, shutdown, and preshutdown arrive as `Terminate`; parameter change as `User2` (reload) and control code 128 as `User1` (dump), the suite's Windows notify set | the manager requires the main thread; `Signal`'s `ALL` array is public, so no new variants (that would be a breaking change in signal-mod) |
| D-066 | 2026-10-28 | The urgent line's memory watermark moves to 0.6: it needs sys-lib 0.4.0's `mem::pressure()`, which another session is building; 0.5 does not wait for it (decided with James through the coordinating session) | a blocker outside this repository, recorded rather than worked around |
| D-067 | 2026-10-28 | `audit`, `registry`, and `secrets` imply `kernel`; `secrets` also implies `config` and `rand`. A CI job builds, lints, and tests every member feature alone on Linux, macOS, and Windows | they record into, notify from, or resolve through the kernel; every member must still compile on its own |
