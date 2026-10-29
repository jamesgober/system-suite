<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br><b>system-suite</b><br>
    <sub><sup>API REFERENCE</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;│&nbsp;</span>
        <span>API</span>
        <span>&nbsp;│&nbsp;</span>
        <a href="./BOOT-ORDER.md" title="Boot and shutdown order"><b>BOOT ORDER</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="../dev/PLAN.md" title="Plan"><b>PLAN</b></a>
    </sup>
</div>
<br>

The complete public API of system-suite 1.0, written at the `0.2.0` design lock. Every item carries the version it arrives in (**Since**) and its item id from [`../dev/PLAN.md`](../dev/PLAN.md) section 8; the published crate exposes only what is implemented, so an item marked `0.4` does not exist in `0.3`. Signatures here are the contract: a change after `0.2.0` is recorded in the decision log of the plan. The members' own types appear as they are published today (or, for sys-lib, as locked in its own `0.2.0` design); a member's major release changes them as described in [Versioning](#versioning).

Costs are estimates (marked [est.]) until they are measured in `0.6.0`; measured numbers then replace them, labelled as measured.

<sub>Copyright &copy; 2026 <strong>James Gober</strong>.</sub>

## Table of contents

- [Setup](#setup)
  - [Features](#features)
  - [Build info in `build.rs`](#build-info-in-buildrs)
- [Conventions](#conventions)
  - [Tiers](#tiers)
  - [Process-wide state](#process-wide-state)
  - [Threads](#threads)
  - [Platforms](#platforms)
- [Tier 1](#tier-1)
  - [`start!`](#start)
  - [`System::run`](#systemrun)
  - [Typed handles](#typed-handles)
  - [`Error::report`](#errorreport)
- [Crate root](#crate-root)
  - [`VERSION`](#version)
  - [`build_info!` and `build::emit`](#build_info-and-buildemit)
  - [`Member`](#member)
  - [`Step`](#step)
  - [`Snapshot` and `Reader`](#snapshot-and-reader)
  - [`foundation()`](#foundation-1)
- [`System`](#system)
  - [Accessors](#accessors)
  - [Lifecycle](#lifecycle)
  - [`request_stop`, `reload`](#request_stop-reload)
  - [`health`, `status`](#health-status)
- [`Builder`](#builder)
  - [Identity and sources](#identity-and-sources)
  - [Members](#members)
  - [Kernel settings](#kernel-settings)
  - [Subsystems and callbacks](#subsystems-and-callbacks)
  - [`start`, `boot`](#start-boot)
- [`Boot`: staged start for frameworks](#boot-staged-start-for-frameworks)
- [`Foundation`](#foundation)
- [`Error`](#error)
  - [`ErrorKind`](#errorkind)
  - [Exit codes](#exit-codes)
- [`kernel`](#kernel)
  - [`Kernel`](#kernel-1)
  - [The urgent line: `UrgentLine`, `Level`, `Watch`, `Notice`](#the-urgent-line-urgentline-level-watch-notice)
  - [Health: `Health`, `Status`, `HealthReport`](#health-health-status-healthreport)
  - [`Event`](#event)
- [`lifecycle`](#lifecycle)
  - [`Phase` and `StopReason`](#phase-and-stopreason)
  - [`Deadline`](#deadline)
  - [`Subsystem`](#subsystem)
  - [`Position`](#position)
  - [`Link` and `Mailbox`](#link-and-mailbox)
  - [`Failure`](#failure)
  - [`Commit`](#commit)
- [`runtime`](#runtime)
  - [`Runtime`](#runtime-1)
  - [Workers: `Worker`, `WorkerContext`, `WorkerHandle`](#workers-worker-workercontext-workerhandle)
  - [`Restart` and `Backoff`](#restart-and-backoff)
  - [Timers: `Runtime::every`, `Runtime::after`, `TimerHandle`](#timers-runtimeevery-runtimeafter-timerhandle)
- [`config`](#config)
  - [`Config`](#config-1)
  - [`FromConfig`](#fromconfig)
  - [`Secret`](#secret)
  - [Finding the file](#finding-the-file)
  - [Sections](#sections)
- [`AuditLog`](#auditlog)
- [Member modules](#member-modules)
- [Signals](#signals)
- [Versioning](#versioning)
- [Item index](#item-index)

## Setup

```toml
[dependencies]
system-suite = "0.6"

[build-dependencies]
# Only for build info (the build commit, time, and profile in the first log line).
system-suite = { version = "0.6", default-features = false, features = ["sys"] }
```

```rust,ignore
// build.rs
fn main() {
    system_suite::build::emit();
}
```

```rust,ignore
// src/main.rs
fn main() -> std::process::ExitCode {
    let system = match system_suite::start!() {
        Ok(system) => system,
        Err(error) => return error.report(),
    };
    system_suite::log::info!(system.log(), "ready");
    system.run()
}
```

### Features

Every member is behind its own feature, and every feature is additive: turning one off removes that member's step from the boot order and its accessor from `System` and `Foundation`, and the crate still builds (invariant: every member can be compiled out).

| Feature | Default | Member (crate) | Brings | Third-party runtime dependencies |
|---|---|---|---|---|
| `kernel` | yes | this crate: modules `kernel`, `lifecycle`, `runtime` | signals to decisions, urgent line, subsystems, workers, timers, health, crash capture, readiness | none |
| `sys` | yes | sys-lib 0.3 | build info, instance info, process guard | none |
| `paths` | yes | path-lib 1.1 | application and OS directories, owner-only where the suite writes | none |
| `env` | yes | env-lib 1 (implies `paths`) | `.env` file and variables | none |
| `config` | yes | config-lib 1.1, `conf` format (implies `paths`) | one configuration file, reload | thiserror, serde, dashmap, arc-swap (through config-lib) |
| `log` | yes | log-io 2.1 | structured logging, the crash report's recent lines | none |
| `metrics` | yes | metrics-lib 2 (counters, gauges, timers; no integrations) | the system's registry and the kernel's own metrics | none |
| `signals` | yes | signal-mod 2, runtime-free back-end (implies `kernel`) | stop, reload, dump signals; Windows service control | libc and signal-hook (Unix), windows-sys (Windows) |
| `time` | yes | clock-lib 1 | monotonic and wall clocks, a mockable `Clock` | none |
| `ids` | yes | id-forge 1 | ULID, UUID, Snowflake | none |
| `audit` | no (D-035) | audit-trail 1 with sha2 (implies `paths`) | tamper-evident audit chain | sha2 |
| `registry` | no | registry-io 1 (implies `kernel`) | kernel events delivered to callbacks | none |
| `globals` | no | global-lib 1 | process-wide typed values; `system_suite::foundation()` | none |
| `rand` | no | mod-rand 1 | OS CSPRNG and fast generators | none |
| `secrets` | no | key-vault 1 with its file and environment fetchers (implies `config`) | secret references resolved into a page-locked vault | subtle, getrandom, blake3, zeroize, arc-swap |
| `full` | no | all of the above | | as above |

Every third-party crate in this table is a reviewed choice recorded in the dependency register of [`../dev/PLAN.md`](../dev/PLAN.md) (section 12.4, D-032), and `cargo audit` and `cargo deny` check them on every CI build. No feature brings an async runtime (D-005).

### Member features passed through

A member's own optional features are available through the suite, so a project still needs one dependency line (D-036). Each is named `<suite feature>-<member feature>`, turns on the suite feature it extends, and arrives with that member's wiring:

| Feature | Turns on | Brings | Since |
|---|---|---|---|
| `time-serde` | clock-lib's `serde`: `Wall` and `Monotonic` serialise | serde | 0.3 |
| `metrics-histogram`, `metrics-meter` | metrics-lib's histogram and summary, rate meter | none | 0.4 |
| `metrics-statsd` | metrics-lib's StatsD and DogStatsD push (std UDP) | none | 0.4 |
| `metrics-http` | metrics-lib's blocking `/metrics` server and Pushgateway client (std TCP) | none | 0.4 |
| `log-bridge` | log-io's `log`: records from crates that use the `log` facade go to the suite's logger (installed at boot step 10 in process-wide mode, D-057) | log | 0.4 |
| `log-tracing` | log-io's `tracing`: `system_suite::log::compat::TracingLayer`, a `tracing-subscriber` layer that writes through log-io; the application adds it to its own subscriber (D-057) | tracing-core, tracing-subscriber | 0.4 |
| `metrics-tracing` | metrics-lib's tracing integration | tracing | 0.4 |
| `config-toml`, `config-noml` | config-lib's TOML and NOML formats: `<name>.toml`, `<name>.noml` (D-059) | noml, indexmap | 0.4 |
| `config-json`, `config-xml` | config-lib's formats of the same names | serde_json; quick-xml | waiting on first-party crates (D-056) |
| `config-validation` | config-lib's rule-based validation | regex | waiting on first-party crates (D-056) |
| `metrics-otlp`, `metrics-json` | metrics-lib's OTLP rendering, JSON rendering | serde, serde_json | waiting on first-party crates (D-056) |
| `metrics-system` | metrics-lib's process metrics (`process_*`) and host health; `[metrics] process` | sysinfo (the BSDs only) | not accepted (D-056) |
| `audit-blake3` | audit-trail's BLAKE3 hasher (selected with `[audit] hash = "blake3"`) | blake3 | 0.5 |
| `registry-async` | registry-io's `AsyncRegistry` (runtime-agnostic futures, no executor) | futures-core | 0.5 |
| `secrets-keychain` | key-vault's OS keychain fetcher | keyring; on Linux its Secret Service back-end runs async-io's reactor thread, so this one is decided against the register when key-vault is wired | 0.5 |

Not passed through: anything that brings an async runtime (signal-mod's `tokio`, metrics-lib's `async`, config-lib's `async`, key-vault's TPM stub), and config-lib's `hot-reload` (an idle watcher thread; the suite reloads by signal or call).

A pass-through marked "waiting on first-party crates" brings a third-party crate that James has held until a first-party crate covers the same ground (D-056); it is not added before then. A project can also reach any member feature directly: adding the member with the feature it wants (`metrics-lib = { version = "2", default-features = false, features = ["http"] }`) works too, because Cargo merges the features of one crate version, so `system_suite::metrics` gets it. The pass-through is the same thing without the second line and without having to match the member's version by hand.

### Build info in `build.rs`

Build info (commit, dirty flag, build time, profile, target, rustc, features) is captured by sys-lib's build-script helper in the **application's** `build.rs` and read by a macro that expands in the application crate. That is why the Tier 1 entry point is a macro: `start!()` expands `build_info!()` where the application's values are visible. Without the `build.rs` line everything still compiles; the commit and time read `"unknown"`. See [`build_info!` and `build::emit`](#build_info-and-buildemit).

## Conventions

### Tiers

| Tier | Who it is for | Items |
|---|---|---|
| 1 | every service and tool: about 80% of uses | `start!`, `System::run`, the typed handles, `Error::report` |
| 2 | services that configure the boot or add their own subsystems, workers, and timers | `Builder`, `request_stop`, `reload`, `health`, `Subsystem`, `Worker`, timers, `Config` |
| 3 | frameworks that embed the suite and libraries that plug into it | `Boot`, `Foundation`, the urgent line, `Snapshot`, `Error` details, `Event`, member modules |

Each item lists **Since**, its id, its feature, and its cost. Ids (`SUI-nn`) are the plan's; the [item index](#item-index) lists them all.

### Process-wide state

Some things are process-wide by nature: signal handlers, the panic hook, the umask, the open-file limit, and the members' global slots (log-io's global logger, metrics-lib's global registry, global-lib's store). A `System` built with the defaults installs all of them, because that is what a service wants: one process, one system, and member macros such as `log::info!("...")` and `metrics::counter!(...)` work without a handle.

`Builder::process_wide(false)` builds a system that installs none of them: no signal intake, no panic hook, no umask or limit change, no global logger or registry. Tests that start several systems in one process, and hosts that embed a system next to their own signal handling, use it. The single-instance lock is not process-wide (it is per name) and stays on unless `[guard] lock = false`.

### Threads

The suite starts no thread before the guard (boot step 6). With every default member enabled, a running system owns:

| Thread | Started by | When it wakes |
|---|---|---|
| `signal-mod-signals` (Unix only) | signal-mod | a signal arrives |
| `system-suite-timer` | the runtime | a timer is due; parks with no timeout when none is |
| one per supervised worker | `Worker::spawn` | as the worker decides |
| `system-suite-stop-<n>` | the kernel, only during shutdown | runs one subsystem's `stop` so the deadline can be enforced |

Idle CPU is zero: no thread polls (NFR-2). On Windows, console events arrive on a thread the operating system creates for each event.

### Platforms

Linux, macOS, and Windows on x86_64 and aarch64, stable Rust and MSRV 1.89. Differences that reach this API:

| Behaviour | Linux | macOS | Windows |
|---|---|---|---|
| stop signals | SIGTERM, SIGINT | same | Ctrl+C, Ctrl+Break, console close, logoff, system shutdown, service stop (D-014) |
| reload signal | SIGHUP | same | service `PARAMCHANGE` control (D-014); otherwise `System::reload` only |
| dump signal | SIGUSR1 | same | service user control code 128 (D-014) |
| umask | `0o027` by the guard | same | not applicable; the log and lock directories get a protected, owner-only access control list (path-lib 1.1) |
| readiness | `sd_notify` when `NOTIFY_SOCKET` is set | none | the service control manager's `SERVICE_RUNNING` (D-014) |

## Tier 1

### `start!`

**Since** 0.3 · SUI-02 · features: any

Boots every enabled member and the kernel in the order of [`BOOT-ORDER.md`](./BOOT-ORDER.md) and returns a `System` only when every step succeeded. On failure nothing that started is left running, and the error names the step, the member, and what to do.

```rust,ignore
#[macro_export]
macro_rules! start {
    () => { /* start!(env!("CARGO_PKG_NAME")) */ };
    ($name:expr) => { /* System::builder($name).build_info(build_info!()).start() */ };
}
// Expands to an expression of type Result<System, Error>.
```

| Parameter | Meaning |
|---|---|
| `name` | the application name: 1 to 64 ASCII letters, digits, `-`, `_`, `.`, not starting with `.`. It names the directories, the lock, the configuration file, the environment prefix (`SITE_` for `site`), and the log target. Defaults to the package name. |

Errors: every [`ErrorKind`](#errorkind) a boot step can return; `Setup` for an invalid name.

Cost: under 5 ms to `Ready` with the default members and a local configuration file [target, NFR-3, measured in 0.6].

```rust,ignore
fn main() -> std::process::ExitCode {
    let system = match system_suite::start!() {
        Ok(system) => system,
        Err(error) => return error.report(),
    };
    system.run()
}
```

```rust,ignore
// An explicit name, for a binary whose package name is not the service name.
let system = system_suite::start!("hive")?;
```

`start!` is a macro and not a function because the build info must be read in the application crate (D-008). Everything `start!` does is available through [`Builder`](#builder).

### `System::run`

**Since** 0.3 · SUI-03 · feature `kernel`

```rust,ignore
impl System {
    pub fn run(self) -> std::process::ExitCode;
}
```

Parks the calling thread with zero CPU until a stop is requested (a stop signal, `request_stop`, a critical worker that exhausted its restarts, or a service stop on Windows), then runs the shutdown order and returns the exit code. Equivalent to `wait_for_stop` followed by `shutdown`.

| Outcome | Exit code |
|---|---|
| clean stop, whatever requested it | `0` |
| a subsystem or worker missed the shutdown deadline and was abandoned | `79` (`exit::DEADLINE`) |
| a critical worker exhausted its restart budget | `70` (`exit::SOFTWARE`) |

Cost: under 50 ms from a stop request to return with nothing in flight [target, NFR-4].

```rust,ignore
let system = system_suite::start!()?;
let code = system.run();          // returns after SIGTERM, Ctrl+C, or request_stop
std::process::exit(code_as_i32(code));
```

On Windows run as a service, `run` hands the main thread to the service control manager for the life of the service (the manager requires it); the boot must therefore finish within the manager's 30 s start window, which `start!` reports through `SERVICE_START_PENDING` wait hints (D-014).

### Typed handles

**Since** 0.3 to 0.5 · SUI-04 · per feature

Every enabled member is reachable from `System` (and `Foundation`) through a method that returns its handle directly. Members are resolved once at boot; nothing is looked up by name at runtime, and a handle access is a field read (about 1 ns [est.]).

```rust,ignore
let system = system_suite::start!()?;
system_suite::log::info!(system.log(), "listening", port = 8080_u16);
let port: u16 = system.config().get_or("server.port", 8080)?;
let started = system.instance().started_at();
let requests = system.metrics().counter("requests_total");
let dir = system.paths().data();
```

The full list is under [`Foundation`](#foundation).

### `Error::report`

**Since** 0.3 · SUI-05 · features: any

```rust,ignore
impl Error {
    pub fn report(&self) -> std::process::ExitCode;
}
```

Writes the error to standard error as one block (what failed, at which step, for which member, and what to do) and returns the exit code for its kind. Use it in `main` when `start!` fails: a boot failure before logging opens (steps 1 to 9) has nowhere else to go, and one after it was also logged. It writes with one `write_all` to a locked `stderr` and never panics; if standard error is closed, the write error is ignored and the exit code is still returned.

```rust,ignore
let system = match system_suite::start!() {
    Ok(system) => system,
    Err(error) => return error.report(),
};
```

```text
site: boot step 6 (guard) failed: another copy is running (pid 4182, lock /run/user/1000/site/site.lock).
  do: stop the other copy, or start this one with a different name.
```

## Crate root

### `VERSION`

**Since** 0.1 · SUI-01

```rust
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
```

The version of system-suite itself (not of the application).

```rust
assert!(!system_suite::VERSION.is_empty());
```

### `build_info!` and `build::emit`

**Since** 0.3 · SUI-06 · feature `sys`

```rust,ignore
pub mod build {
    pub use sys_lib::build::{emit, emitter, Emitter};
}
pub use sys_lib::build_info;            // #[macro_export] in sys-lib: &'static sys::BuildInfo
```

`build::emit()` is called from the application's `build.rs`; it records the commit, the dirty flag, the build time (`SOURCE_DATE_EPOCH` wins), the profile, the target, the compiler, and the enabled features, and never fails the build. `build_info!()` expands in the application crate to a `&'static BuildInfo`; [`start!`](#start) calls it for you. Tier 2 code passes it to [`Builder::build_info`](#identity-and-sources).

```rust,ignore
// build.rs
fn main() { system_suite::build::emit(); }
```

```rust,ignore
let build = system_suite::build_info!();
let system = system_suite::System::builder("site").build_info(build).start()?;
assert_eq!(system.build().map(|b| b.version), Some(env!("CARGO_PKG_VERSION")));
```

### `Member`

**Since** 0.3 · SUI-23

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Member {
    Kernel, Sys, Paths, Env, Config, Log, Metrics, Signals,
    Time, Ids, Audit, Registry, Globals, Rand, Secrets,
}

impl Member {
    pub const fn name(self) -> &'static str;      // "kernel", "sys", ... (the feature name)
    pub const fn crate_name(self) -> &'static str; // "sys-lib", "path-lib", ... ("system-suite" for Kernel)
    pub const fn is_compiled(self) -> bool;       // its feature is enabled in this build
}
impl std::fmt::Display for Member;                // the feature name
```

Names a member in errors, health reports, and [`Builder::disable`](#members).

```rust,ignore
use system_suite::Member;
assert_eq!(Member::Log.crate_name(), "log-io");
assert_eq!(Member::Kernel.is_compiled(), cfg!(feature = "kernel"));
```

### `Step`

**Since** 0.3 · SUI-23

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Step {
    BuildInfo, Instance, Paths, Environment, Configuration, Guard, Signals, Secrets,
    Directories, Logging, CrashCapture, Metrics, Audit, Registry, Kernel, Subsystems, Ready,
}

impl Step {
    pub const fn number(self) -> u8;          // 1 to 17, the position in BOOT-ORDER.md
    pub const fn name(self) -> &'static str;  // "guard"
    pub const fn member(self) -> Option<Member>;
}
impl std::fmt::Display for Step;              // "step 6 (guard)"
```

The boot steps, in order. `Step` values compare in boot order.

```rust,ignore
use system_suite::Step;
assert!(Step::Configuration < Step::Guard);
assert_eq!(Step::Guard.number(), 6);
```

### `Snapshot` and `Reader`

**Since** 0.4 · SUI-22 · features: any

```rust
pub struct Snapshot<T> { /* Arc: version counter and the current value */ }
impl<T: Send + Sync + 'static> Snapshot<T> {
    pub fn new(value: T) -> Self;
    pub fn publish(&self, value: T) -> u64;   // returns the new version
    pub fn load(&self) -> std::sync::Arc<T>;
    pub fn version(&self) -> u64;
    pub fn reader(&self) -> Reader<T>;
}
impl<T> Clone for Snapshot<T>;                // cheap: shares the same slot

pub struct Reader<T> { /* the snapshot, the version seen, the cached Arc */ }
impl<T: Send + Sync + 'static> Reader<T> {
    pub fn get(&mut self) -> &T;              // #[inline]
    pub fn version(&self) -> u64;
}
```

A value that is read on every request and replaced whole on a reload: profiles, compiled rules, budgets, and the configuration itself. `publish` swaps in a complete new value in one step; readers already holding the old one keep it until their next `get`, and the old value is freed when the last reader moves on. One `Reader` per worker thread: `get` is one atomic load and a compare when nothing changed, and a short lock plus an `Arc` clone only right after a publish.

| Operation | Cost [est.] |
|---|---|
| `Reader::get`, unchanged | about 1 ns, no allocation |
| `Reader::get`, first call after a publish | 20 to 50 ns |
| `publish` | one allocation for the new `Arc`, one short lock |

`Snapshot` is `Send + Sync`; `Reader` is `Send` and not `Sync` (it belongs to one thread).

```rust
use system_suite::Snapshot;

let limits = Snapshot::new(vec![100_u32, 200]);
let mut reader = limits.reader();
assert_eq!(reader.get()[0], 100);

let version = limits.publish(vec![150, 250]);   // a reload
assert_eq!(reader.get()[0], 150);
assert_eq!(reader.version(), version);
```

### `foundation()`

**Since** 0.5 · SUI-30 · feature `globals`

```rust,ignore
pub fn foundation() -> Option<Foundation>;
```

The running system's `Foundation`, for code that cannot be handed one (a plugin loaded by name, a callback registered by a third-party library). It is published through global-lib when the system reaches `Ready` and removed when shutdown starts; it is `None` before, after, and in a system built with `process_wide(false)`. Prefer passing `Foundation` explicitly; this exists for the cases where that is impossible.

```rust,ignore
if let Some(foundation) = system_suite::foundation() {
    system_suite::log::warn!(foundation.log(), "plugin reload");
}
```

## `System`

**Since** 0.3 · SUI-03, SUI-04, SUI-11, SUI-12, SUI-26

```rust,ignore
#[must_use = "dropping a System shuts it down at once; call run()"]
pub struct System { /* Foundation, the kernel's control block, the started subsystems */ }
```

The running system: owns the lifecycle, the started members, and the application's subsystems. It is `Send` and `Sync` (a framework may keep it in an `Arc` and call `request_stop` from another thread) and not `Clone` (exactly one owner runs the shutdown). Dropping it without calling `run` or `shutdown` runs the shutdown order immediately, as for a stop request.

### Accessors

Every accessor of [`Foundation`](#foundation) is also on `System`, with the same signature, plus:

```rust,ignore
impl System {
    pub fn builder(name: &str) -> Builder;          // same as Builder::new
    pub fn foundation(&self) -> &Foundation;        // to hand to a framework or a thread (Clone)
    pub fn name(&self) -> &str;
}
```

```rust,ignore
let system = system_suite::start!()?;
let foundation = system.foundation().clone();
std::thread::spawn(move || {
    system_suite::log::info!(foundation.log(), "background job started");
});
```

### Lifecycle

```rust,ignore
impl System {
    pub fn phase(&self) -> lifecycle::Phase;
    pub fn run(self) -> ExitCode;                    // Tier 1, above
    pub fn wait_for_stop(&self) -> lifecycle::StopReason;
    pub fn shutdown(self) -> ExitCode;
    pub fn boot_timeline(&self) -> &[(Step, Duration)];   // 0.6: every step that ran, with its duration (D-070)
}
```

`wait_for_stop` parks the calling thread until a stop is requested and returns why; it can be called from several threads. `shutdown` runs the shutdown order now, whether or not a stop was requested, and returns the exit code. A framework that has work to do between the stop request and the shutdown (handing its listening sockets to a supervisor for a restart, for example) calls the two separately.

```rust,ignore
let system = system_suite::start!()?;
let reason = system.wait_for_stop();
system_suite::log::info!(system.log(), "stopping", reason = %reason);
hand_sockets_to_supervisor();
let code = system.shutdown();
```

**Since** 0.3 (SUI-26).

### `request_stop`, `reload`

**Since** 0.4 · SUI-11 · feature `kernel` (`reload`: `config`)

```rust,ignore
impl System {
    pub fn request_stop(&self, reason: &'static str) -> bool;
    pub fn reload(&self) -> Result<u64, Error>;
}
```

`request_stop` asks for a clean stop exactly as a SIGTERM would; it returns `false` if a stop was already requested (the first reason is kept). It never blocks.

`reload` reads the configuration file again, validates it, resolves secret references, and asks every member and subsystem to prepare its change; only if all of them succeed is the new configuration published and every change committed. It returns the new configuration version. On any failure nothing changes and the old configuration stays in force (threat T7).

| Error kind | When |
|---|---|
| `ConfigNotFound`, `ConfigInsecure`, `ConfigInvalid` | as at boot step 5 |
| `Secret`, `PlainSecret` | as at boot step 8 |
| `ReloadRefused` | a member or subsystem refused the change; the error names it |
| `Stopping` | a stop was already requested |

Settings that cannot change while running (paths, the guard, which members are enabled, sink files) do not fail the reload; they are logged as "takes effect at restart".

```rust,ignore
match system.reload() {
    Ok(version) => system_suite::log::info!(system.log(), "configuration reloaded", version),
    Err(error) => system_suite::log::warn!(system.log(), "reload refused", error = %error),
}
```

```rust,ignore
// From a request handler that decided the process must go away.
if !system.request_stop("admin requested") {
    // already stopping
}
```

Cost of a reload: the file read and parse, plus each member's and subsystem's preparation [measured in 0.6]; nothing on the request path changes except the next `Reader::get`.

### `health`, `status`

**Since** 0.4 · SUI-12 · feature `kernel`

```rust,ignore
impl System {
    pub fn health(&self) -> kernel::HealthReport;
    pub fn status(&self) -> kernel::Status;
}
```

`health` collects every member, subsystem, worker, and the timer thread into a [`HealthReport`](#health-health-status-healthreport) (it allocates; for health endpoints and the dump signal). `status` returns only the aggregate and does not allocate (for frequent checks).

```rust,ignore
let report = system.health();
let mut body = String::new();
report.write_json(&mut body)?;
let code = if report.ready() { 200 } else { 503 };
```

## `Builder`

**Since** 0.3 to 0.5 · SUI-10

```rust,ignore
#[must_use = "a Builder does nothing until start() or boot()"]
pub struct Builder { /* every setting below */ }
impl Builder {
    pub fn new(name: &str) -> Builder;
}
```

Every setting has a secure default; a setting given here wins over the same setting in the configuration file. `Builder` is `Send`. Invalid settings are reported by `start` (kind `Setup`), never by the setter, so a chain of setters never needs `?`.

### Identity and sources

```rust,ignore
impl Builder {
    pub fn build_info(self, build: &'static sys::BuildInfo) -> Self;        // 0.3, feature sys
    pub fn scope(self, scope: path::Scope) -> Self;                         // 0.3, feature paths; default User
    pub fn portable(self, root: impl Into<PathBuf>) -> Self;               // 0.3, feature paths
    pub fn env_file(self, path: impl Into<PathBuf>) -> Self;               // 0.3, feature env
    pub fn config_file(self, path: impl Into<PathBuf>) -> Self;            // 0.4, feature config
    pub fn config_str(self, text: impl Into<String>) -> Self;              // 0.4, feature config
    pub fn clock(self, clock: impl clock::Clock + 'static) -> Self;        // 0.3, feature time
}
```

| Setting | Default | Notes |
|---|---|---|
| `build_info` | none: `Foundation::build()` returns `None` | `start!` sets it; a missing value is logged once as a warning (D-038) |
| `scope` | `User` (overridden by `<NAME>_PATH_SCOPE`: `user`, `global`, `auto`) | `Global` for system services; directory overrides come from `<NAME>_<KIND>_DIR`; `Scope::Portable` needs `portable(root)` and is a `Setup` error without it |
| `portable` | none | all directories under `root` (`root/data`, `root/config`, ...) |
| `env_file` | `<config dir>/.env`, if it exists | an explicit file that does not exist is an error (`Environment`) |
| `config_file` | searched (see [Finding the file](#finding-the-file)) | an explicit file that does not exist is an error (`ConfigNotFound`) |
| `config_str` | none | the configuration text itself; for tests and embedded defaults; disables the file search and reload reads it again |
| `clock` | the OS clock | what `Foundation::clock` returns; kernel deadlines and timers always use the OS monotonic clock |

```rust,ignore
let system = system_suite::System::builder("hive")
    .build_info(system_suite::build_info!())
    .scope(system_suite::path::Scope::Global)
    .config_file("/etc/hive/hive.conf")
    .start()?;
```

```rust,ignore
// A test: configuration in memory, a manual clock, nothing process-wide.
let clock = std::sync::Arc::new(system_suite::clock::ManualClock::new());
let system = system_suite::System::builder("test-svc")
    .config_str("[log]\nlevel = \"debug\"\nfile = false\n")
    .clock(clock.clone())
    .process_wide(false)
    .start()?;
```

### Members

```rust,ignore
impl Builder {
    pub fn disable(self, member: Member) -> Self;          // 0.3
    pub fn process_wide(self, on: bool) -> Self;           // 0.3; default true
    pub fn crash_capture(self, on: bool) -> Self;          // 0.3, feature kernel; default true (D-016)
    pub fn single_instance(self, on: bool) -> Self;        // 0.3, feature sys; default true
}
```

`disable` turns a compiled-in member off for this run. Members that others need at run time cannot be disabled; asking is a `Setup` error.

| Can be disabled at run time | Cannot |
|---|---|
| `Env`, `Config` (defaults only), `Log` (no sinks; crash reports keep no lines), `Metrics`, `Signals`, `Audit`, `Registry`, `Globals`, `Secrets` | `Kernel`, `Sys`, `Paths`, `Time`, `Ids`, `Rand` (they hold no run-time state to turn off) |

`process_wide(false)`: see [Process-wide state](#process-wide-state). `crash_capture(false)` leaves the panic hook alone (D-016 explains the default). `single_instance(false)` takes no lock (the same as `[guard] lock = false`); use it when a supervisor holds the lock or when several copies are meant to run.

```rust,ignore
// A command-line tool: several copies may run, no metrics, no audit.
let system = system_suite::System::builder("hivectl")
    .single_instance(false)
    .disable(system_suite::Member::Metrics)
    .start()?;
```

### Kernel settings

```rust,ignore
impl Builder {
    pub fn shutdown_deadline(self, deadline: Duration) -> Self;   // 0.3; default 25 s
    pub fn member_reserve(self, reserve: Duration) -> Self;       // 0.4; default 2 s
}
```

The [deadlines](./BOOT-ORDER.md#deadlines): the whole shutdown, and the part of it kept back for the members after the subsystems. Zero, a reserve larger than the deadline, or a deadline over 1 hour is a `Setup` error.

```rust,ignore
let system = system_suite::System::builder("hive")
    .shutdown_deadline(Duration::from_secs(60))     // a database flushes for longer
    .member_reserve(Duration::from_secs(5))
    .start()?;
```

### Subsystems and callbacks

```rust,ignore
impl Builder {
    pub fn register(self, subsystem: impl lifecycle::Subsystem) -> Self;                         // 0.4
    pub fn on_ready<F>(self, callback: F) -> Self                                              // 0.4
        where F: FnOnce(&Foundation) -> Result<(), lifecycle::Failure> + Send + 'static;
    pub fn on_vault<F>(self, callback: F) -> Self                                              // 0.5, feature secrets
        where F: FnOnce(&vault::KeyVault) -> Result<(), lifecycle::Failure> + Send + 'static;
}
```

`on_vault` runs at boot step 8, after the vault is built and before `vault:` references are resolved: the place to register keys the application obtains itself (from a hardware module, a key-management service, an operator prompt).

`register` adds a subsystem; it is started at boot step 16 in ascending [`Position`](#position), ties in registration order, and stopped in exactly the reverse of the order it actually started. A duplicate or invalid name is a `Setup` error at `start`.

`on_ready` callbacks run at step 17, in registration order, after every subsystem started and before `READY=1`. A failing callback unwinds the boot like a failing step.

```rust,ignore
let system = system_suite::System::builder("site")
    .register(Database::new())                    // Position::DEFAULT
    .register(HttpListener::new())                // Position::LAST: the door opens last
    .on_ready(|f| { system_suite::log::info!(f.log(), "serving"); Ok(()) })
    .start()?;
```

### `start`, `boot`

```rust,ignore
impl Builder {
    pub fn start(self) -> Result<System, Error>;      // 0.3
    pub fn boot(self) -> Result<Boot, Error>;         // 0.4, feature kernel
}
```

`start` runs every boot step. `boot` runs steps 1 to 15 (the foundation and the kernel) and returns a [`Boot`](#boot-staged-start-for-frameworks) for frameworks that build their subsystems from what the foundation provides.

## `Boot`: staged start for frameworks

**Since** 0.4 · SUI-25 · feature `kernel`

```rust,ignore
#[must_use = "dropping a Boot stops everything it started"]
pub struct Boot { /* the System under construction */ }
impl Boot {
    pub fn foundation(&self) -> &Foundation;
    pub fn kernel(&self) -> &kernel::Kernel;
    pub fn start(&mut self, subsystem: impl lifecycle::Subsystem) -> Result<(), Error>;
    pub fn ready(self) -> Result<System, Error>;
}
```

A framework's own boot phases happen between the foundation and readiness, and each phase builds things that need the previous phase's results (for example: load the application's modules, then connect the providers they declared, then open the listener with the routes they registered). `Boot` lets a framework do that with full typed access:

- `start` starts a subsystem immediately (its `start` runs on the calling thread) and appends it to the stop order. On error, the subsystem is not added and the `Boot` is still usable; the framework decides whether to continue or drop the `Boot`.
- `ready` starts the subsystems given to `Builder::register` (in `Position` order), runs `on_ready`, sends readiness, and returns the `System`.
- Dropping a `Boot` stops everything it started, in reverse, as a boot failure does. A stop signal received while a `Boot` exists is recorded and makes the next `start` or `ready` fail with `Interrupted`.

```rust,ignore
let mut boot = system_suite::System::builder("gateway")
    .build_info(system_suite::build_info!())
    .boot()?;
let modules = Modules::load(boot.foundation())?;           // the framework's first phase
for provider in modules.providers()? {                     // then what the modules declared
    boot.start(provider)?;
}
boot.start(Listener::new(modules.routes())?)?;             // the door, last
let system = boot.ready()?;
system.run()
```

## `Foundation`

**Since** 0.3 · SUI-20, SUI-04

```rust,ignore
#[derive(Clone)]
pub struct Foundation { /* Arc: one typed handle per enabled member */ }
```

The typed handles of every enabled member, built once at boot. It is what a framework receives, what every subsystem gets through its `Link`, and what `System` delegates to. `Clone` is one reference-count increment; it is `Send + Sync`.

| Method | Returns | Feature | Since |
|---|---|---|---|
| `name(&self)` | `&str` | any | 0.3 |
| `build(&self)` | `Option<&'static sys::BuildInfo>` (`None` when the builder was not given it, D-038) | `sys` | 0.3 |
| `instance(&self)` | `&'static sys::Instance` | `sys` | 0.3 |
| `guard_report(&self)` | `&sys::process::GuardReport` | `sys` | 0.3 |
| `paths(&self)` | `&path::App` | `paths` | 0.3 |
| `env(&self)` | `&env::Env` | `env` | 0.3 |
| `config(&self)` | `config::Config` (the current snapshot; cheap clone) | `config` | 0.4 |
| `config_snapshot(&self)` | `&SnapshotView<config::Config>` (for a per-thread `Reader`; read-only since 0.7, D-074) | `config` | 0.4 |
| `log(&self)` | `&log::Logger` | `log` | 0.3 |
| `metrics(&self)` | `&metrics::MetricsCore` | `metrics` | 0.4 |
| `clock(&self)` | `&dyn clock::Clock` | `time` | 0.3 |
| `ids(&self)` | `&id::snowflake::Snowflake` (worker id from `[ids] worker`) | `ids` | 0.3 |
| `audit(&self)` | `&AuditLog` | `audit` | 0.5 |
| `events(&self)` | `&Arc<registry::SyncRegistry<kernel::Event>>` | `registry` | 0.5 |
| `vault(&self)` | `&vault::KeyVault` | `secrets` | 0.5 |
| `kernel(&self)` | `&kernel::Kernel` | `kernel` | 0.3 |
| `runtime(&self)` | `&runtime::Runtime` | `kernel` | 0.4 |

A member disabled at run time with [`Builder::disable`](#members) still has its accessor (the build decides what exists): `log()` returns a logger with no sinks, `metrics()` a registry nothing exports, `config()` the defaults, `env()` an empty environment. The accessors of `Audit`, `Registry`, and `Secrets` return handles whose operations fail with kind `Disabled`.

```rust,ignore
fn connect(f: &system_suite::Foundation) -> Result<Db, Error> {
    let url: String = f.config().require("database.url")?;
    let timeout = f.config().get_or("database.timeout", Duration::from_secs(5))?;
    system_suite::log::info!(f.log(), "connecting", instance = %f.instance().id());
    Db::open(&url, timeout)
}
```

```rust,ignore
// Ids: Snowflake for entity ids (unique per worker id), ULID for request ids.
let order_id = f.ids().try_next_id()?;                 // never next_id(): it panics on clock skew
let request_id = system_suite::id::ulid::Ulid::new();
```

Ids from id-forge are unique, not secret: they come from a fast non-cryptographic generator. Tokens and nonces come from `rand::tier3` (feature `rand`).

## `Error`

**Since** 0.3 · SUI-23

```rust,ignore
#[non_exhaustive]
pub struct Error { /* kind, step, member, subsystem, detail, action, source */ }

pub type Result<T, E = Error> = core::result::Result<T, E>;

impl Error {
    pub fn kind(&self) -> ErrorKind;
    pub fn step(&self) -> Option<Step>;          // None for errors after boot
    pub fn member(&self) -> Option<Member>;
    pub fn subsystem(&self) -> Option<&str>;     // the subsystem's name, for kind Subsystem
    pub fn action(&self) -> &str;                // what the operator or caller should do
    pub fn exit_code(&self) -> ExitCode;
    pub fn report(&self) -> ExitCode;            // Tier 1, above
}
impl std::fmt::Display for Error;
impl std::fmt::Debug for Error;
impl std::error::Error for Error;                // source(): the member's own error, when there is one
```

One error type for the whole crate. Every error names what failed, where, and what to do (NFR-7); the member's own error (`path_lib::Error`, `sys_lib::Error`, ...) is kept as `source()`. It is `Send + Sync + 'static` and not `Clone` (it can hold an `io::Error`). Neither `Display` nor `Debug` ever contains a secret value or a configuration value; they name keys and files only.

```rust,ignore
match system_suite::start!() {
    Ok(system) => system.run(),
    Err(error) if error.kind() == system_suite::ErrorKind::AlreadyRunning => {
        std::process::ExitCode::SUCCESS            // a cron-started copy: the running one does the work
    }
    Err(error) => error.report(),
}
```

### `ErrorKind`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind { /* the variants below */ }
```

| Kind | Meaning | What to do | Exit code | Since |
|---|---|---|---|---|
| `Setup` | an invalid builder setting: bad name, duplicate subsystem, a member that cannot be disabled, a zero deadline | fix the code; the message names the setting | 64 | 0.3 |
| `Entropy` | the OS random source is unavailable (a sandbox or seccomp filter blocks it) | allow `getrandom`/`BCryptGenRandom` for the process | 69 | 0.3 |
| `Paths` | an application directory cannot be resolved or created, or is a symlink | the message names the directory and the reason; set `<NAME>_<KIND>_DIR` or fix the permissions | 73 | 0.3 |
| `Environment` | the `.env` file is malformed, too large, or unreadable | fix the line named in the message | 78 | 0.3 |
| `ConfigNotFound` | an explicit configuration file (builder or `<NAME>_CONFIG`) does not exist | create it, or fix the path | 78 | 0.4 |
| `ConfigInsecure` | the configuration file (or the `.env` file) is group- or world-writable, or owned by another user (Unix; D-020) | `chmod 640` (or `600`) and `chown` to the service user; there is no switch to accept it (D-048) | 77 | 0.4 |
| `ConfigInvalid` | the file does not parse, a member section has an invalid value, or a value read later has the wrong type | the message names the file, line, key, expected type, and accepted values | 78 | 0.4 |
| `AlreadyRunning` | another copy holds the single-instance lock | stop the other copy, or use another name | 75 | 0.3 |
| `LockDirInsecure` | the lock directory cannot be created, is writable by others, a symlink, or on a network filesystem | the message gives the fix (`chmod 700`, a local directory, `[guard] allow_network_lock`) | 77 | 0.3 |
| `Limits` | the hard open-file limit is below `[guard] open_files_minimum` | raise `LimitNOFILE=` in the systemd unit, `ulimit -Hn`, or the launchd limit | 69 | 0.3 |
| `Signals` | the signal intake could not be installed | the message names the signal; another library may own it, or run with `disable(Member::Signals)` | 70 | 0.3 |
| `Secret` | a secret reference cannot be resolved: the variable is unset, the file is missing or too open (Unix: looser than `0600`), the vault has no such name; in 0.4, any reference (resolution needs the `secrets` feature, 0.5) | the message names the key, never the value | 78 | 0.4 |
| `PlainSecret` | a key whose name marks a secret (`password`, `token`, `secret`, ...) holds a plain value instead of a reference | replace it with `env:NAME`, `file:/path`, or `vault:name`, or set `[secrets] allow_plain = true` | 78 | 0.5 |
| `Logging` | a log sink cannot be opened | the message names the file and the I/O error | 73 | 0.3 |
| `Metrics` | the metrics registry rejected its configuration | the message names the key | 70 | 0.4 |
| `Audit` | the audit file cannot be opened, read, or synced | the message names the file and the I/O error | 74 | 0.5 |
| `AuditTampered` | the existing audit chain fails verification: a record was changed, removed, or reordered | do not delete the file; keep it as evidence, move it aside, and investigate; the message names the first bad record | 65 | 0.5 |
| `Kernel` | a kernel thread could not be started (the OS refused a thread) | check the process and thread limits | 70 | 0.3 |
| `Subsystem` | a subsystem's `start`, `stop`, or `on_ready` failed or panicked | the message carries the subsystem's name and its `Failure` | 70 | 0.4 |
| `Interrupted` | a stop signal arrived during boot; the boot unwound cleanly | nothing: the operator asked for it | 0 | 0.3 |
| `Stopping` | the kernel no longer accepts the request (a worker, timer, or reload after a stop was requested) | stop scheduling work; the process is going away | 70 | 0.4 |
| `ReloadRefused` | a member or subsystem refused the new configuration; nothing changed | the message names who refused and why | 78 | 0.4 |
| `UrgentRefused` | an urgent-line change that is not allowed (`Draining` or `Stopping` set directly, or any change after a stop) | use `request_stop` | 70 | 0.4 |
| `Disabled` | an operation on a member disabled at run time | enable it, or do not call it | 70 | 0.5 |
| `Io` | an I/O error not covered above | the message names the operation and the file | 74 | 0.3 |

### Exit codes

**Since** 0.3 · SUI-16

```rust
pub mod exit {
    pub const OK: u8 = 0;
    pub const USAGE: u8 = 64;        // invalid setup
    pub const DATAERR: u8 = 65;      // audit chain tampered
    pub const UNAVAILABLE: u8 = 69;  // a requirement of the environment is missing
    pub const SOFTWARE: u8 = 70;     // internal failure, subsystem failure, critical worker failure
    pub const CANTCREAT: u8 = 73;    // a directory or file cannot be created
    pub const IOERR: u8 = 74;
    pub const TEMPFAIL: u8 = 75;     // already running: try again later
    pub const NOPERM: u8 = 77;       // insecure permissions refused
    pub const CONFIG: u8 = 78;       // configuration missing or invalid
    pub const DEADLINE: u8 = 79;     // shutdown deadline missed; something was abandoned
}
```

The codes follow BSD `sysexits.h` (D-017), so supervisors and scripts can tell a configuration error from a crash; `79` is the one code of the suite's own. A panic ends the process as the application's profile decides (with `panic = "abort"`, SIGABRT, shown as 134 by shells on Unix).

## `kernel`

**Since** 0.3 to 0.5 · feature `kernel`

The suite's controller: it receives the signals, decides when the process stops, raises the urgent line, runs the boot and shutdown orders, aggregates health, reports readiness, captures crashes, and records what happened. The parts it drives are their own modules: [`lifecycle`](#lifecycle) (the phases and the contract every subsystem follows) and [`runtime`](#runtime) (supervised workers and timers) (D-037). Everything here is `std` only, with no async runtime (D-005).

### `Kernel`

**Since** 0.3 · SUI-27

```rust,ignore
#[derive(Clone)]
pub struct Kernel { /* Arc: the kernel's control block */ }
impl Kernel {
    pub fn phase(&self) -> lifecycle::Phase;                                                   // 0.3
    pub fn request_stop(&self, reason: &'static str) -> bool;                                 // 0.3
    pub fn stop_reason(&self) -> Option<lifecycle::StopReason>;                               // 0.3
    pub fn wait_for_stop(&self) -> lifecycle::StopReason;                                     // 0.3
    pub fn wait_for_stop_timeout(&self, timeout: Duration) -> Option<lifecycle::StopReason>;  // 0.3
    pub fn urgent(&self) -> &UrgentLine;                                           // 0.4
    pub fn health(&self) -> HealthReport;                                          // 0.4
    pub fn status(&self) -> Status;                                                // 0.4
    pub fn runtime(&self) -> &runtime::Runtime;                                    // 0.4
}
```

The kernel's control handle, the one thing a framework's core receives instead of raw signals (signals reach only the kernel, D-006). Cheap to clone, `Send + Sync`; every method is safe from any thread. `request_stop` and `stop_reason` are an atomic operation each; `wait_for_stop` parks on a condition variable.

```rust,ignore
let kernel = system.kernel().clone();
std::thread::spawn(move || {
    if disk_almost_full() {
        kernel.request_stop("disk almost full");
    }
});
```

### The urgent line: `UrgentLine`, `Level`, `Watch`, `Notice`

**Since** 0.4 · SUI-21

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum Level { Normal = 0, Shed = 1, Lockdown = 2, Draining = 3, Stopping = 4 }

#[derive(Clone)]
pub struct UrgentLine { /* Arc: one cache-line-aligned atomic level, epoch, and reason */ }
impl UrgentLine {
    pub fn detached() -> UrgentLine;                                            // a line no kernel drives: tests, libraries
    pub fn level(&self) -> Level;                                               // #[inline]
    pub fn epoch(&self) -> u64;                                                 // #[inline]
    pub fn notice(&self) -> Notice;
    pub fn set(&self, level: Level, reason: &'static str) -> Result<u64, Error>;
    pub fn watch(&self) -> Watch;
}

pub struct Watch { /* the line and the epoch last seen */ }
impl Watch {
    pub fn changed(&mut self) -> Option<Level>;                                 // #[inline]
    pub fn level(&self) -> Level;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Notice { pub level: Level, pub epoch: u64, pub reason: &'static str }
```

How the kernel (and, when present, the security package) tells the whole process "shed load", "lock down", "drain", or "stop" now, without an event loop in the way (PLAN section 6.3). Tier T0 is the atomic level and epoch that every worker reads at its checkpoints; tier T1 is each subsystem's [`Mailbox`](#link-and-mailbox), poked and woken directly; tier T2 is the [`Event`](#event) record in the registry, logs, and metrics.

| Level | Raised by | Meaning for a worker at its checkpoint |
|---|---|---|
| `Normal` | `set`, or the condition cleared | normal behaviour |
| `Shed` | `set` (a memory watermark, a supervisor) | refuse low-priority work |
| `Lockdown` | `set` (the security package) | strict policies only |
| `Draining` | the kernel, at a stop request | accept no new work; finish what is in flight |
| `Stopping` | the kernel, after the subsystems stopped or at the deadline | stop now |

`set` changes the level to `Normal`, `Shed`, or `Lockdown` and returns the new epoch. `Draining` and `Stopping` belong to the kernel (`request_stop`), and once a stop is requested the line only moves forward; both cases return `UrgentRefused`. Every change is logged, counted, and published as an `Event`.

| Operation | Cost [est.] |
|---|---|
| `level`, `epoch`, `Watch::changed` when nothing changed | about 1 ns, no allocation (NFR-5) |
| a change becoming visible on another core | 40 to 100 ns |
| `set` | one store per tier-0 word plus one mailbox poke per subsystem; waking a parked thread is microseconds |

```rust,ignore
// In a worker loop: the check costs about a nanosecond when nothing changed.
let mut watch = system.kernel().urgent().watch();
loop {
    if let Some(level) = watch.changed() {
        match level {
            Level::Draining | Level::Stopping => break,
            Level::Lockdown => enter_lockdown(),
            Level::Shed => shed_low_priority(),
            _ => back_to_normal(),
        }
    }
    serve_one()?;
}
```

```rust,ignore
// A memory watermark lowers the load, then clears it.
let line = system.kernel().urgent();
line.set(Level::Shed, "memory above 90%")?;
// ...
line.set(Level::Normal, "memory below 80%")?;
```

### Health: `Health`, `Status`, `HealthReport`

**Since** 0.4 · SUI-12

```rust,ignore
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Status { Healthy, Degraded, Unhealthy }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Health { /* status, optional reason */ }
impl Health {
    pub const HEALTHY: Health;
    pub fn degraded(reason: impl Into<Cow<'static, str>>) -> Health;
    pub fn unhealthy(reason: impl Into<Cow<'static, str>>) -> Health;
    pub fn status(&self) -> Status;
    pub fn reason(&self) -> Option<&str>;
}

#[derive(Clone, Debug)]
pub struct HealthReport { /* status, phase, urgent level, components */ }
impl HealthReport {
    pub fn status(&self) -> Status;
    pub fn phase(&self) -> Phase;
    pub fn urgent(&self) -> Level;
    pub fn ready(&self) -> bool;        // phase Ready and status not Unhealthy: a readiness probe
    pub fn live(&self) -> bool;         // the kernel is running: a liveness probe
    pub fn components(&self) -> &[ComponentHealth];
    pub fn write_json(&self, out: &mut impl std::fmt::Write) -> std::fmt::Result;
    pub fn write_kv(&self, out: &mut impl std::fmt::Write) -> std::fmt::Result;
}
impl std::fmt::Display for HealthReport;    // one line per component

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ComponentHealth {
    pub kind: Component,
    pub name: Cow<'static, str>,
    pub health: Health,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Component { Member(Member), Subsystem, Worker, Timers }
```

Health aggregates every member, subsystem, worker, and the timer thread (FR-11). The aggregate is the worst component status, with one exception: a worker that is not `critical` can make the system at most `Degraded`. The report never contains a configuration value or a secret; reasons are the components' own words.

When `NOTIFY_SOCKET` is set (systemd, Linux only) and `[kernel] notify` is `"auto"`, the kernel sends `READY=1` and `STATUS=ready` at boot step 17, `STOPPING=1` and `STATUS=stopping` at shutdown step 1, and, when systemd asked for keep-alives (`WATCHDOG_USEC`, for this process), `WATCHDOG=1` at half that interval while the status is not `Unhealthy` (so systemd restarts a wedged process) and `STATUS=<status>` whenever the aggregate status changes. The keep-alive is a timer named `systemd-watchdog` on the runtime's timer thread. It uses a std Unix datagram socket, no FFI. A heartbeat to the planned out-of-process watchdog (tower) is added when tower exists; the API above does not change.

| Operation | Cost [est.] |
|---|---|
| `status()` | one `health()` call per subsystem (a subsystem whose `stop`, `reload`, or `start` holds it reports its last health instead of blocking), an atomic read per worker; no allocation of its own [counting-allocator test in 0.6] |
| `health()` | the same plus one `Vec` and the reasons |

```rust,ignore
// A health endpoint.
let report = system.health();
let mut body = String::with_capacity(512);
report.write_json(&mut body)?;
respond(if report.ready() { 200 } else { 503 }, "application/json", body);
```

```json
{"status":"degraded","phase":"ready","urgent":"normal","components":[
 {"kind":"member","name":"log","status":"healthy"},
 {"kind":"subsystem","name":"db","status":"degraded","reason":"replica lag 12 s"},
 {"kind":"worker","name":"log-shipper","status":"healthy"}]}
```

### `Event`

**Since** 0.5 · SUI-28 · feature `registry`

```rust,ignore
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Event {
    Phase { from: Phase, to: Phase },
    Urgent { notice: Notice },
    Reloaded { version: u64 },
    ReloadRefused { by: String, reason: String },
    WorkerRestarted { name: String, restarts: u32 },
    WorkerFailed { name: String },
    SubsystemStopped { name: String, took: Duration, met_deadline: bool },
}
```

The kernel's record of what happened (urgent line tier T2), delivered synchronously to callbacks registered on `Foundation::events()` (registry-io's `SyncRegistry`). The registry declares and records; it never delivers urgent changes, which go through the urgent line first (D-006). A panicking callback is isolated by registry-io and does not stop the kernel.

```rust,ignore
let _guard = system.events().register_guard(|event: &Event| {
    if let Event::WorkerFailed { name } = event {
        page_on_call(name);
    }
});
```

## `lifecycle`

**Since** 0.3 to 0.4 · feature `kernel`

The phases of the system and the contract a subsystem follows to be started, stopped, checked, and reloaded in order. The kernel drives it; the application implements `Subsystem`.

### `Phase` and `StopReason`

**Since** 0.3 · SUI-27

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Phase { Booting, Ready, Draining, Stopping, Stopped }

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StopReason {
    Signal { name: &'static str },      // "SIGTERM", "SIGINT", "CTRL_C", "CTRL_CLOSE", "SERVICE_STOP", ...
    Requested { reason: &'static str }, // request_stop
    WorkerFailed { name: String },      // a critical worker exhausted its restart budget
    Supervisor { reason: String },      // the out-of-process watchdog (tower), when present
}
impl std::fmt::Display for StopReason;
impl StopReason { pub fn exit_code(&self) -> u8; }   // 0, except WorkerFailed: 70
```

```text
 Booting ─▶ Ready ─▶ Draining ─▶ Stopping ─▶ Stopped
    │                    ▲
    └── boot failure ────┘ (reverse shutdown of what started, then Stopped)
```

A reload never changes the phase. `Phase` values compare in lifecycle order (`phase >= Phase::Draining` means "going down").

### `Deadline`

**Since** 0.3

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Deadline { /* std::time::Instant */ }
impl Deadline {
    pub fn after(duration: Duration) -> Deadline;
    pub fn instant(&self) -> std::time::Instant;
    pub fn remaining(&self) -> Duration;        // zero once passed
    pub fn is_past(&self) -> bool;
}
```

The point in time by which a subsystem's `stop` (or a worker during shutdown) must return. Always the OS monotonic clock, never the `Builder::clock`, so a test clock cannot stall a shutdown.

```rust
use system_suite::lifecycle::Deadline;
use std::time::Duration;

let deadline = Deadline::after(Duration::from_millis(50));
assert!(!deadline.is_past());
assert!(deadline.remaining() <= Duration::from_millis(50));
```

### `Subsystem`

**Since** 0.4 · SUI-13

```rust,ignore
pub trait Subsystem: Send + 'static {
    fn name(&self) -> &str;
    fn position(&self) -> Position { Position::DEFAULT }
    fn start(&mut self, link: &Link) -> Result<(), Failure>;
    fn stop(&mut self, deadline: Deadline) -> Result<(), Failure>;
    fn health(&self) -> Health { Health::HEALTHY }
    #[cfg(feature = "config")]
    fn reload(&mut self, config: &config::Config) -> Result<Commit, Failure> { Ok(Commit::none()) }
}
```

A part of the application that the kernel starts after the members and stops before them: a database pool, a cache, a job runner, a listener. Registration is the subscription: a registered subsystem receives every stop and every urgent change (through its `Link`), and it cannot forget to listen.

| Method | Called | Contract |
|---|---|---|
| `name` | once, at registration | 1 to 64 ASCII letters, digits, `-`, `_`, `.`; unique in the system |
| `position` | once, at registration | the start order among registered subsystems (below) |
| `start` | boot step 16, on the booting thread | open what it needs; keep the `Link` (clone it) if it wants mailbox notices; return quickly or start a worker for long work. An error or a panic unwinds the boot. |
| `stop` | shutdown step 2, on a kernel stop thread | finish in-flight work and release resources by `deadline`; a `stop` that has not returned at the deadline is abandoned (exit code 79) |
| `health` | on `health()`, `status()`, and the dump signal | cheap and non-blocking; called with the subsystem's lock taken with `try_lock`, so while `start`, `stop`, or `reload` runs, the last reported health is used |
| `reload` | on a reload, before anything is applied | validate the new configuration and build what changes; return a `Commit` that applies it, or a `Failure` that refuses the whole reload |

The trait needs `Send` (stop runs on a kernel thread) and not `Sync` (the kernel never calls it from two threads at once).

```rust,ignore
use system_suite::kernel::Health;
use system_suite::lifecycle::{Deadline, Failure, Link, Position, Subsystem};

struct HttpListener { listener: Option<std::net::TcpListener>, port: u16 }

impl Subsystem for HttpListener {
    fn name(&self) -> &str { "http" }
    fn position(&self) -> Position { Position::LAST }        // the door opens last, closes first

    fn start(&mut self, link: &Link) -> Result<(), Failure> {
        self.port = link.foundation().config().get_or("http.port", 8080)?;
        self.listener = Some(std::net::TcpListener::bind(("0.0.0.0", self.port))?);
        Ok(())
    }

    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> {
        self.listener = None;                                  // stop accepting
        Ok(())
    }

    fn health(&self) -> Health {
        if self.listener.is_some() { Health::HEALTHY } else { Health::unhealthy("not listening") }
    }
}
```

### `Position`

**Since** 0.4 · SUI-13

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position(u16);
impl Position {
    pub const FIRST: Position = Position(0);
    pub const DEFAULT: Position = Position(32_768);
    pub const LAST: Position = Position(u16::MAX);
    pub const fn new(value: u16) -> Position;
    pub const fn get(self) -> u16;
}
```

The declared start order (FR-8): lower starts first, equal positions start in registration order, and the stop order is always the exact reverse of the order in which subsystems actually started. Providers that others depend on go early, the door (anything that accepts outside work) at `LAST`.

```rust
use system_suite::lifecycle::Position;
assert!(Position::FIRST < Position::DEFAULT && Position::DEFAULT < Position::LAST);
let providers = Position::new(1_000);
assert_eq!(providers.get(), 1_000);
```

### `Link` and `Mailbox`

**Since** 0.4 · SUI-13

```rust,ignore
#[derive(Clone)]
pub struct Link { /* the subsystem's name, Foundation, Kernel, mailbox */ }
impl Link {
    pub fn name(&self) -> &str;
    pub fn foundation(&self) -> &Foundation;
    pub fn kernel(&self) -> &Kernel;
    pub fn urgent(&self) -> &UrgentLine;
    pub fn mailbox(&self) -> &Mailbox;
    pub fn report_error(&self, error: &(dyn std::error::Error + 'static));
}

#[derive(Clone)]
pub struct Mailbox { /* the latest notice and a condition variable */ }
impl Mailbox {
    pub fn wait(&self) -> Notice;
    pub fn wait_timeout(&self, timeout: Duration) -> Option<Notice>;
    pub fn try_take(&self) -> Option<Notice>;
}
```

What a subsystem receives at `start`: everything it is allowed to reach, by reference (the core contract's "link"). The `Mailbox` is the urgent line's tier T1: on every urgent change the kernel stores the latest `Notice` in each subsystem's mailbox and wakes whoever waits on it, so a subsystem whose thread is parked (an idle listener, a flusher) acts at once instead of at its next checkpoint. Notices coalesce: a waiter gets the latest one.

`report_error` records an error the subsystem handled but the operator should see: it is logged at error level with the subsystem's name and counted (`system_suite_subsystem_errors_total{subsystem}`). When the planned error suite is published, it is also delivered to it (PLAN section 12.3); the signature does not change.

```rust,ignore
fn start(&mut self, link: &Link) -> Result<(), Failure> {
    let mailbox = link.mailbox().clone();
    let link = link.clone();
    self.thread = Some(std::thread::spawn(move || loop {
        match mailbox.wait_timeout(Duration::from_secs(1)) {
            Some(notice) if notice.level >= Level::Draining => break,
            Some(_) | None => {
                if let Err(error) = flush_cache() { link.report_error(&error); }
            }
        }
    }));
    Ok(())
}
```

### `Failure`

**Since** 0.4 · SUI-13

```rust,ignore
pub struct Failure { /* message, optional source */ }
impl Failure {
    pub fn new(message: impl Into<Cow<'static, str>>) -> Failure;
    pub fn message(&self) -> &str;
}
impl<E: std::error::Error + Send + Sync + 'static> From<E> for Failure;
impl std::fmt::Display for Failure;
impl std::fmt::Debug for Failure;
```

The error a subsystem, worker, or callback returns. Any error type converts into it with `?`, so application code keeps its own error types; the kernel wraps it into an `Error` of kind `Subsystem` that names the subsystem. `Failure` does not implement `std::error::Error` itself (that is what makes the blanket `From` possible); it is reached as the `source()` of the `Error`.

```rust,ignore
fn start(&mut self, link: &Link) -> Result<(), Failure> {
    let path = link.foundation().paths().data_at("cache")?;    // path_lib::Error converts
    if !path.exists() {
        return Err(Failure::new("cache directory missing; run `hivectl init` first"));
    }
    Ok(())
}
```

### `Commit`

**Since** 0.4 · SUI-13 · feature `config`

```rust,ignore
#[must_use = "a Commit applies nothing unless it is returned to the kernel"]
pub struct Commit { /* Option<Box<dyn FnOnce() + Send>> */ }
impl Commit {
    pub fn new(apply: impl FnOnce() + Send + 'static) -> Commit;
    pub fn none() -> Commit;
}
```

The second half of a two-phase reload (D-019). In `reload`, a subsystem validates the new configuration and builds everything that can fail, then returns a `Commit` whose closure only swaps the prepared value in (a `Snapshot::publish`, an atomic store). The kernel runs the commits only when every member and subsystem prepared successfully, so a reload either changes everything or nothing (invariant: a reload swaps the whole configuration or changes nothing). The closure cannot fail and should not block.

```rust,ignore
fn reload(&mut self, config: &Config) -> Result<Commit, Failure> {
    let rules = Rules::compile(&config.require::<Vec<String>>("firewall.rules")?)?;   // may fail
    let live = self.rules.clone();                                                    // Snapshot<Rules>
    Ok(Commit::new(move || { let _version = live.publish(rules); }))
}
```

## `runtime`

**Since** 0.4 · feature `kernel`

Where background work runs: supervised worker threads and timers, without an async runtime and with zero CPU when idle. The kernel owns the runtime, asks it to stop at shutdown step 3, and reports its health.

### `Runtime`

**Since** 0.4 · SUI-14

```rust,ignore
#[derive(Clone)]
pub struct Runtime { /* Arc: the worker supervisor and the timers */ }
impl Runtime {
    pub fn every<F>(&self, name: &'static str, period: Duration, tick: F) -> Result<TimerHandle, Error>
    where F: FnMut() + Send + 'static;
    pub fn after<F>(&self, name: &'static str, delay: Duration, fire: F) -> Result<TimerHandle, Error>
    where F: FnOnce() + Send + 'static;
    pub fn workers(&self) -> Vec<WorkerHandle>;
}
```

The handle to the runtime, from `Foundation::runtime()` (and `System::runtime()`, `Kernel::runtime()`, `Link::foundation().runtime()`). Cheap to clone, `Send + Sync`. Workers are spawned with [`Worker::spawn`](#workers-worker-workercontext-workerhandle), timers with `every` and `after`; `workers` lists the live workers for a dump or a status page.

### Workers: `Worker`, `WorkerContext`, `WorkerHandle`

**Since** 0.4 · SUI-14

```rust,ignore
#[must_use = "a Worker does nothing until spawn()"]
pub struct Worker { /* name and policy */ }
impl Worker {
    pub fn new(name: impl Into<Cow<'static, str>>) -> Worker;
    pub fn restart(self, policy: Restart) -> Self;                    // default Restart::OnFailure
    pub fn backoff(self, backoff: Backoff) -> Self;                   // default exponential, 100 ms to 30 s
    pub fn budget(self, restarts: u32, window: Duration) -> Self;     // default 5 within 60 s
    pub fn critical(self, on: bool) -> Self;                          // default false
    pub fn heartbeat(self, every: Duration) -> Self;                  // default none
    pub fn stack_size(self, bytes: usize) -> Self;                    // default: the platform's
    pub fn spawn<F>(self, runtime: &Runtime, run: F) -> Result<WorkerHandle, Error>
    where
        F: FnMut(&WorkerContext) -> Result<(), Failure> + Send + 'static;
}

pub struct WorkerContext { /* name, restarts, heartbeat word, urgent line */ }
impl WorkerContext {
    pub fn name(&self) -> &str;
    pub fn restarts(&self) -> u32;
    pub fn heartbeat(&self);                              // one relaxed atomic store
    pub fn should_stop(&self) -> bool;                    // this worker was asked to stop, or the line is at Stopping
    pub fn urgent(&self) -> &UrgentLine;
    pub fn park(&self, timeout: Option<Duration>);        // zero CPU until woken, stopped, or the timeout
}

#[derive(Clone)]
pub struct WorkerHandle { /* shared with the supervisor */ }
impl WorkerHandle {
    pub fn name(&self) -> &str;
    pub fn state(&self) -> WorkerState;
    pub fn restarts(&self) -> u32;
    pub fn wake(&self);                                   // unparks the worker's park()
    pub fn stop(&self);                                   // asks this worker to stop; does not wait
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WorkerState { Running, BackingOff, Stopped, Failed }
```

A named thread the kernel supervises: the services' own housekeeping (log shipping, metrics export, cache sweeps, a job queue's consumer). Long-running job systems belong to task-io; workers are the building block below it.

- `run` is called on the worker's thread. Returning `Ok(())` ends the worker (restarted only under `Restart::Always`, and never once a stop was requested). Returning `Err` or panicking is a failure: it is logged and counted, and the worker restarts per its policy after its backoff.
- A worker that fails more than its budget allows within the window is marked `Failed` and reported `Unhealthy`. If it is `critical`, the kernel requests a stop with `StopReason::WorkerFailed` (exit code 70).
- With `heartbeat(every)`, a worker that has not called `heartbeat()` for three periods is reported `Degraded` (the heartbeat word is written only by the worker, and model-checked with `loom`, D-058).
- At shutdown step 3 every worker is asked to stop (`should_stop()` becomes true and `park` returns) and joined within the deadline; one that does not return is abandoned and reported.
- Under `panic = "abort"` a panic ends the process after the crash report; restarts on panic apply to `panic = "unwind"` builds.

Errors from `spawn`: `Setup` (invalid or duplicate name, zero budget window), `Stopping` (a stop was requested), `Kernel` (the OS refused the thread).

```rust,ignore
use system_suite::runtime::{Backoff, Restart, Worker};

let shipper = Worker::new("log-shipper")
    .restart(Restart::OnFailure)
    .backoff(Backoff::exponential(Duration::from_millis(200), Duration::from_secs(60)))
    .heartbeat(Duration::from_secs(10))
    .spawn(system.runtime(), |cx| {
        while !cx.should_stop() {
            ship_pending_lines()?;               // a Failure: restarted after the backoff
            cx.heartbeat();
            cx.park(Some(Duration::from_secs(5)));
        }
        Ok(())
    })?;
```

```rust,ignore
// A queue consumer: zero CPU while the queue is empty.
let consumer = Worker::new("jobs").critical(true).spawn(system.runtime(), move |cx| {
    while !cx.should_stop() {
        match queue.pop() {
            Some(job) => job.run()?,
            None => cx.park(None),               // the producer calls consumer.wake()
        }
    }
    Ok(())
})?;
queue.push(job);
consumer.wake();
```

### `Restart` and `Backoff`

**Since** 0.4 · SUI-14

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Restart { Never, #[default] OnFailure, Always }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Backoff { None, Fixed(Duration), Exponential { min: Duration, max: Duration } }
impl Backoff {
    pub const fn exponential(min: Duration, max: Duration) -> Backoff;
    pub fn delay(&self, attempt: u32) -> Duration;     // saturating; full jitter of up to 25%
}
```

`OnFailure` restarts after an `Err` or a panic; `Always` also after `Ok(())`; `Never` never. The backoff delay doubles from `min` to `max` with up to 25% jitter (a mix of the clock's nanoseconds and the attempt number in 0.4; mod-rand's generator once `rand` is wired in 0.5), so many workers failing together do not restart together.

```rust
use system_suite::runtime::Backoff;
use std::time::Duration;

let b = Backoff::exponential(Duration::from_millis(100), Duration::from_secs(30));
assert!(b.delay(0) <= Duration::from_millis(125));
assert!(b.delay(20) <= Duration::from_millis(37_500));
```

### Timers: `Runtime::every`, `Runtime::after`, `TimerHandle`

**Since** 0.4 · SUI-14

```rust,ignore
impl Runtime {
    pub fn every<F>(&self, name: &'static str, period: Duration, tick: F) -> Result<TimerHandle, Error>
    where F: FnMut() + Send + 'static;
    pub fn after<F>(&self, name: &'static str, delay: Duration, fire: F) -> Result<TimerHandle, Error>
    where F: FnOnce() + Send + 'static;
}

#[derive(Clone)]
pub struct TimerHandle { /* the timer's name and state */ }
impl TimerHandle {
    pub fn name(&self) -> &'static str;
    pub fn cancel(&self) -> bool;          // false if it already fired (one-shot) or was cancelled
    pub fn is_active(&self) -> bool;
}
```

Periodic and one-shot timers on one kernel thread, ordered in a min-heap of due times (D-046): no thread per timer, the thread starts with the first timer, and with nothing due it waits with no timeout (FR-10, NFR-2). Resolution is the OS's condition-variable timeout (about 1 ms on Linux, up to the scheduler tick on Windows). A periodic timer fires first after one period; ticks missed while a callback ran long are skipped, not replayed in a burst. Callbacks run on the timer thread and must return quickly (well under a millisecond); heavier work wakes a worker. A callback that panics under `panic = "unwind"` cancels its timer and is reported `Degraded`. At shutdown step 3 every timer is cancelled (`is_active` becomes false) and the thread is joined within the deadline.

Errors: `Setup` (invalid name, zero period, a period over 30 days), `Stopping`, `Kernel` (the OS refused the timer thread).

Cost [est.]: arming O(log n), one allocation for the callback, about 100 ns; cancelling one atomic compare-and-swap (the entry is dropped when it comes due); a fire O(log n) plus the callback.

```rust,ignore
let flush = system.runtime().every("metrics-export", Duration::from_secs(15), || {
    export_metrics();
})?;
let log = system.log().clone();
let warn = system.runtime().after("slow-start-warning", Duration::from_secs(30), move || {
    system_suite::log::warn!(log, "warm-up is taking longer than expected");
})?;
warn.cancel();
```

## `config`

**Since** 0.4 to 0.5 · SUI-15 · feature `config`

The suite's view of the configuration: one file per application, one section per member, read through config-lib (D-034) in its `conf` format. The view below does not depend on the backend: a later format (NOML or TOML through `config-noml` / `config-toml`, or a change of backend) changes the accepted file syntax, not these signatures (D-018). config-lib itself is re-exported as `config::backend` for anything the view does not cover.

### `Config`

```rust,ignore
#[derive(Clone)]
pub struct Config { /* Arc: the parsed document, its source, its version */ }
impl Config {
    pub fn from_text(text: &str) -> Result<Config, Error>;   // parse `conf` text: tests, embedded defaults
    pub fn get<T: FromConfig>(&self, path: &str) -> Result<Option<T>, Error>;
    pub fn require<T: FromConfig>(&self, path: &str) -> Result<T, Error>;
    pub fn get_or<T: FromConfig>(&self, path: &str, default: T) -> Result<T, Error>;
    pub fn contains(&self, path: &str) -> bool;
    pub fn keys(&self, section: &str) -> Vec<&str>;
    pub fn source(&self) -> Option<&Path>;          // None for config_str or defaults only
    pub fn version(&self) -> u64;                   // 1 at boot, +1 per applied reload
    #[cfg(feature = "secrets")]
    pub fn secret(&self, path: &str) -> Result<Secret, Error>;
    pub fn backend(&self) -> &backend::Config;      // config-lib's own type
}
```

An immutable snapshot of the configuration. Keys are dotted paths (`"database.pool.size"`). `get` returns `Ok(None)` for an absent key and an error of kind `ConfigInvalid` (naming the key, the expected type, and the type found, never the value) for a present key of the wrong type. `Config` is `Send + Sync`; cloning it is one reference-count increment. A handler that wants the newest configuration on every request keeps a `Reader` from `Foundation::config_snapshot()`.

```rust,ignore
let config = system.config();
let port: u16 = config.get_or("http.port", 8080)?;
let hosts: Vec<String> = config.require("cluster.hosts")?;
let timeout: Duration = config.get_or("http.read_timeout", Duration::from_secs(30))?;
if config.contains("tls") {
    let cert: PathBuf = config.require("tls.cert")?;
}
```

```rust,ignore
// Per-thread reader: about a nanosecond per request, the newest version after a reload.
let mut reader = foundation.config_snapshot().reader();
loop {
    let limit: u32 = reader.get().get_or("http.max_body_kib", 1024)?;
    serve_one(limit)?;
}
```

### `FromConfig`

```rust,ignore
pub trait FromConfig: Sized + private::Sealed { /* conversion from the backend's value */ }
```

Implemented for `bool`, `i8` to `i64`, `u8` to `u64`, `usize`, `f64`, `String`, `PathBuf` (a leading `~` expanded through path-lib), `Duration` (an integer number of seconds, or a string with a unit: `"500ms"`, `"30s"`, `"5m"`, `"2h"`), and `Vec<T>` of any of these. Integers are range-checked into the target type. The trait is sealed so the suite can change the backend without breaking implementations; structured values are read field by field, or through `backend()`.

### `Secret`

**Since** 0.5 · feature `secrets`

```rust,ignore
pub struct Secret { /* the key's name and its vault handle */ }
impl Secret {
    pub fn name(&self) -> &str;
    pub fn with<T>(&self, use_it: impl FnOnce(&[u8]) -> T) -> Result<T, Error>;
}
impl std::fmt::Debug for Secret;     // Secret("database.password", [redacted])
```

Configuration holds references, never secrets (FR-16, threat T2). A value of the form `env:NAME`, `file:/path`, or `vault:name` is resolved at boot step 8 (and again on reload) into key-vault; `Config::secret` returns a handle, and the bytes are only visible inside `with`, through key-vault's scoped buffer that is wiped afterwards.

| Reference | Resolved from | Refused when |
|---|---|---|
| `env:NAME` | the process environment, then the `.env` file (env-lib) | unset or empty |
| `file:/path` | the file's contents, trailing newline removed (key-vault's file fetcher) | missing, over 64 KiB, or on Unix group- or world-readable, or not owned by the service user |
| `vault:name` | a key the application registered in the vault before boot step 8 (`Builder::on_vault`, 0.5) | no such name |

A key whose last segment marks a secret (`password`, `passwd`, `secret`, `token`, `api_key`, `private_key`, `credentials`, and the rest of log-io's list of common secret keys) must hold a reference; a plain value is refused with `PlainSecret` unless `[secrets] allow_plain = true`. Without the `secrets` feature, any reference is refused with `Secret` ("enable the secrets feature"), so a reference is never mistaken for a literal password.

```rust,ignore
// site.conf:  [database]  password = "file:/run/secrets/db_password"
let password = system.config().secret("database.password")?;
let pool = password.with(|bytes| Pool::connect(&url, bytes))??;
```

Cost: resolution at boot and reload only; `with` is key-vault's read path [measured in key-vault].

The vault is built with key-vault's BLAKE3 normalization off, so `with` sees the bytes that were referenced. Each configuration version stores its values under its own names: a reload resolves the new version before subsystems prepare (they can read the new secrets in `reload`), a refused reload removes them again, and a `Secret` taken from the configuration in force keeps its value through one reload (D-064). A `file:` reference is checked on the opened file (Unix: no group or other permission bit; owner the service user or root), at most 64 KiB, with one trailing newline removed. The values are removed from the vault (wiped) at shutdown step 7.

### Finding the file

The first that applies wins:

1. `Builder::config_file(path)` or `Builder::config_str(text)`;
2. the `<NAME>_CONFIG` variable (from the process environment or the `.env` file);
3. `<config dir>/<name>.conf` (with `config-toml` or `config-noml`, also `<name>.toml` or `<name>.noml`; finding two is a `ConfigInvalid` error naming both; the format follows the extension, D-059);
4. `./<name>.conf` (and the same alternatives) in the working directory, only when the process is not running as root or an administrator (a working directory is easy to influence; a privileged service must name its file).

In cases 1 and 2 a missing file is an error. In cases 3 and 4 a missing file means "no file": every section takes its defaults and the first log lines say where the configuration came from. Before parsing, the file's owner and permissions are checked on the opened file, so the file checked is the file read (the 0.2 defence idea, D-020, and the 0.4 one, D-049): on Unix a file that is group- or world-writable, or owned by a user other than the service user or root, is refused with `ConfigInsecure`. There is no switch to accept such a file (D-048): the fix is `chmod 640` (or `600`) and, if needed, `chown`. The `.env` file is checked the same way before it is read, because it can name the configuration file. A file over 1 MiB is refused with `ConfigInvalid`. A parse error names the line and column and never quotes the text, which may hold a secret.

### Sections

Every section is optional and every key has a secure default. The complete reference with every key, type, and default is `docs/CONFIGURATION.md` (written in 0.5); the keys below are the ones that shape this API.

| Section | Keys (default) | Since |
|---|---|---|
| `[guard]` | `lock` (`true`), `lock_dir` (the runtime directory), `allow_network_lock` (`false`), `umask` (`"0027"`), `open_files_target` (`65536`), `open_files_minimum` (`1024`) | 0.4 |
| `[log]` | `level` (`"info"`), `filter`, `format` (`"auto"`: `"human"` on a terminal, `"json"` otherwise; or `"logfmt"`), `stderr` (`true`), `file` (`true` when a log directory exists), `max_bytes`, `rotation` (`"daily"`; `"hourly"`, `"never"`), `max_files` (`14`), `redact` (list, `["a", "b"]`), `redact_common` (`true`), `recent_lines` (`64`, for crash reports) | 0.4 |
| `[crash]` | `enabled` (`true`), `max_reports` (`8`) | 0.4 |
| `[metrics]` | `enabled` (`true`), `max_series` (`10000`); `process` would come with `metrics-system`, not accepted (D-056) | 0.4 |
| `[kernel]` | `shutdown_deadline` (`"25s"`), `member_reserve` (`"2s"`), `notify` (`"auto"`), `reload_signal` (`true`), `dump_signal` (`true`) | 0.4 |
| `[ids]` | `worker` (`0`; 0 to 1023), `epoch` (milliseconds since 1970; id-forge's default) | 0.4 |
| `[audit]` | `enabled` (`true` when compiled in), `file` (`<data dir>/audit/<name>.audit`), `hash` (`"sha256"`), `sync` (`"every"`; or `"flush"`), `repair_tail` (`false`) | 0.5 |
| `[secrets]` | `allow_plain` (`false`) | 0.5 |

Settings given to the `Builder` win over the file. There are no implicit environment overrides of individual keys (D-021); a value that must come from the environment says so with a reference. An unknown key in one of these sections is refused with `ConfigInvalid`, so a misspelt key cannot silently leave a setting at its default; other sections belong to the application and are not checked.

On reload, `[kernel] shutdown_deadline` and `member_reserve` take effect at once (unless the `Builder` set them); changes to `[guard]`, `[log]`, `[crash]`, `[metrics]`, `[ids]`, and the `[kernel]` signal and notification keys are logged as "takes effect at restart".

## `AuditLog`

**Since** 0.5 · SUI-29 · feature `audit`

```rust,ignore
#[derive(Clone)]
pub struct AuditLog { /* Arc<Mutex<audit-trail chain over a file sink>> */ }
impl AuditLog {
    pub fn record(&self, actor: &str, action: &str, target: &str, outcome: audit::Outcome)
        -> Result<audit::RecordId, Error>;
    pub fn sync(&self) -> Result<(), Error>;
    pub fn path(&self) -> Option<&Path>;          // None when audit is disabled (D-060)
    pub fn next_id(&self) -> Option<audit::RecordId>;
    pub fn is_enabled(&self) -> bool;
}
```

The application's tamper-evident record of privileged actions (REPS: who did what and when, without sensitive content). The chain file is opened at boot step 13, every existing record is verified, and appending continues at the tail; a chain that fails verification stops the boot with `AuditTampered`. `record` appends one hash-linked record; with `[audit] sync = "every"` (the default) it is on disk before `record` returns. The kernel records its own privileged actions here (a reload, an urgent-line change to `Lockdown`, a stop request with its reason).

Cost [est.]: `record` 1 to 3 µs plus one `fdatasync` with `sync = "every"`; with `sync = "flush"` records are synced at shutdown and every second.

```rust,ignore
system.audit().record("admin:42", "user.delete", "user:1009", system_suite::audit::Outcome::Success)?;
```

## Member modules

**Since** as each member is wired · SUI-24

Each member crate is re-exported whole, so a project depends only on `system-suite` and uses the members' own APIs directly.

| Module | Crate | Feature | Since |
|---|---|---|---|
| `system_suite::sys` | sys-lib | `sys` | 0.3 |
| `system_suite::path` | path-lib | `paths` | 0.3 |
| `system_suite::env` | env-lib | `env` | 0.3 |
| `system_suite::config::backend` | config-lib | `config` | 0.4 |
| `system_suite::log` | log-io | `log` | 0.3 |
| `system_suite::metrics` | metrics-lib | `metrics` | 0.4 |
| `system_suite::signal` | signal-mod | `signals` | 0.3 |
| `system_suite::clock` | clock-lib | `time` | 0.3 |
| `system_suite::id` | id-forge | `ids` | 0.3 |
| `system_suite::audit` | audit-trail | `audit` | 0.5 |
| `system_suite::registry` | registry-io | `registry` | 0.5 |
| `system_suite::global` | global-lib | `globals` | 0.5 |
| `system_suite::rand` | mod-rand | `rand` | 0.5 |
| `system_suite::vault` | key-vault | `secrets` | 0.5 |

The members' macros work through the re-export (`system_suite::log::info!`, `system_suite::metrics::counter!`, `system_suite::build_info!`). `system_suite::log` and `system_suite::rand` have the same names as the `log` and `rand` crates; a project that also uses those writes `use system_suite::log as slog;` (D-015).

```rust,ignore
use system_suite::log as slog;

slog::info!(system.log(), "order placed", order = order_id, total_cents = 1299_u64);
slog::info!("through the process-wide logger");     // works unless process_wide(false)
```

## Signals

**Since** 0.3 · feature `signals`

Signals reach only the kernel (D-006, invariant). The kernel installs signal-mod's intake at boot step 7 and turns each signal into a lifecycle decision; nothing else in the process receives raw signals.

| Signal | Unix | Windows | Kernel decision | Since |
|---|---|---|---|---|
| terminate | SIGTERM | console close, logoff, system shutdown, service stop and preshutdown | stop | 0.3 |
| interrupt | SIGINT | Ctrl+C | stop | 0.3 |
| quit | SIGQUIT | Ctrl+Break | stop | 0.3 |
| reload | SIGHUP | service `PARAMCHANGE` | `reload` | 0.4 |
| dump | SIGUSR1 | service user control code 128 | the health report, phase, urgent level, workers, build and instance info written to the log | 0.4 |
| a second stop signal during shutdown | | | recorded and ignored: the shutdown already has a deadline | 0.3 |

The Windows service rows come from signal-mod 2.1's service control (D-065): `System::run` hands the main thread to the service control manager when the process was started by it. `[kernel] reload_signal` and `dump_signal` turn those two off.

## Versioning

- A member's patch release needs no suite release.
- A member's minor release that changes something re-exported here is a suite minor release.
- A member's major release is a suite minor release before 1.0 and a suite major release after 1.0, unless the suite hides the change completely.
- The suite's MSRV is the highest MSRV among its default members (1.89, sys-lib's); raising it is a major change after 1.0.

## Item index

| Id | Item | Tier | Since |
|---|---|---|---|
| SUI-01 | `VERSION` | 1 | 0.1 |
| SUI-02 | `start!` | 1 | 0.3 |
| SUI-03 | `System::run` | 1 | 0.3 |
| SUI-04 | typed handles on `System` and `Foundation` | 1 | 0.3 to 0.5 |
| SUI-05 | `Error::report` | 1 | 0.3 |
| SUI-06 | `build::emit`, `build_info!` | 1 | 0.3 |
| SUI-10 | `Builder` | 2 | 0.3 to 0.5 |
| SUI-11 | `System::request_stop`, `System::reload` | 2 | 0.4 |
| SUI-12 | `System::health`, `System::status`, `Health`, `Status`, `HealthReport` | 2 | 0.4 |
| SUI-13 | `lifecycle::{Subsystem, Position, Link, Mailbox, Failure, Commit}` | 2 | 0.4 |
| SUI-14 | `runtime::{Runtime, Worker, WorkerContext, WorkerHandle, Restart, Backoff, TimerHandle}` | 2 | 0.4 |
| SUI-15 | `config::Config`, `FromConfig`, `Secret` | 2 | 0.4 (`Secret` 0.5) |
| SUI-16 | `exit` codes | 2 | 0.3 |
| SUI-20 | `Foundation` | 3 | 0.3 |
| SUI-21 | `UrgentLine`, `Level`, `Watch`, `Notice` | 3 | 0.4 |
| SUI-22 | `Snapshot`, `Reader` | 3 | 0.4 |
| SUI-23 | `Error`, `ErrorKind`, `Result`, `Member`, `Step` | 3 | 0.3 |
| SUI-24 | member modules | 3 | as wired |
| SUI-25 | `Builder::boot`, `Boot` | 3 | 0.4 |
| SUI-26 | `System::wait_for_stop`, `System::shutdown` | 3 | 0.3 |
| SUI-27 | `kernel::Kernel`, `lifecycle::{Phase, StopReason, Deadline}` | 3 | 0.3 |
| SUI-28 | `Event`, `Foundation::events` | 3 | 0.5 |
| SUI-29 | `AuditLog` | 3 | 0.5 |
| SUI-30 | `foundation()` | 3 | 0.5 |
