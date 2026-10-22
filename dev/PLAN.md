# system-suite - Project Plan

> The design, strategy, and decision record for `system-suite`. A living document: every design decision, idea, or open question about the suite is written here or in [`ROADMAP.md`](./ROADMAP.md) before it is acted on.
>
> **Read order:** [`../REPS.md`](../REPS.md), then [`DIRECTIVES.md`](./DIRECTIVES.md), then **this file**, then [`ROADMAP.md`](./ROADMAP.md).
>
> **Status:** scaffold, `0.1.0`. Design lock is `0.2.0`.

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

Versions checked on crates.io on 2026-10-22.

| Member | Feature | Crate | Published | Third-party runtime deps | Status for the suite |
|---|---|---|---|---|---|
| System facts, boot kit | `sys` | sys-lib | 0.1.0 | none | **blocker for 0.3**: needs sys-lib 0.3 (build info, instance info, guard) |
| Paths | `paths` | path-lib | 1.0.0 | none | ready |
| Environment | `env` | env-lib | 1.0.0 | none (uses path-lib) | ready |
| Configuration | `config` | noml-core or config-lib | noml-core unpublished; config-lib 1.1.0 | noml-core: none; config-lib: thiserror, serde, dashmap, arc-swap | **open decision OD-2** |
| Logging | `log` | log-io | 2.0.0 | none | ready |
| Metrics | `metrics` | metrics-lib | 2.0.0 | none | ready |
| Signals | `signals` | signal-mod | 2.0.0 | none | ready |
| Time | `time` | clock-lib | 1.1.0 | none | ready |
| Ids | `ids` | id-forge | 1.0.0 | none | ready |
| Audit | `audit` | audit-trail | 1.0.1 | sha2, blake3 (accepted cryptography exception) | ready; default or opt-in is OD-3 |
| Registry | `registry` | registry-io | 1.1.0 | none | ready |
| Globals | `globals` | global-lib | 1.0.0 | none | ready |
| Randomness | `rand` | mod-rand | 1.0.0 | none | ready |
| Secrets | `secrets` | key-vault | 1.1.0 | subtle, getrandom, blake3, zeroize | ready; opt-in because of its dependencies |
| Kernel | `kernel` | this crate (OD-1) | - | none | built here |

`service-kernel` (local, unpublished 0.1.0) depends on mod-events and lang-lib and has optional tokio, proc-daemon, and older first-party versions. Whether the kernel is built here or service-kernel is cleaned and published is OD-1.

---

## 5. Requirements

### 5.1 Functional

| Id | Requirement |
|---|---|
| FR-1 | `system_suite::start(name)` boots every enabled member in the order of section 7 and returns a `System` only when every step succeeded. |
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
| NFR-1 | Zero third-party runtime dependencies in the default feature set, except cryptography accepted through audit-trail if OD-3 makes audit a default. |
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

The kernel is the suite's controller: it owns the lifecycle, the order, and the background work. It is the "system core" of the cores architecture: every subsystem registers with it, and registration is the subscription (nothing can miss a stop or an urgent change).

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

### 6.5 Workers and timers

- Workers are named threads with a restart policy (never, on panic, always) and a backoff; their health is a heartbeat word written only by the worker.
- Timers run on one kernel timer thread with a hierarchical timing wheel; with nothing due, the thread parks with no timeout.
- Long-running work belongs to task-io; the kernel's workers are for the services' own housekeeping (log flushing, metrics export, cache sweeps).

### 6.6 Health and readiness

- `system.health()` aggregates members and subsystems into `Healthy`, `Degraded(reasons)`, or `Unhealthy(reasons)`.
- Readiness: `READY=1`, `STOPPING=1`, and watchdog keep-alives to systemd when `NOTIFY_SOCKET` is set (std Unix datagram, no FFI); heartbeats to the watchdog (tower) when configured.

### 6.7 Crash capture

The panic hook (installed at boot unless disabled) reports the panic, flushes every member that buffers (logs, metrics, audit), writes `crash-<instance>-<time>.txt` to the application's log directory (build info, instance info, the panic message and location, the last N log lines held in a small ring), and returns so the process ends as configured (`panic = "abort"` in release). Error-suite integration (section 12.3) replaces the plain report with coded events when available.

