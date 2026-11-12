<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br><b>system-suite</b><br>
    <sub><sup>API REFERENCE &middot; v0.9.0</sup></sub>
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

A task-oriented tour of the 0.9 API. The rustdoc on
[docs.rs](https://docs.rs/system-suite) is the exhaustive reference; every
example here is compiled as a doctest with all features enabled, and run
unless it would boot a real service (those are marked `no_run`). The boot
and shutdown orders are in [`BOOT-ORDER.md`](./BOOT-ORDER.md), every
configuration key in [`CONFIGURATION.md`](./CONFIGURATION.md).

<sub>Copyright &copy; 2026 <strong>James Gober</strong>.</sub>

- [Setup](#setup)
- [Tier 1: a whole service](#tier-1-a-whole-service)
- [Errors and exit codes](#errors-and-exit-codes)
- [The builder](#the-builder)
- [Subsystems](#subsystems)
- [Workers and timers](#workers-and-timers)
- [Configuration](#configuration)
- [Stopping, health, and phases](#stopping-health-and-phases)
- [The urgent line](#the-urgent-line)
- [Events](#events)
- [Audit](#audit)
- [Frameworks](#frameworks)
- [Member modules](#member-modules)
- [Conventions](#conventions)
- [Versioning](#versioning)
- [Migrating from 0.8](#migrating-from-08)
- [Examples in the repository](#examples-in-the-repository)

## Setup

```toml
[dependencies]
system-suite = "0.9"
# or pick members:
# system-suite = { version = "0.9", default-features = false, features = ["kernel", "log", "config"] }

[build-dependencies]
# Optional: the build commit and time in the first log line.
system-suite = { version = "0.9", default-features = false, features = ["sys"] }
```

### Features

Every member is behind its own feature and every feature is additive:
turning one off removes that member's boot step and its accessor, and the
crate still builds. No feature brings an async runtime.

| Feature | Default | Member | Brings | Third-party runtime dependencies |
|---|---|---|---|---|
| `kernel` | yes | this crate: `kernel`, `lifecycle`, `runtime` | stop decisions, urgent line, subsystems, workers, timers, health, crash capture, readiness | none |
| `sys` | yes | sys-lib 0.5 | build info, instance info, process guard | none |
| `paths` | yes | path-lib 1.2 | application and OS directories, owner-only where the suite writes; file access checks on Windows | none |
| `env` | yes | env-lib 1 (implies `paths`) | `.env` file and variables | none |
| `config` | yes | config-lib 1.3, `conf` format (implies `paths`) | one configuration file, reload | thiserror, serde, dashmap, arc-swap (through config-lib) |
| `log` | yes | log-io 2.1 | structured logging, the crash report's recent lines | none |
| `metrics` | yes | metrics-lib 2.1 (counters, gauges, timers) | the system's registry and the kernel's own metrics | none |
| `signals` | yes | signal-mod 2.1, runtime-free back-end (implies `kernel`) | stop, reload, dump signals; Windows service control | libc and signal-hook (Unix), windows-sys (Windows) |
| `time` | yes | clock-lib 1.1 | monotonic and wall clocks, a mockable `Clock` | none |
| `ids` | yes | id-forge 1 | ULID, UUID, Snowflake | none |
| `audit` | no (D-035) | audit-trail 1.2 with sha2 (implies `kernel`, `paths`) | tamper-evident audit chain | sha2 |
| `registry` | no | registry-io 1.1 (implies `kernel`) | kernel events delivered to callbacks | none |
| `globals` | no | global-lib 1 | process-wide typed values; `system_suite::foundation()` | none |
| `rand` | no | mod-rand 1 | OS CSPRNG and fast generators | none |
| `secrets` | no | key-vault 1.2 (implies `kernel`, `config`, `rand`) | secret references resolved into a page-locked vault | subtle, getrandom, blake3, zeroize, arc-swap |
| `full` | no | all of the above | | as above |

Every third-party crate in this table is a reviewed choice recorded in the
dependency register of [`../dev/PLAN.md`](../dev/PLAN.md) (D-032), and
`cargo audit` and `cargo deny` check them on every CI build.

### Member features passed through

A member's own optional features are reached through the suite, so a
project keeps one dependency line (D-036). Each is named
`<suite feature>-<member feature>` and turns on the suite feature it
extends.

| Feature | Turns on | Brings |
|---|---|---|
| `time-serde` | clock-lib's `serde`: `Wall` and `Monotonic` serialise | serde |
| `metrics-histogram`, `metrics-meter` | metrics-lib's histogram and summary, rate meter | none |
| `metrics-statsd` | metrics-lib's StatsD and DogStatsD push (std UDP) | none |
| `metrics-http` | metrics-lib's blocking `/metrics` server and Pushgateway client (std TCP) | none |
| `metrics-json` | metrics-lib's JSON exporter on jsonx-lib: `metrics::exporters::json::to_json` | jsonx-lib |
| `metrics-otlp` | metrics-lib's OTLP exporter on jsonx-lib: `exporters::otlp::to_json` and protobuf | jsonx-lib |
| `metrics-tracing` | metrics-lib's `tracing` integration | tracing |
| `log-bridge` | log-io's `log`: records from crates that use the `log` facade go to the suite's logger (installed at boot step 10 in process-wide mode) | log |
| `log-tracing` | log-io's `tracing`: `log::compat::TracingLayer`, a layer the application adds to its own subscriber | tracing-core, tracing-subscriber |
| `config-toml`, `config-noml` | config-lib's TOML and NOML formats: `<name>.toml`, `<name>.noml` | noml, indexmap |
| `config-json` | config-lib's JSON format on jsonx-lib: `<name>.json`; a repeated key is refused, depth and size budgets apply | jsonx-lib |
| `audit-blake3` | audit-trail's BLAKE3 hasher (`[audit] hash = "blake3"`) | blake3 |

Not passed through: anything that brings an async runtime (signal-mod's
`tokio`, metrics-lib's `async`, config-lib's `async`), config-lib's
`hot-reload` (an idle watcher thread; the suite reloads by signal or
call), and the features held until a first-party crate covers the same
ground (config-lib's XML and rule-based validation, metrics-lib's
`system`; D-056). Adding a member directly with the feature it wants also
works, because Cargo merges the features of one crate version; the
pass-through is the same thing without the second line.

### Build info in `build.rs`

Build info (commit, dirty flag, build time, profile, target, rustc,
features) is captured in the application's `build.rs` and read by
`build_info!()`, which must expand in the application crate. That is why
the Tier 1 entry points are macros (D-008). Without the `build.rs` line
everything still compiles and the commit and time read `"unknown"`.

```rust,no_run
// build.rs
fn main() {
    system_suite::build::emit();
}
```

`start!` and `run!` pass `build_info!()` for you; with the builder, pass it
yourself:

```rust
use system_suite::{Member, System};

let system = System::builder("api-doc-build")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-build"))
    .build_info(system_suite::build_info!())
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .start()?;
let build = system.build_info().map(|build| build.profile);
assert!(build.is_some());
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

## Tier 1: a whole service

### `run!`

```rust,no_run
fn main() -> std::process::ExitCode {
    system_suite::run!()
}
```

`run!()` names the application after its package (`run!("hive")` names it
explicitly), boots every enabled member and the kernel in the boot order,
parks the main thread with zero CPU until a stop (SIGTERM, SIGINT, Ctrl+C,
a console close, a Windows service stop, or `Kernel::request_stop`), stops
everything in reverse within the shutdown deadline, and returns the exit
code. When the boot fails it writes the [report](#errorreport) to standard
error and returns the exit code for the kind of failure.

The name is 1 to 64 ASCII letters, digits, `-`, `_`, `.`, not starting
with `.`. It names the directories, the lock, the configuration file
(`<name>.conf`), the environment prefix (`SITE_` for `site`), and the log
target.

| Outcome | Exit code |
|---|---|
| clean stop, whatever requested it | `0` |
| a subsystem or worker missed the shutdown deadline and was abandoned | `79` (`exit::DEADLINE`) |
| a critical worker exhausted its restart budget | `70` (`exit::SOFTWARE`) |
| the boot failed | the error kind's code (see [Errors and exit codes](#errors-and-exit-codes)) |

On Windows run as a service, `run` hands the main thread to the service
control manager for the life of the service; started any other way, it
runs in the foreground.

### `start!` and `System::run`

With work of its own between the boot and the wait, a service uses
`start!`, which returns `Result<System, Error>`, and `System::run`:

```rust,no_run
fn main() -> Result<std::process::ExitCode, system_suite::Error> {
    let system = system_suite::start!()?;
    let port: u16 = system.config().get_or("http.port", 8080)?;
    system_suite::log::info!(system.log(), "ready", port = port);
    Ok(system.run())
}
```

An `Error` that leaves `main` through `?` prints the same block as
`Error::report` (its `{:?}` is that block), but the process exit code is
then `1`. `run!()` and `Error::report` return the code for the kind of
failure, which is what supervisors and scripts read; to keep it with
`start!`, match on the result:

```rust,no_run
fn main() -> std::process::ExitCode {
    let system = match system_suite::start!("hive") {
        Ok(system) => system,
        Err(error) => return error.report(),
    };
    system.run()
}
```

### Typed handles

Every enabled member is reached through a method that returns its handle
directly. Members are resolved once at boot; a handle access is a field
read. The accessors live on [`Foundation`](#foundation), and `System`
dereferences to it, so `system.log()` and `foundation.log()` are the same
method.

```rust
use system_suite::{Member, System};

let system = System::builder("api-doc-handles")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-handles"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .config_str("[http]\nport = 9090\n")
    .start()?;
system_suite::log::info!(system.log(), "listening", port = 9090_u16);
let port: u16 = system.config().get_or("http.port", 8080)?;
assert_eq!(port, 9090);
system.metrics().counter("requests_total").inc();
let uptime = system.instance().uptime();
let data_dir = system.paths().data();
# let _ = (uptime, data_dir);
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

### `Error::report`

`Error::report` writes the error to standard error as one block (what
failed, at which step, and what to do) and returns its exit code. A boot
failure before logging opens has nowhere else to go; one after it was
also logged. It writes with one `write_all` and never panics.

```text
site: boot step 6 (guard) failed: another copy is running (pid 4182, lock /run/user/1000/site/site.lock).
  do: stop the other copy, or start this one with a different name.
```

## Errors and exit codes

One [`Error`] type for the whole crate. It names the kind, the boot step
and member it happened in, the subsystem for subsystem failures, and what
to do; the member's own error is its `source()`. Neither `Display` nor
`Debug` ever contains a secret or a configuration value: they name keys
and files only. `{:?}` is the report block; `{:#?}` shows the fields.

```rust
use system_suite::{ErrorKind, Member, System, exit};

let error = System::builder("api-doc-error")
    .process_wide(false)
    .disable(Member::Kernel)            // the kernel cannot be disabled at run time
    .start()
    .err()
    .ok_or("the boot should have failed")?;
assert_eq!(error.kind(), ErrorKind::Setup);
assert_eq!(error.step(), None);         // refused before any step ran
assert_eq!(error.exit_code(), std::process::ExitCode::from(exit::USAGE));
assert!(!error.action().is_empty());
assert!(format!("{error:?}").contains("do: "));
# Ok::<(), Box<dyn std::error::Error>>(())
```

A kind can be matched to treat one failure differently:

```rust,no_run
use system_suite::ErrorKind;

fn main() -> std::process::ExitCode {
    match system_suite::start!() {
        Ok(system) => system.run(),
        // A copy started by cron while the service runs: nothing to do.
        Err(error) if error.kind() == ErrorKind::AlreadyRunning => std::process::ExitCode::SUCCESS,
        Err(error) => error.report(),
    }
}
```

### Error kinds

`ErrorKind` is `#[non_exhaustive]`.

| Kind | Meaning | What to do | Exit code |
|---|---|---|---|
| `Setup` | an invalid builder setting: bad name, duplicate subsystem, a member that cannot be disabled, a zero deadline | fix the code; the message names the setting | 64 |
| `Entropy` | the OS random source is unavailable (a sandbox or seccomp filter blocks it) | allow `getrandom` / `BCryptGenRandom` | 69 |
| `Paths` | an application directory cannot be resolved or created, or is a symlink | set `<NAME>_<KIND>_DIR` or fix the permissions | 73 |
| `Environment` | the `.env` file is missing (when named explicitly), malformed, too large, or unreadable | fix the line named in the message | 78 |
| `ConfigNotFound` | an explicit configuration file (builder or `<NAME>_CONFIG`) does not exist | create it, or fix the path | 78 |
| `ConfigInsecure` | the configuration or `.env` file can be changed by another user (Unix; D-020) | `chmod 640` (or `600`) and `chown`; there is no switch to accept it (D-048) | 77 |
| `ConfigInvalid` | the file does not parse, a suite section has an unknown key or invalid value, or a value read later has the wrong type | the message names the file, line, key, and accepted values | 78 |
| `AlreadyRunning` | another copy holds the single-instance lock | stop the other copy, or use another name | 75 |
| `LockDirInsecure` | the lock directory cannot be created, is writable by others, a symlink, or on a network filesystem | the message gives the fix | 77 |
| `Limits` | the hard open-file limit is below `[guard] open_files_minimum` | raise `LimitNOFILE=`, `ulimit -Hn`, or the launchd limit | 69 |
| `Signals` | the signal intake could not be installed | another library may own the signal; or `disable(Member::Signals)` | 70 |
| `Secret` | a secret reference cannot be resolved (unset variable, missing or too-open file, no such vault name), or the `secrets` feature is off | the message names the key, never the value | 78 |
| `PlainSecret` | a key whose name marks a secret holds a plain value | use `env:NAME`, `file:/path`, `vault:name`, or `[secrets] allow_plain = true` | 78 |
| `Logging` | a log sink cannot be opened | the message names the file and the I/O error | 73 |
| `Metrics` | the metrics registry rejected its configuration | the message names the key | 70 |
| `Audit` | the audit file cannot be opened, read, written, or synced | the message names the file | 74 |
| `AuditTampered` | the existing audit chain fails verification | do not delete the file: keep it as evidence, move it aside, investigate | 65 |
| `Kernel` | a kernel thread could not be started | check the process and thread limits | 70 |
| `Subsystem` | a subsystem's `start` or an `on_ready` callback failed or panicked | the message carries the subsystem's name and its `Failure` | 70 |
| `Interrupted` | a stop signal arrived during boot; the boot unwound cleanly | nothing: the operator asked for it | 0 |
| `Stopping` | the kernel no longer accepts the request (a worker, timer, or reload after a stop) | stop scheduling work | 70 |
| `ReloadRefused` | a subsystem refused the new configuration; nothing changed | the message names who refused and why | 78 |
| `UrgentRefused` | an urgent-line change that is not allowed | see [The urgent line](#the-urgent-line) | 70 |
| `Disabled` | an operation on a member disabled at run time | enable it, or do not call it | 70 |
| `Io` | an I/O error not covered above | the message names the operation and the file | 74 |

### Exit codes

The codes follow BSD `sysexits.h` (D-017), so a supervisor can tell a
configuration error from a second copy or a crash; `79` is the suite's
own. They are `u8` constants in `system_suite::exit`.

| Constant | Value | Used for |
|---|---|---|
| `OK` | 0 | a clean stop |
| `USAGE` | 64 | an invalid setup in the code |
| `DATA_ERROR` | 65 | the audit chain failed verification |
| `UNAVAILABLE` | 69 | a requirement of the environment is missing |
| `SOFTWARE` | 70 | an internal, subsystem, or critical worker failure |
| `CANNOT_CREATE` | 73 | a directory or file cannot be created |
| `IO_ERROR` | 74 | an I/O error |
| `TEMPORARY_FAILURE` | 75 | already running: try again later |
| `NO_PERMISSION` | 77 | insecure permissions refused |
| `CONFIG` | 78 | configuration missing or invalid |
| `DEADLINE` | 79 | the shutdown deadline was missed and something was abandoned |

```rust
use system_suite::exit;

assert_eq!(exit::TEMPORARY_FAILURE, 75);
let code = std::process::ExitCode::from(exit::CONFIG);
# let _ = code;
```

A panic ends the process as the application's profile decides (with
`panic = "abort"`, SIGABRT, shown as 134 by shells on Unix).

## The builder

`System::builder(name)` is the one way to configure a boot. Every setting
has a secure default, and a setting given here wins over the same setting
in the configuration file. Invalid settings are reported by `start` as
`Setup`, never by the setter, so a chain of setters needs no `?`.

```rust
use std::{sync::Arc, time::Duration};
use system_suite::clock::{Clock, ManualClock};
use system_suite::{Member, System};

let clock = Arc::new(ManualClock::new());
let system = System::builder("api-doc-builder")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-builder"))
    .process_wide(false)                         // touch no process-wide state
    .single_instance(false)                      // several copies may run
    .disable(Member::Log)                        // no log sinks
    .config_str("[http]\nport = 8080\n")         // the configuration text itself
    .clock(Arc::clone(&clock))                   // what system.clock() returns
    .shutdown_deadline(Duration::from_secs(5))
    .start()?;
let before = system.clock().now();
clock.advance(Duration::from_secs(60));
assert_eq!(system.clock().elapsed(before), Duration::from_secs(60));
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

| Setting | Default | Notes |
|---|---|---|
| `build_info(build)` | none: `build_info()` returns `None` | `start!` and `run!` set it; a missing value is logged once (D-038) |
| `scope(scope)` | `User`, or `<NAME>_PATH_SCOPE` (`user`, `global`, `auto`) | `Global` for system services; directory overrides come from `<NAME>_<KIND>_DIR` |
| `portable(root)` | none | every directory under `root` (`root/data`, `root/config`, ...) |
| `env_file(path)` | `<config dir>/.env`, if it exists | an explicit file that does not exist is `Environment` |
| `config_file(path)` | searched (see [Finding the file](#finding-the-file)) | an explicit file that does not exist is `ConfigNotFound` |
| `config_str(text)` | none | `impl Into<String>`, in the `conf` format; disables the file search, and a reload parses the same text again |
| `clock(clock)` | the OS clock | `impl Clock + 'static`; kernel deadlines and timers always use the OS monotonic clock (D-029) |
| `disable(member)` | none | turns a compiled-in member off for this run (below) |
| `process_wide(on)` | `true` | see [Process-wide state](#process-wide-state) |
| `crash_capture(on)` | `true` (D-016), or `[crash] enabled` | never installed with `process_wide(false)` |
| `single_instance(on)` | `true`, or `[guard] lock` | `false` when a supervisor holds the lock or several copies are meant to run |
| `shutdown_deadline(d)` | 25 s, or `[kernel] shutdown_deadline` | zero or over one hour is `Setup` |
| `member_reserve(d)` | 2 s, or `[kernel] member_reserve` | the part of the deadline kept for the members after the subsystems; must be shorter than the deadline |
| `register(subsystem)` | none | see [Subsystems](#subsystems) |
| `on_ready(callback)` | none | runs at boot step 17, after every subsystem started and before readiness is announced; a failure unwinds the boot |
| `on_vault(callback)` | none | runs at boot step 8, before `vault:` references are resolved (see [Secrets](#secrets)) |

`start()` runs every boot step and returns the `System`; on failure nothing
that started is left running. `boot()` runs steps 1 to 15 and returns a
[`Boot`](#staged-start-with-boot) for frameworks.

Members that others need at run time cannot be disabled; asking is a
`Setup` error.

| Can be disabled at run time | Cannot |
|---|---|
| `Env` (an empty environment), `Config` (defaults only), `Log` (no sinks), `Metrics`, `Signals`, `Audit`, `Registry`, `Globals`, `Secrets` | `Kernel`, `Sys`, `Paths`, `Time`, `Ids`, `Rand` |

A disabled member keeps its accessor (the build decides what exists): the
handles of `Audit` and `Secrets` then fail with `Disabled`, and the kernel
never notifies a disabled `Registry`.

## Subsystems

A subsystem is a part of the application that the kernel starts after the
members and stops before them: a database pool, a cache, a job runner, a
listener. Registration is the subscription: a registered subsystem
receives every stop and every urgent change through its `Link`.

| Method | Called | Contract |
|---|---|---|
| `name` | once, at registration | 1 to 64 ASCII letters, digits, `-`, `_`, `.`; unique in the system |
| `position` | once, at registration | the start order (default `Position::DEFAULT`) |
| `start(&mut self, &Link)` | boot step 16, on the booting thread | open what it needs; return quickly or start a worker for long work; an error or a panic unwinds the boot |
| `stop(&mut self, Deadline)` | shutdown step 2, on a kernel stop thread | finish in-flight work by the deadline; one that has not returned is abandoned (exit code 79) |
| `health(&self)` | on `health()`, `status()`, and the dump signal | cheap and non-blocking; while `start`, `stop`, or `reload` runs, the last reported health is used |
| `reload(&mut self, &Config)` | on a reload, before anything is applied | see [Reload](#reload) |

The trait needs `Send` (stop runs on a kernel thread), not `Sync`.

```rust
use std::sync::{Arc, Mutex};
use system_suite::lifecycle::{Deadline, Failure, Link, Position, Subsystem};
use system_suite::{Member, System};

type Journal = Arc<Mutex<Vec<String>>>;

struct Part { name: &'static str, position: Position, journal: Journal }

impl Subsystem for Part {
    fn name(&self) -> &str { self.name }
    fn position(&self) -> Position { self.position }
    fn start(&mut self, _link: &Link) -> Result<(), Failure> {
        self.journal.lock().unwrap().push(format!("start {}", self.name));
        Ok(())
    }
    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> {
        self.journal.lock().unwrap().push(format!("stop {}", self.name));
        Ok(())
    }
}

let journal = Journal::default();
let system = System::builder("api-doc-subsystems")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-subsystems"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .register(Part { name: "http", position: Position::LAST, journal: journal.clone() })
    .register(Part { name: "db", position: Position::DEFAULT, journal: journal.clone() })
    .start()?;
let _ = system.shutdown();
assert_eq!(*journal.lock().unwrap(), ["start db", "start http", "stop http", "stop db"]);
# Ok::<(), system_suite::Error>(())
```

### `Position`

Lower starts first, equal positions start in registration order, and the
stop order is always the exact reverse of the order in which subsystems
actually started. Providers that others depend on go early; the door
(anything that accepts outside work) goes at `LAST`, so it opens last and
closes first.

```rust
use system_suite::lifecycle::Position;

assert!(Position::FIRST < Position::DEFAULT && Position::DEFAULT < Position::LAST);
assert_eq!(Position::new(1_000).get(), 1_000);
```

### `Link` and `Mailbox`

What a subsystem receives at `start`: its name, the `Foundation` (the
kernel and its urgent line are `link.foundation().kernel()`), and its
`Mailbox`. `Link` is cheap to clone; keep
a clone to use it from the subsystem's own thread. The mailbox is the
urgent line's second tier: on every urgent change the kernel stores the
latest `Notice` in each subsystem's mailbox and wakes whoever waits on it,
so a parked thread acts at once. Notices coalesce: a waiter gets the
latest one, and its `previous` field says which level it replaced.

`report_error` records an error the subsystem handled but the operator
should see: logged at error level with the subsystem's name and counted
(`system_suite_subsystem_errors_total{subsystem}`).

```rust
use std::{thread::JoinHandle, time::Duration};
use system_suite::kernel::Level;
use system_suite::lifecycle::{Deadline, Failure, Link, Subsystem};
use system_suite::{Member, System};

struct Flusher { thread: Option<JoinHandle<()>> }

impl Subsystem for Flusher {
    fn name(&self) -> &str { "flusher" }
    fn start(&mut self, link: &Link) -> Result<(), Failure> {
        let link = link.clone();
        self.thread = Some(std::thread::spawn(move || loop {
            match link.mailbox().wait_timeout(Duration::from_millis(100)) {
                Some(notice) if notice.level >= Level::Draining => break,
                _ => {
                    // flush; a handled error goes to link.report_error(&error)
                }
            }
        }));
        Ok(())
    }
    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();       // the Draining notice already woke it
        }
        Ok(())
    }
}

let system = System::builder("api-doc-mailbox")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-mailbox"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .register(Flusher { thread: None })
    .start()?;
let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

`Mailbox::new()` makes a free-standing mailbox for tests.

### `Failure`

The error a subsystem, worker, or callback returns. Any
`std::error::Error + Send + Sync + 'static` converts into it with `?`, so
application code keeps its own error types; the kernel wraps it into an
`Error` of kind `Subsystem` that names the subsystem, with the `Failure`
as its source.

```rust
use system_suite::lifecycle::Failure;

fn open(path: &str) -> Result<std::fs::File, Failure> {
    if path.is_empty() {
        return Err(Failure::new("no path configured"));
    }
    Ok(std::fs::File::open(path)?)                 // io::Error converts
}
assert_eq!(open("").err().map(|f| f.to_string()), Some("no path configured".into()));
```

### `Deadline`

The point in time by which a `stop` (or a worker during shutdown) must
return. Always the OS monotonic clock, never the builder's clock, so a
test clock cannot stall a shutdown.

```rust
use std::time::Duration;
use system_suite::lifecycle::Deadline;

let deadline = Deadline::after(Duration::from_millis(50));
assert!(!deadline.is_past());
assert!(deadline.remaining() <= Duration::from_millis(50));
```

## Workers and timers

Background work runs on supervised worker threads and on one timer thread,
without an async runtime and with zero CPU when idle. The runtime is
`system.runtime()`.

### Workers

A worker is a named thread the kernel supervises. Its function gets a
`WorkerContext` and runs on the worker's thread:

- Returning `Ok(())` ends the worker (restarted only under
  `Restart::Always`, never once a stop was requested). Returning `Err` or
  panicking is a failure: logged, counted, and restarted per the policy
  after the backoff.
- A worker that fails more often than its budget allows (default 5 within
  60 s) is marked `Failed` and reported `Unhealthy`. If it is `critical`,
  the kernel requests a stop with `StopReason::WorkerFailed` (exit code
  70); otherwise it makes the system at most `Degraded`.
- With `heartbeat(every)`, a worker that has not called `cx.heartbeat()`
  for three periods is reported `Degraded`.
- At shutdown step 3 every worker is asked to stop (`should_stop()` turns
  true and `park` returns) and joined within the deadline.
- Under `panic = "abort"` a panic ends the process after the crash report.

```rust
use std::time::Duration;
use system_suite::runtime::{Backoff, Restart, Worker, WorkerState};
use system_suite::{Member, System};

let system = System::builder("api-doc-worker")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-worker"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .start()?;
let sweeper = Worker::new("sweeper")
    .restart(Restart::OnFailure)
    .backoff(Backoff::exponential(Duration::from_millis(10), Duration::from_secs(1)))
    .heartbeat(Duration::from_secs(10))
    .spawn(system.runtime(), |cx| {
        while !cx.should_stop() {
            cx.heartbeat();
            cx.park(Some(Duration::from_millis(5)));
        }
        Ok(())
    })?;
assert_eq!(sweeper.state(), WorkerState::Running);
let _ = system.shutdown();                          // stops and joins the worker
assert_eq!(sweeper.state(), WorkerState::Stopped);
# Ok::<(), system_suite::Error>(())
```

A consumer that parks while there is nothing to do, and is woken by the
producer through its `WorkerHandle`:

```rust
use std::sync::{Arc, atomic::{AtomicU32, Ordering}, mpsc};
use std::time::Duration;
use system_suite::runtime::Worker;
use system_suite::{Member, System};

let system = System::builder("api-doc-consumer")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-consumer"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .start()?;
let (jobs, queue) = mpsc::channel::<u32>();
let done = Arc::new(AtomicU32::new(0));
let total = Arc::clone(&done);
let consumer = Worker::new("jobs").critical(true).spawn(system.runtime(), move |cx| {
    while !cx.should_stop() {
        match queue.try_recv() {
            Ok(job) => { total.fetch_add(job, Ordering::Relaxed); }
            Err(_) => cx.park(None),                // zero CPU until woken or stopped
        }
    }
    Ok(())
})?;
jobs.send(5).ok();
consumer.wake();
for _ in 0..400 {
    if done.load(Ordering::Relaxed) == 5 { break; }
    std::thread::sleep(Duration::from_millis(5));
}
assert_eq!(done.load(Ordering::Relaxed), 5);
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

| `Worker` setting | Default |
|---|---|
| `restart(Restart)` | `OnFailure` (`Never`, `OnFailure`, `Always`) |
| `backoff(Backoff)` | exponential, 100 ms to 30 s, up to 25% jitter (`None`, `Fixed(d)`, `Backoff::exponential(min, max)`) |
| `budget(restarts, window)` | 5 within 60 s |
| `critical(on)` | `false` |
| `heartbeat(every)` | none expected |
| `stack_size(bytes)` | the platform's |

`spawn` fails with `Setup` (invalid or duplicate name, zero budget window),
`Stopping` (a stop was requested), or `Kernel` (the OS refused the
thread). `WorkerHandle` (cheap to clone) gives `name`, `state`,
`restarts`, `wake`, and `stop`; dropping it does not stop the worker.
`system.runtime().workers()` lists them for a status page.

```rust
use std::time::Duration;
use system_suite::runtime::Backoff;

let b = Backoff::exponential(Duration::from_millis(100), Duration::from_secs(30));
assert!(b.delay(0) <= Duration::from_millis(125));
assert!(b.delay(20) <= Duration::from_millis(37_500));
```

### Timers

`every(name, period, f)` and `after(name, delay, f)` (names as for workers:
a literal or a `String`) run callbacks on one
kernel thread, ordered by due time: no thread per timer, the thread starts
with the first timer, and with nothing due it waits with no timeout. A
periodic timer first fires one period from now; ticks missed while a
callback ran long are skipped, not replayed. Callbacks must return quickly
(well under a millisecond); heavier work wakes a worker. A callback that
panics cancels its timer and is reported `Degraded`. At shutdown step 3
every timer is cancelled.

```rust
use std::sync::{Arc, atomic::{AtomicU32, Ordering}};
use std::time::Duration;
use system_suite::{Member, System};

let system = System::builder("api-doc-timers")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-timers"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .start()?;
let ticks = Arc::new(AtomicU32::new(0));
let counter = Arc::clone(&ticks);
let tick = system.runtime().every("tick", Duration::from_millis(5), move || {
    counter.fetch_add(1, Ordering::Relaxed);
})?;
let warning = system.runtime().after("slow-start", Duration::from_secs(30), || {})?;
std::thread::sleep(Duration::from_millis(50));
assert!(tick.cancel());
assert!(warning.cancel());                    // it never fires
assert!(!warning.is_active());
assert!(ticks.load(Ordering::Relaxed) > 0);
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

Errors: `Setup` (invalid name, zero period, a period over 30 days),
`Stopping`, `Kernel`. Resolution is the OS's condition-variable timeout
(about 1 ms on Linux, up to the scheduler tick on Windows).

## Configuration

One file per application, one section per member, read through
config-lib (D-034) in its `conf` format. A later format or backend changes
the accepted file syntax, not these signatures (D-018); config-lib itself
is `system_suite::config::backend`.

```ini
# site.conf
[log]
level = "debug"

[kernel]
shutdown_deadline = "40s"

[http]
port = 8080
hosts = ["a.example", "b.example"]
```

In the `conf` format a value with spaces must be quoted: an unquoted
`reason = disk full` is read as the array `["disk", "full"]`.

### `Config`

An immutable snapshot of the configuration; `system.config()` returns the
one in force (lock-free: a pointer read and one reference count; a
per-request handler keeps a `Reader` from `config_snapshot()` instead,
one atomic load). Keys are dotted paths. `get` returns
`Ok(None)` for an absent key and `ConfigInvalid` (naming the key, the type
expected, and the type found, never the value) for a present key of the
wrong type; `require` also fails for an absent key; `get_or` takes a
default.

```rust
use std::{path::PathBuf, time::Duration};
use system_suite::config::Config;

let config = Config::from_text(
    "[http]\nport = 8080\nread_timeout = \"30s\"\nhosts = [\"a.example\", \"b.example\"]\n",
)?;
let port: u16 = config.get_or("http.port", 80)?;
let timeout: Duration = config.require("http.read_timeout")?;
let hosts: Vec<String> = config.require("http.hosts")?;
assert_eq!((port, timeout, hosts.len()), (8080, Duration::from_secs(30), 2));
assert_eq!(config.get::<PathBuf>("http.cert")?, None);
assert!(config.get::<bool>("http.port").is_err());   // wrong type
assert!(config.contains("http.port"));
assert_eq!(config.keys("http"), ["hosts", "port", "read_timeout"]);
assert_eq!(config.version(), 1);
assert!(config.source().is_none());                  // not read from a file
# Ok::<(), system_suite::Error>(())
```

`Config::from_text` parses `conf` text without the suite's section checks,
for tests and tools. `backend()` returns config-lib's own document for
anything the view does not cover.

### Value types

`FromConfig` is implemented for `bool`, `i8` to `i64`, `u8` to `u64`,
`usize`, `f64`, `String`, `PathBuf` (a leading `~` expanded through
path-lib), `Duration` (whole seconds, or a string with a unit: `"500ms"`,
`"30s"`, `"5m"`, `"2h"`), and `Vec<T>` of any of these. Integers are
range-checked into the target type. The trait is sealed, so the backend
can change without breaking callers; structured values are read field by
field, or through `backend()`.

### A reader per thread

A handler that wants the newest configuration on every request keeps a
`Reader` from `config_snapshot()`: about a nanosecond per `get` when
nothing changed, and the new version right after a reload.

```rust
use system_suite::{Member, System};

let system = System::builder("api-doc-reader")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-reader"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .config_str("[http]\nmax_body_kib = 512\n")
    .start()?;
let mut reader = system.config_snapshot().reader();
let limit: u32 = reader.get().get_or("http.max_body_kib", 1024)?;
assert_eq!(limit, 512);
system.reload()?;
assert_eq!(reader.get().version(), 2);
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

### Reload

`system.reload()` (also run by SIGHUP, or the Windows service
`PARAMCHANGE` control) reads the configuration again, validates it,
resolves secret references, and asks every subsystem to prepare its
change; only if all of them succeed is the new configuration published and
every change committed (D-019). It returns the new version. On any failure
nothing changes and the old configuration stays in force.

A subsystem prepares in `reload`: it validates and builds everything that
can fail, then returns a `Commit` whose closure only swaps the prepared
value in. The closure cannot fail and should not block; `Commit::none()`
(the default) has nothing to apply.

```rust
use system_suite::config::Config;
use system_suite::lifecycle::{Commit, Deadline, Failure, Link, Subsystem};
use system_suite::{Member, Snapshot, System};

struct Limits { live: Snapshot<u32> }

impl Subsystem for Limits {
    fn name(&self) -> &str { "limits" }
    fn start(&mut self, link: &Link) -> Result<(), Failure> {
        let _version = self.live.publish(link.foundation().config().get_or("http.max_body_kib", 1024)?);
        Ok(())
    }
    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> { Ok(()) }
    fn reload(&mut self, config: &Config) -> Result<Commit, Failure> {
        let limit: u32 = config.get_or("http.max_body_kib", 1024)?;    // may fail
        if limit == 0 {
            return Err(Failure::new("http.max_body_kib must be above zero"));   // refuses the reload
        }
        let live = self.live.clone();
        Ok(Commit::new(move || { let _version = live.publish(limit); }))
    }
}

let limits = Snapshot::new(0_u32);
let system = System::builder("api-doc-reload")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-reload"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .config_str("[http]\nmax_body_kib = 512\n")
    .register(Limits { live: limits.clone() })
    .start()?;
let mut reader = limits.reader();
assert_eq!(*reader.get(), 512);
assert_eq!(system.reload()?, 2);
assert_eq!(system.config().version(), 2);
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

| Error kind | When |
|---|---|
| `ConfigNotFound`, `ConfigInsecure`, `ConfigInvalid` | as at boot step 5 |
| `Secret`, `PlainSecret` | as at boot step 8 |
| `ReloadRefused` | a subsystem refused the change; the error names it |
| `Stopping` | a stop was already requested |

Settings that cannot change while running (paths, the guard, which members
are enabled, sink files) do not fail a reload; they are logged as "takes
effect at restart". `[kernel] shutdown_deadline` and `member_reserve` take
effect at once, unless the builder set them.

### Secrets

Configuration holds references, never secrets. A value of the form
`env:NAME`, `file:/path`, or `vault:name` is resolved at boot step 8 (and
again on reload) into key-vault; `Config::secret` returns a handle, and
the bytes are only visible inside `Secret::with`, in a buffer that is
wiped afterwards. `Secret`'s `Debug` prints the key's name and
`[redacted]`.

```rust
use system_suite::vault::RawKey;
use system_suite::{Member, System};

let system = System::builder("api-doc-secrets")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-secrets"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .config_str("[database]\npassword = \"vault:db-password\"\n")
    .on_vault(|vault| {
        // A key the application obtains itself: a hardware module, a key service.
        let _handle = vault.register("db-password", RawKey::new(b"hunter2".to_vec()))?;
        Ok(())
    })
    .start()?;
let password = system.config().secret("database.password")?;
assert_eq!(password.name(), "database.password");
assert_eq!(password.with(|bytes| bytes.len())?, 7);
assert!(!format!("{password:?}").contains("hunter2"));
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

| Reference | Resolved from | Refused when |
|---|---|---|
| `env:NAME` | the process environment, then the `.env` file | unset or empty |
| `file:/path` | the file's contents, one trailing newline removed | missing, over 64 KiB, or (Unix) group- or world-readable or not owned by the service user or root |
| `vault:name` | a key registered in `Builder::on_vault` | no such name |
| any, inside a list | | always: give each secret its own key |

A key whose last segment marks a secret (`password`, `passwd`, `secret`,
`token`, `api_key`, `private_key`, `credentials`, and the rest of log-io's
list) must hold a reference; a plain value is refused with `PlainSecret`
unless `[secrets] allow_plain = true`. Without the `secrets` feature, or
with `Member::Secrets` disabled, every reference is refused with `Secret`,
so a reference is never mistaken for a literal password. A `Secret` taken
from the configuration in force keeps its value through one reload
(D-064); the vault is wiped at shutdown step 7.

### Finding the file

The first that applies wins:

1. `Builder::config_file(path)` or `Builder::config_str(text)`;
2. the `<NAME>_CONFIG` variable in the process environment, then `<NAME>_CONFIG=` or `CONFIG=` in the `.env` file;
3. `<config dir>/<name>.conf` (with `config-toml`, `config-noml`, or
   `config-json`, also `<name>.toml`, `.noml`, or `.json`; finding two is
   `ConfigInvalid` naming both);
4. `./<name>.conf` (and the same alternatives) in the working directory,
   only when the process is not running as root or an administrator.

In cases 1 and 2 a missing file is an error; in cases 3 and 4 it means
every section takes its defaults. The owner and permissions are checked on
the opened file, so the file checked is the file read: on Unix a file that
is group- or world-writable, or owned by someone other than the service
user or root, is refused with `ConfigInsecure`, and there is no switch to
accept it (D-048). The `.env` file is checked the same way. A file over
1 MiB is refused; a parse error names the line and column and never quotes
the text.

### Sections

Every section is optional and every key has a secure default; the full
reference is [`CONFIGURATION.md`](./CONFIGURATION.md).

| Section | Keys (default) |
|---|---|
| `[guard]` | `lock` (`true`), `lock_dir` (the runtime directory), `allow_network_lock` (`false`), `umask` (`"0027"`), `open_files_target` (`65536`), `open_files_minimum` (`1024`) |
| `[log]` | `level` (`"info"`), `filter`, `format` (`"auto"`, `"human"`, `"json"`, `"logfmt"`), `stderr` (`true`), `file` (`true`), `max_bytes`, `rotation` (`"daily"`, `"hourly"`, `"never"`), `max_files` (`14`), `redact`, `redact_common` (`true`), `recent_lines` (`64`) |
| `[crash]` | `enabled` (`true`), `max_reports` (`8`) |
| `[metrics]` | `enabled` (`true`), `max_series` (`10000`) |
| `[kernel]` | `shutdown_deadline` (`"25s"`), `member_reserve` (`"2s"`), `notify` (`"auto"`), `reload_signal` (`true`), `dump_signal` (`true`), `memory_watch` (off), `memory_shed` (`90`) |
| `[ids]` | `worker` (`0`, 0 to 1023), `epoch` (id-forge's default) |
| `[audit]` | `enabled` (`true` when compiled in), `file` (`<data dir>/audit/<name>.audit`), `hash` (`"sha256"`), `sync` (`"every"` or `"flush"`), `repair_tail` (`false`) |
| `[secrets]` | `allow_plain` (`false`) |

An unknown key in one of these sections is refused with `ConfigInvalid`,
so a misspelt key cannot silently leave a setting at its default; other
sections belong to the application. There are no implicit environment
overrides of individual keys (D-021).

## Stopping, health, and phases

The kernel decides when the process stops and reports how it is doing.
`system.kernel()` is its control handle: cheap to clone, `Send + Sync`,
and every method is safe from any thread. Signals reach only the kernel
(D-006); the rest of the process asks it for a stop and waits for one.

```rust
use std::time::Duration;
use system_suite::lifecycle::{Phase, StopReason};
use system_suite::{Member, System};

let system = System::builder("api-doc-stop")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-stop"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .start()?;
let kernel = system.kernel().clone();
assert_eq!(kernel.phase(), Phase::Ready);
assert_eq!(kernel.wait_for_stop_timeout(Duration::from_millis(1)), None);
std::thread::spawn(move || {
    kernel.request_stop("disk almost full");       // exactly as a SIGTERM would
});
let reason = system.kernel().wait_for_stop();
assert_eq!(reason, StopReason::Requested { reason: "disk almost full".into() });
assert!(!system.kernel().request_stop("again"));   // the first reason is kept
let code = system.shutdown();
# let _ = code;
# Ok::<(), system_suite::Error>(())
```

| `Kernel` method | Does |
|---|---|
| `request_stop(reason)` | asks for a clean stop; `false` if one was already requested; never blocks |
| `request_supervisor_stop(reason)` | the same, with `StopReason::Supervisor` (a watchdog, an orchestrator) |
| `stop_reason()` | why a stop was requested, if one was |
| `wait_for_stop()` | parks with zero CPU until a stop is requested; several threads may wait |
| `wait_for_stop_timeout(d)` | the same, `None` after `d` |
| `phase()` | the lifecycle phase; one atomic load |
| `health()`, `status()` | see [Health](#health) |
| `urgent()` | see [The urgent line](#the-urgent-line) |
| `runtime()` | the same runtime as `system.runtime()` |

`System::run` is `kernel().wait_for_stop()` followed by `shutdown()`. A
framework with work between the two (handing its sockets to a supervisor
for a restart, say) calls them separately. `shutdown` runs the shutdown
order now, whether or not a stop was requested, and returns the exit code;
dropping a `System` without either runs it too. `boot_timeline()` lists
how long each boot step took.

### Phases and stop reasons

```text
 Booting ─▶ Ready ─▶ Draining ─▶ Stopping ─▶ Stopped
    │                    ▲
    └── boot failure ────┘ (reverse shutdown of what started, then Stopped)
```

`Phase` values compare in lifecycle order (`phase >= Phase::Draining`
means going down); a reload never changes the phase. `StopReason` is
`Signal { name }` (`"SIGTERM"`, `"CTRL_C"`, `"SERVICE_STOP"`, ...),
`Requested { reason }`, `WorkerFailed { name }`, or
`Supervisor { reason }`; `exit_code()` is success except for
`WorkerFailed` (70).

```rust
use system_suite::lifecycle::{Phase, StopReason};

assert!(Phase::Draining > Phase::Ready);
let reason = StopReason::Requested { reason: "admin".into() };
assert_eq!(reason.exit_code(), std::process::ExitCode::SUCCESS);
assert_eq!(reason.to_string(), "requested: admin");
```

### Health

`kernel().health()` collects every member, subsystem, worker, and the
timer thread into a `HealthReport` (it allocates: for health endpoints
and the dump signal). `kernel().status()` returns only the aggregate and
allocates nothing of its own. The aggregate is the worst component status,
except that a worker that is not `critical` makes the system at most
`Degraded`. Each reason is at most 256 bytes. The suite writes no
configuration value or secret into a reason: a worker's reason is the
message of its `Failure::new`, while an error converted with `?` (whose
text can quote a URL or a response) and a panic show only that the message
is in the log. A subsystem's reason is whatever its `health` returns.

```rust
use system_suite::kernel::{Health, Status};
use system_suite::lifecycle::{Deadline, Failure, Link, Subsystem};
use system_suite::{Member, System};

struct Db;

impl Subsystem for Db {
    fn name(&self) -> &str { "db" }
    fn start(&mut self, _link: &Link) -> Result<(), Failure> { Ok(()) }
    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> { Ok(()) }
    fn health(&self) -> Health { Health::degraded("replica lag 12 s") }
}

let system = System::builder("api-doc-health")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-health"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .register(Db)
    .start()?;
assert_eq!(system.kernel().status(), Status::Degraded);
let report = system.kernel().health();
assert!(report.ready());           // phase Ready and not Unhealthy: a readiness probe
assert!(report.live());            // the kernel is running: a liveness probe
let mut json = String::new();
report.write_json(&mut json)?;
assert!(json.starts_with(r#"{"status":"degraded","phase":"ready""#));
let text = report.to_string();     // or write!(out, "{report}") into any fmt::Write
assert!(text.contains("health.subsystem.db=degraded reason=\"replica lag 12 s\""));
# let _ = system.shutdown();
# Ok::<(), Box<dyn std::error::Error>>(())
```

`components()` lists each `ComponentHealth` (`kind`, `name`, `health`);
`Health::HEALTHY`, `Health::degraded(reason)`, and
`Health::unhealthy(reason)` build a component's health.

When `NOTIFY_SOCKET` is set (systemd, Linux) and `[kernel] notify` is
`"auto"`, the kernel sends `READY=1` at boot step 17, `STOPPING=1` at
shutdown step 1, `STATUS=<status>` when the aggregate changes, and, when
systemd asked for keep-alives (`WATCHDOG_USEC`), `WATCHDOG=1` at half that
interval while the status is not `Unhealthy`. On Windows the service
control manager gets `SERVICE_RUNNING`.

### Signals

| Signal | Unix | Windows | Kernel decision |
|---|---|---|---|
| terminate | SIGTERM | console close; as a service: stop, shutdown, preshutdown | stop |
| interrupt | SIGINT | Ctrl+C | stop |
| quit | SIGQUIT | Ctrl+Break | stop (a graceful one, as nginx treats SIGQUIT; `kill -ABRT` still gives a core) |
| hangup | SIGHUP, at a terminal with no service manager, or with `reload_signal` off | logoff, system shutdown | Unix: stop. Windows: ignored, so a user logging off never stops a service |
| reload | SIGHUP, under a service manager (or not at a terminal) | service `PARAMCHANGE` | `reload` |
| dump | SIGUSR1 | service user control code 128 | the health report, phase, urgent level, workers, build and instance info written to the log |
| a second stop signal during shutdown | | | recorded and ignored: the shutdown already has a deadline |

The signal intake is installed at boot step 7, only for a process-wide
system. `[kernel] reload_signal` and `dump_signal` turn those two off; with
`reload_signal` off SIGHUP stops the process, its default meaning.

## The urgent line

How the kernel (and, when present, the security package) tells the whole
process "shed load", "lock down", "drain", or "stop" now, without an event
loop in the way. Tier T0 is one cache-line-aligned level and epoch that
every worker reads at its checkpoints; tier T1 is each subsystem's
[`Mailbox`](#link-and-mailbox); tier T2 is the [event](#events) record,
the log, and the metrics.

| Level | Raised by | Meaning for a worker at its checkpoint |
|---|---|---|
| `Normal` | the start, or `shift` down | normal behaviour |
| `Shed` | `set` (a supervisor), or the memory watermark | refuse low-priority work |
| `Lockdown` | `set` (the security package) | strict policies only |
| `Draining` | the kernel, at a stop request | accept no new work; finish what is in flight |
| `Stopping` | the kernel, after the subsystems stopped or at the deadline | stop now |

`set` raises the level to `Shed` or `Lockdown` and returns the new epoch;
it never lowers it (D-084), so a `Lockdown` cannot be undone by a
`set(Normal)` from anywhere else. `shift(from, to, reason)` is the way
down: it moves the line only if it is at `from`. Setting the level the
line is already at changes nothing. `Draining` and `Stopping` belong to the
kernel, and once a stop is requested the line only moves forward. A change
made from inside a handler of an urgent change is refused, so handlers
cannot recurse. Each of these refusals is `UrgentRefused`. Reasons are cut
to 256 bytes; lifting a `Lockdown` is audited like raising one.

```rust
use system_suite::kernel::Level;
use system_suite::{Member, System};

let system = System::builder("api-doc-urgent")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-urgent"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .start()?;
let line = system.kernel().urgent();
let mut watch = line.watch();
assert_eq!(watch.changed(), None);                   // about a nanosecond
line.set(Level::Shed, "upstream is failing")?;
assert_eq!(watch.changed(), Some(Level::Shed));
assert!(line.set(Level::Normal, "too early").is_err());   // set never lowers
line.shift(Level::Shed, Level::Normal, "upstream recovered")?;
assert!(line.set(Level::Draining, "no").is_err());   // the kernel's, through a stop
assert_eq!(line.notice().reason, "upstream recovered");
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

In a worker loop, the check costs about a nanosecond when nothing changed:

```rust,no_run
# use system_suite::kernel::Level;
# fn serve_one() {}
# let system = system_suite::start!()?;
let mut watch = system.kernel().urgent().watch();
loop {
    if let Some(level) = watch.changed() {
        match level {
            Level::Draining | Level::Stopping => break,
            Level::Lockdown => { /* strict policies only */ }
            Level::Shed => { /* refuse low-priority work */ }
            _ => { /* back to normal */ }
        }
    }
    serve_one();
}
# Ok::<(), system_suite::Error>(())
```

A `Notice` carries `level`, `previous`, `epoch`, and `reason`; an epoch
more than one past the last one seen means changes were missed in between.
`UrgentLine::detached()` is a line no kernel drives, for tests and for
libraries that want the same checkpoint pattern outside a system.

The memory watermark (D-081) is off until `[kernel] memory_watch` sets how
often it reads memory. It raises the line from `Normal` to `Shed` when the
OS reports memory pressure or the share of memory used (inside a
container, of its limit) reaches `[kernel] memory_shed`, and clears it
after three readings in a row with no pressure and the share at least 5
points below the mark. It never lowers a `Lockdown`, never overrides a
stop, and leaves a `Shed` someone else set for them to clear.

## Events

Feature `registry`. The kernel's record of what happened, delivered
synchronously to callbacks registered on `system.events()`. Only the
kernel publishes events (D-085). `register` returns an `EventGuard`:
dropping it removes the callback, `keep()` keeps it until the shutdown,
which removes every callback at its step 4. A panicking callback is
isolated, logged, and counted (`panic_count()`); callbacks run on the
thread that caused the event and must return quickly.

```rust
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use system_suite::kernel::{Event, Level};
use system_suite::{Member, System};

let system = System::builder("api-doc-events")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-events"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .start()?;
let changes = Arc::new(AtomicUsize::new(0));
let seen = Arc::clone(&changes);
let guard = system.events().register(move |event: &Event| {
    if let Event::Urgent { notice } = event {
        assert_eq!(notice.level, Level::Shed);
        seen.fetch_add(1, Ordering::Relaxed);
    }
});
system.kernel().urgent().set(Level::Shed, "upstream is failing")?;
assert_eq!(changes.load(Ordering::Relaxed), 1);
drop(guard);                                           // removes the callback
system.events().register(|_event: &Event| {}).keep(); // kept until the shutdown
assert_eq!(system.events().handler_count(), 1);
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

| `Event` | When |
|---|---|
| `Phase { from, to }` | the lifecycle phase changed |
| `Urgent { notice }` | the urgent line changed |
| `Reloaded { version }` | a reload was published and committed |
| `ReloadRefused { by, reason }` | a reload was refused; `by` is the subsystem, or `"configuration"` |
| `WorkerRestarted { name, restarts }` | a worker failed and was restarted |
| `WorkerFailed { name }` | a worker exhausted its restart budget |
| `SubsystemStopped { name, took, met_deadline }` | a subsystem's `stop` returned or was abandoned |

The registry declares and records; urgent changes reach workers and
subsystems through the urgent line first (D-006). `Event` is
`#[non_exhaustive]`.

## Audit

Feature `audit`. The application's tamper-evident record of privileged
actions: who did what to what, and the outcome, without sensitive content.
The chain is opened at boot step 13, every existing record is verified,
and appending continues at the tail; a chain that fails verification stops
the boot with `AuditTampered`. With `[audit] sync = "every"` (the default)
a record is on disk when `record` returns. The kernel records its own
privileged actions here (a reload, a `Lockdown`, a stop request with its
reason). The chain's tail is also anchored outside the chain file
(`<state dir>/audit/<name>.anchor`), so a chain cut short or replaced
since the last sync is refused (D-072, D-078).

```rust
use system_suite::audit::Outcome;
use system_suite::{Member, System};

let root = std::env::temp_dir().join("system-suite-api-doc-audit");
let system = System::builder("api-doc-audit")
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .portable(&root)
    .start()?;
let audit = system.audit();
assert!(audit.is_enabled());
let id = audit.record("admin:42", "user.delete", "user:1009", Outcome::Success)?;
assert_eq!(audit.next_id().map(|next| next.as_u64()), Some(id.as_u64() + 1));
assert!(audit.path().is_some());
# let _ = system.shutdown();
# let _ = std::fs::remove_dir_all(&root);
# Ok::<(), system_suite::Error>(())
```

`sync()` flushes and syncs on demand; with `sync = "flush"` records are
synced every second and at shutdown. Text from a request is safe to pass:
each field is cut to 4 KiB and the field separator is replaced (D-090,
D-094).

## Frameworks

### Staged start with `Boot`

A framework's own boot phases happen between the foundation and
readiness, and each phase builds things that need the previous phase's
results (load modules, then connect the providers they declared, then open
the listener with the routes they registered). `Builder::boot()` runs boot
steps 1 to 15 and returns a `Boot`:

- `boot.foundation()` gives full typed access, the kernel among it.
- `boot.start(subsystem)` starts a subsystem at once, on the calling
  thread, and appends it to the stop order. On error the subsystem is not
  added and the `Boot` is still usable.
- `boot.ready()` starts the subsystems given to `Builder::register` (after
  everything `start` started), runs `on_ready`, announces readiness, and
  returns the `System`.
- Dropping a `Boot` stops everything it started, in reverse. A stop
  requested while a `Boot` exists makes the next `start` or `ready` fail
  with `Interrupted`.

```rust
use system_suite::lifecycle::{Deadline, Failure, Link, Subsystem};
use system_suite::{Member, System};

struct Listener { routes: usize }

impl Subsystem for Listener {
    fn name(&self) -> &str { "listener" }
    fn start(&mut self, _link: &Link) -> Result<(), Failure> { Ok(()) }
    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> { Ok(()) }
}

let mut boot = System::builder("api-doc-boot")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-boot"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .boot()?;
let routes = boot.foundation().config().keys("routes").len();   // the framework's first phase
boot.start(Listener { routes })?;                                 // the door, last
let system = boot.ready()?;
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

### `Foundation`

The typed handles of every enabled member, built once at boot: what a
framework receives, what every subsystem reaches through its `Link`, and
what `System` dereferences to. `Clone` is one reference-count increment;
it is `Send + Sync`. `system.foundation()` hands one to a thread.

| Method | Returns | Feature |
|---|---|---|
| `name()` | `&str` | any |
| `build_info()` | `Option<&'static sys::BuildInfo>` (`None` when the builder was not given it) | `sys` |
| `instance()` | `&'static sys::Instance`: id, start time, host, pid, user, uptime | `sys` |
| `guard_report()` | `&sys::process::GuardReport`: the lock, the umask, the open-file limit | `sys` |
| `paths()` | `&path::App` | `paths` |
| `env()` | `&env::Env` (prefix `<NAME>_`) | `env` |
| `config()` | `config::Config`, the one in force (lock-free: a pointer read and one reference count) | `config` |
| `config_snapshot()` | `&SnapshotView<config::Config>`, for a per-thread `Reader` | `config` |
| `log()` | `&log::Logger` | `log` |
| `metrics()` | `&metrics::MetricsCore` (metrics-lib's global one in a process-wide system) | `metrics` |
| `clock()` | `&dyn clock::Clock` | `time` |
| `ids()` | `&id::snowflake::Snowflake`, worker id from `[ids] worker` | `ids` |
| `audit()` | `&AuditLog` | `audit` |
| `events()` | `&kernel::Events` | `registry` |
| `vault()` | `&vault::KeyVault` | `secrets` |
| `kernel()` | `&kernel::Kernel` | `kernel` |
| `runtime()` | `&runtime::Runtime` | `kernel` |

```rust
use system_suite::{Member, System};

let system = System::builder("api-doc-foundation")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-foundation"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .config_str("[ids]\nworker = 12\n")
    .start()?;
let foundation = system.foundation().clone();
let worker = std::thread::spawn(move || {
    // Snowflake for entity ids: try_next_id, never next_id (it panics on clock skew).
    foundation.ids().try_next_id().is_ok()
});
assert_eq!(worker.join().ok(), Some(true));
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

Ids from id-forge are unique, not secret; tokens and nonces come from
`rand::tier3` (feature `rand`).

### `foundation()`

Feature `globals`. The running system's `Foundation`, for code that
cannot be handed one (a plugin loaded by name, a callback registered by a
third-party library). It is published when the system reaches `Ready` and
withdrawn when its shutdown starts; it is `None` before, after, and for a
system built with `process_wide(false)` or with `Member::Globals`
disabled. Prefer passing `Foundation` explicitly.

```rust,no_run
fn on_plugin_reload() {
    if let Some(foundation) = system_suite::foundation() {
        system_suite::log::warn!(foundation.log(), "plugin reload");
    }
}
# on_plugin_reload();
```

### `Snapshot`, `Reader`, and `SnapshotView`

A value read on every request and replaced whole on a reload: profiles,
compiled rules, budgets, and the configuration itself. `publish` swaps in
a complete new value in one step and returns the new version (versions
start at 1); readers holding the old value keep it until their next `get`.
Give each thread its own `Reader`: `get` is one atomic load and a compare
when nothing changed (measured at 0.3 ns), and a short lock plus an `Arc`
clone only right after a publish. `Snapshot` is `Send + Sync` and cheap to
clone; `Reader` is `Send`, not `Sync`.

```rust
use system_suite::Snapshot;

let limits = Snapshot::new(vec![100_u32, 200]);
let mut reader = limits.reader();
assert_eq!(reader.get()[0], 100);
let version = limits.publish(vec![150, 250]);   // a reload
assert_eq!(reader.get()[0], 150);
assert_eq!(reader.version(), version);

let view = limits.view();                       // load, version, reader; no publish
assert_eq!(view.load()[1], 250);
```

A `SnapshotView` hands out the value, the version, and readers, but cannot
publish: the configuration's snapshot is handed out this way, so only a
reload (validated, all or nothing) replaces it (D-074).

## Member modules

Each member crate is re-exported whole, so a project depends only on
`system-suite` and uses the members' own APIs directly.

| Module | Crate | Feature |
|---|---|---|
| `system_suite::sys` | sys-lib | `sys` |
| `system_suite::path` | path-lib | `paths` |
| `system_suite::env` | env-lib | `env` |
| `system_suite::config::backend` | config-lib | `config` |
| `system_suite::log` | log-io | `log` |
| `system_suite::metrics` | metrics-lib | `metrics` |
| `system_suite::signal` | signal-mod | `signals` |
| `system_suite::clock` | clock-lib | `time` |
| `system_suite::id` | id-forge | `ids` |
| `system_suite::audit` | audit-trail | `audit` |
| `system_suite::registry` | registry-io | `registry` |
| `system_suite::global` | global-lib | `globals` |
| `system_suite::rand` | mod-rand | `rand` |
| `system_suite::vault` | key-vault | `secrets` |

The members' macros work through the re-export (`system_suite::log::info!`,
`system_suite::metrics::counter!`, `system_suite::build_info!`).
`system_suite::log` and `system_suite::rand` have the same names as the
`log` and `rand` crates; a project that also uses those writes
`use system_suite::log as slog;` (D-015).

```rust
use system_suite::log as slog;
use system_suite::{Member, System};

let system = System::builder("api-doc-members")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("api-doc-members"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .start()?;
slog::info!(system.log(), "order placed", order = 7_u64, total_cents = 1299_u64);
let request_id = system_suite::id::ulid::Ulid::new();
let token = system_suite::rand::tier3::random_u64();
# let _ = (request_id, token);
# let _ = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

In a process-wide system the members' global slots are set too, so
`slog::info!("...")` (no handle) and metrics-lib's macros record into the
system's logger and registry.

## Conventions

### Tiers

| Tier | For | Items |
|---|---|---|
| 1 | every service and tool | `run!`, `start!`, `System::run`, the typed handles, `Error::report` |
| 2 | services that configure the boot or add their own work | `System::builder`, `Subsystem`, `Worker`, timers, `Config`, `reload`, `Kernel` |
| 3 | frameworks that embed the suite and libraries that plug into it | `Boot`, `Foundation`, `foundation()`, the urgent line, `Snapshot`, `Events`, `Error` details, member modules |

### Process-wide state

Some things are process-wide by nature: signal handlers, the panic hook,
the umask, the open-file limit, systemd notifications, and the members'
global slots (log-io's logger, metrics-lib's registry, global-lib's
store). A system built with the defaults installs all of them, because
that is what a service wants: one process, one system.

`Builder::process_wide(false)` builds a system that installs none of
them. Tests that start several systems in one process, and hosts that
embed a system next to their own signal handling, use it; every runnable
example on this page does. The single-instance lock is per name, not
process-wide, and stays on unless `single_instance(false)` or
`[guard] lock = false`.

### Threads

The suite starts no thread before the guard (boot step 6). With every
default member enabled, a running system owns:

| Thread | Started by | When it wakes |
|---|---|---|
| `signal-mod-signals` (Unix only) | signal-mod | a signal arrives |
| `system-suite-timer` | the runtime, with the first timer | a timer is due; parks with no timeout when none is |
| one per worker | `Worker::spawn` | as the worker decides |
| `system-suite-stop-<n>` | the kernel, only during shutdown | runs one subsystem's `stop` so the deadline can be enforced |

Idle CPU is zero: no thread polls. On Windows, console events arrive on a
thread the operating system creates for each event.

### Platforms

Linux, macOS, and Windows on x86_64 and aarch64, stable Rust and MSRV
1.89. Differences that reach this API:

| Behaviour | Linux | macOS | Windows |
|---|---|---|---|
| stop signals | SIGTERM, SIGINT, SIGQUIT; SIGHUP at a terminal with no service manager | same | Ctrl+C, Ctrl+Break, console close, service stop and shutdown (D-014); never logoff (D-099) |
| reload signal | SIGHUP | same | service `PARAMCHANGE`; otherwise `System::reload` only |
| dump signal | SIGUSR1 | same | service user control code 128 |
| umask | `0o027` by the guard | same | not applicable; the log and lock directories get an owner-only access control list |
| readiness | `sd_notify` when `NOTIFY_SOCKET` is set | none | the service control manager's `SERVICE_RUNNING` |

### Costs

Measured on the release build ([`PERFORMANCE.md`](./PERFORMANCE.md)):

| Operation | Linux | Windows |
|---|---|---|
| boot to ready, default members, a configuration file, log file on | 0.078 ms | 0.31 ms |
| stop to exit, nothing in flight | 0.16 ms | 0.20 ms |
| `Watch::changed`, nothing changed | 0.20 ns | 0.19 ns |
| `Reader::get`, unchanged | 0.35 ns | 0.30 ns |
| heap held by a booted system | 98 KiB | 99 KiB |

A handle access is a field read. A reload costs the file read and parse
plus each subsystem's preparation; `AuditLog::record` with
`sync = "every"` costs one `fdatasync`; `Kernel::health` allocates one
`Vec` and the reasons, `status` nothing of its own.

## Versioning

- A member's patch release needs no suite release.
- A member's minor release that changes something re-exported here is a
  suite minor release.
- A member's major release is a suite minor release before 1.0 and a
  suite major release after 1.0, unless the suite hides the change
  completely.
- The MSRV is the highest among the default members (1.89, sys-lib's);
  raising it is a major change after 1.0.
- 0.9.x changes the API only to fix it; 1.0 follows.

## Migrating from 0.8

0.9.0 simplified the API (D-096): each handle exists once, on
`Foundation`, reached from `System` through `Deref`; the stop, wait, and
health methods are on `system.kernel()`; `Foundation::build` is
`build_info`; `System::builder` is the one way to get a builder;
`Events::register` returns an `EventGuard`; a `HealthReport` prints
through `Display`; and five exit-code constants have plain names. Nothing about
the boot, the shutdown, the configuration file, or the defaults changed.
Every change, with its replacement, is in
[`MIGRATION-0.9.md`](./MIGRATION-0.9.md).

## Examples in the repository

| Example | Shows |
|---|---|
| `service` | a small service: boots, runs one supervised worker that watches the urgent line, stops in order on a signal; `SERVICE_FILL_MB` and a `[kernel] memory_watch` configuration exercise the memory watermark |

Run it with `cargo run --example service`.

[`Error`]: https://docs.rs/system-suite/latest/system_suite/struct.Error.html