---

## 7. Boot and shutdown order

### 7.1 Boot (listeners and other outward-facing work belong to the application and come last)

```
 1 build info         compiled in (sys-lib); the first log line
 2 instance info      id, start time, host, pid (sys-lib, id-forge, clock-lib)
 3 process guard      single-instance lock, umask, open-file limit (sys-lib)     fail: exit, nothing opened
 4 paths              application directories created with safe permissions (path-lib)
 5 environment        .env and variables (env-lib)
 6 configuration      load, resolve secret references, validate (config, key-vault)   fail: exit
 7 logging            sinks opened (log-io)
 8 crash capture      panic hook installed
 9 metrics            registry and exporters (metrics-lib)
10 audit              chain opened and verified (audit-trail)
11 registry           (registry-io)
12 kernel             urgent line, signals hooked, workers and timers, health
13 subsystems         the application's registered subsystems, in their declared order
14 ready              READY=1 to systemd, heartbeat to the watchdog; `start` returns
```

### 7.2 Shutdown (reverse; the kernel goes last)

```
 1 urgent level Draining; the application closes its doors first (its subsystem stops first)
 2 subsystems        stopped in reverse declared order, each within the deadline
 3 workers, timers   stopped
 4 registry, audit (flushed and sealed), metrics (final export), logging (flushed)
 5 guard released    the lock is released by the OS when the process ends
 6 kernel            last: exit code reported, STOPPING=1 sent
```

A subsystem past the deadline is abandoned (threads cannot be killed safely), reported, and the process exits with a distinct code.

---

## 8. API outline

Item ids are used by the roadmap and the tests. Signatures are settled in `docs/API.md` at the 0.2.0 design lock.

### 8.1 Tier 1

| Id | Item | Version |
|---|---|---|
| SUI-01 | `pub const VERSION: &str` | 0.1 |
| SUI-02 | `pub fn start(name: &str) -> Result<System, Error>`: boot with defaults and the configuration file found by name | 0.3 |
| SUI-03 | `System::run(self) -> ExitCode`: park until stop, shut down in order | 0.3 |
| SUI-04 | Typed handles: `System::{log, config, metrics, paths, env, build, instance, clock, ids, kernel}` (each behind its feature) | 0.3 to 0.4 |

```rust
fn main() -> std::process::ExitCode {
    let system = match system_suite::start("site") {
        Ok(system) => system,
        Err(error) => return error.exit_code(),   // already logged to stderr with what to do
    };
    system.log().info("ready");
    system.run()
}
```

### 8.2 Tier 2

| Id | Item | Version |
|---|---|---|
| SUI-10 | `System::builder(name) -> Builder` with `config_file`, `config_str`, `disable(member)`, `shutdown_deadline`, `crash_capture(bool)`, `register(subsystem)`, `on_ready(fn)`, `start()` | 0.3 to 0.5 |
| SUI-11 | `System::request_stop(&self, reason)`; `System::reload(&self) -> Result<(), Error>` | 0.4 |
| SUI-12 | `System::health(&self) -> Health` | 0.4 |
| SUI-13 | `Subsystem` trait: `name`, `position`, `start`, `stop(deadline)`, `health`; `Registration` link handed back with the urgent line and the error hook | 0.4 |
| SUI-14 | `kernel::Worker` (spawn with a restart policy) and `kernel::Timer` (periodic, one-shot) | 0.4 |

### 8.3 Tier 3

| Id | Item | Version |
|---|---|---|
| SUI-20 | `Foundation`: the struct of typed handles, for frameworks that embed the suite | 0.3 |
| SUI-21 | `kernel::UrgentLine` read and raise API | 0.4 |
| SUI-22 | `Snapshot<T>` and per-worker readers for reloadable tables | 0.4 |
| SUI-23 | `Error` (`#[non_exhaustive]`), with the failing boot step, the member, and the "do" sentence | 0.3 |
| SUI-24 | Re-exported member modules: `sys`, `path`, `env`, `config`, `log`, `metrics`, `signal`, `clock`, `id`, `audit`, `registry`, `global`, `rand`, `vault` | as each member is wired |

---

## 9. Configuration

- One file per application (`<name>.noml` if OD-2 picks noml-core), found by the order: explicit path, `<NAME>_CONFIG` environment variable, the application's configuration directory (path-lib), the working directory.
- One section per member (`[log]`, `[metrics]`, `[audit]`, `[kernel]`, `[guard]`), each optional with secure defaults.
- Secret values are references only (FR-16).
- The loaded configuration is immutable and read through a `Snapshot`; reload swaps the whole snapshot.

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
  lib.rs          crate docs, Tier 1 entry points, re-exports, VERSION
  system.rs       System, Builder, Foundation
  boot.rs         the boot order and its failure unwinding
  shutdown.rs     the shutdown order and deadlines
  error.rs        Error (#[non_exhaustive])
  config.rs       configuration loading, sections, secret references, snapshot
  kernel/
    mod.rs        Kernel, lifecycle state
    urgent.rs     UrgentLine (T0, T1)
    signals.rs    signal translation (signal-mod)
    worker.rs     supervised workers
    timer.rs      timing wheel
    health.rs     aggregation, readiness, sd_notify, watchdog heartbeat
    crash.rs      panic hook, crash report
    subsystem.rs  Subsystem trait, registration
```

---

## 12. Ecosystem and dependencies

### 12.1 Uses (by published version, never `path`)

Section 4. Each member is added in the version that first needs it (ROADMAP).

### 12.2 Needs from members (requests to file in their repositories)

| Member | Need | Needed by |
|---|---|---|
| sys-lib | 0.3 boot kit (build info, instance info, guard) | suite 0.3 |
| noml-core | first published version (if OD-2 picks it) | suite 0.4 |
| config-lib | the four third-party dependencies removed or made optional (if OD-2 picks it) | suite 0.4 |
| log-io | a small ring of the last N lines for crash reports, if it does not have one | suite 0.4 |
| signal-mod | confirm it exposes the signals of section 6.4 on every OS without a runtime | suite 0.3 |

### 12.3 Planned packages that plug in

- **Error suite** (planned): the kernel exposes an error hook in `Registration`; when the error suite is published, a feature `errors` wires its service, and crash capture reports coded events.
- **Security signals** (planned): the kernel exposes the shared threat state (urgent level `Lockdown`, revocation epochs); a feature `security` wires the bridge when published.

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

| Id | Question | Recommendation |
|---|---|---|
| OD-1 | Where does the kernel live: built here as a module, or service-kernel cleaned (no tokio, current first-party versions) and published as its own crate? | **Built here** at first, from the design in sections 6 and 7 (which follows the WASP and cores architecture decisions); split it into its own crate only when a second consumer needs the kernel without the suite (James's split rule). service-kernel stays as reference. |
| OD-2 | Configuration backend: noml-core (zero dependencies, unpublished) or config-lib 1.1 (eight formats, four third-party dependencies)? | **noml-core** as the default backend once published; config-lib behind a non-default `config-formats` feature for projects that need other formats. Until noml-core is published, the `config` feature waits (suite 0.4). |
| OD-3 | Is audit a default member? audit-trail brings sha2 and blake3 (the accepted cryptography exception). | **Opt-in** at first (CLI tools and small services do not need it); WASP and Hive DB enable it. Revisit at 0.8. |
| OD-4 | Does the suite install the panic hook by default? | **Yes**, with `crash_capture(false)` to opt out: losing the last log lines at a crash is the failure this package exists to prevent. |
| OD-5 | Re-export names: `system_suite::log` shadows the `log` crate name in user code that imports both. | Keep `log`; document `use system_suite::log as slog;` for projects that also use the `log` crate. |
| OD-6 | Windows service control (start, stop, pause from the service manager) | Through sys-lib (it already carries the Windows FFI); the kernel maps service stop to the stop decision. |

---

## 18. Ideas

- **Boot timeline:** record the duration of each boot step and expose it in health and the first log lines, so slow starts are visible without a profiler.
- **Shutdown report:** at exit, one log line per subsystem with its stop duration and whether it met the deadline.
- **Defence idea (0.2):** hypothesis: a configuration file that became world-writable since the last boot is a sign of tampering; method: the loader checks the file's permissions and owner before reading and refuses a group- or world-writable file unless explicitly allowed; expected cost: one `stat` at boot.

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
