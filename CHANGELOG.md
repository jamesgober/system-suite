<h1 align="center">
    <img width="90px" height="auto" src="https://raw.githubusercontent.com/jamesgober/jamesgober/main/media/icons/hexagon-3.svg" alt="Triple Hexagon">
    <br><b>CHANGELOG</b>
</h1>
<p>
  All notable changes to <code>system-suite</code> will be documented in this file. The format is based on <a href="https://keepachangelog.com/en/1.1.0/">Keep a Changelog</a>,
  and this project adheres to <a href="https://semver.org/spec/v2.0.0.html/">Semantic Versioning</a>.
</p>

---

## [Unreleased]

---

## [0.7.0] - 2026-10-29

Hardening: every threat has a passing attack test, and a security review
of the whole suite is closed.

### Added

- The audit chain's tail is anchored outside the chain file
  (`<state dir>/audit/<name>.anchor`, owner-only, rewritten after every
  sync): a chain cut short or replaced is refused with `AuditTampered`.
- `SnapshotView` and `Snapshot::view`: a read-only handle (load, version,
  readers, no publish).
- `docs/SECURITY.md`: every threat, its defence, its cost, its attack
  test, and the known limits.
- Attack tests: T3 for the audit chain and its anchor, T5 for a
  panicking worker, T8 with a real SIGTERM while a subsystem starts.
- An 8-hour soak test (`tests/soak.rs`, ignored by default) of start,
  reload, and stop cycles.

### Changed

- `config_snapshot()` returns `&SnapshotView<Config>` instead of
  `&Snapshot<Config>`: only a reload, validated, replaces the
  configuration. Code that called `.reader()`, `.load()`, or `.version()`
  is unchanged; code that called `.publish()` no longer compiles, as
  intended.
- The `.env` file is read by the suite and checked on the opened file
  (regular file only, at most 1 MiB).
- Reloads and dumps run on their own threads, not the signal thread.

### Fixed

- A panicking signal handler no longer ends the signal intake.
- A reload cancels when a stop arrives while subsystems prepare; a
  panicking commit no longer leaves the rest uncommitted.
- A reload cannot set a zero or over-long shutdown deadline; an
  `[ids] epoch` in the future is refused.
- A secret reference inside a list, or under a disabled secrets member,
  is refused instead of read as a literal.
- Secrets from a reload that finished after the shutdown wiped the vault
  are removed at once.
- The configuration and `.env` files must be regular files (a FIFO no
  longer blocks the boot); an existing audit chain another user can
  change is refused.
- The crash hook syncs the audit chain; the crash report's write error is
  logged within a bound.
- `status()` no longer holds the subsystem list while health checks run;
  a stopped worker's name can be used again; a timer thread that misses
  the deadline is reported as late.

## [0.6.0] - 2026-10-29

Optimisation: every performance target measured and published, all met.

### Added

- `benches/boot.rs`: the boot benchmark harness (standard library only):
  boot to ready, stop to exit, the urgent-line check, an unchanged
  snapshot read, the heap a booted system holds, and on Linux idle CPU
  and wake-ups. Baselines per OS in `benches/baseline/`, and a regression
  gate (over 105% and over a noise floor).
- `System::boot_timeline()`: every boot step that ran, with its duration
  (steps 16 and 17 included).
- `docs/PERFORMANCE.md`.
- CI runs the benchmark on every push.

### Moved

- The memory watermark ships in the release after sys-lib 0.4.0.

## [0.5.0] - 2026-10-28

Feature complete: every member wired, secrets resolved into a vault, a
tamper-evident audit chain, kernel events, and Windows service control.

### Added

- Audit (feature `audit`, audit-trail 1.1): `AuditLog` (`record`, `sync`,
  `path`, `next_id`, `is_enabled`), `Foundation::audit`, boot step 13
  (the chain is verified before the first new record; `AuditTampered`
  stops the boot), `[audit]` (`enabled`, `file`, `hash`, `sync`,
  `repair_tail`), the `audit-blake3` pass-through. The kernel records
  reloads, lockdowns, and stops. Shutdown step 5 syncs the chain.
- Secrets (feature `secrets`, key-vault 1.1): boot step 8 resolves `env:`,
  `file:`, and `vault:` references into a page-locked vault;
  `Config::secret` and `config::Secret` (`name`, `with`);
  `Builder::on_vault`; `Foundation::vault`; `[secrets] allow_plain`. A
  plain value under a key that marks a secret is refused (`PlainSecret`).
  Reloads resolve again without changing what the version in force
  reads; shutdown step 7 wipes the values.
- Events (feature `registry`, registry-io 1.1): `kernel::Event` (phase,
  urgent, reload, worker, subsystem stop), `Foundation::events`, boot step
  14, shutdown step 4.
- `system_suite::foundation()` (feature `globals`, global-lib 1.0).
- Randomness (feature `rand`, mod-rand 1.0): boot step 2 checks the OS
  random source (`Entropy`); backoff jitter from mod-rand.
- Windows service control (signal-mod 2.1): `System::run` hands the main
  thread to the service control manager when started by it; stop,
  shutdown, and preshutdown stop; parameter change reloads; control code
  128 dumps.
- `ErrorKind::{Entropy, PlainSecret, Audit, AuditTampered, Disabled}`.
- Member modules `audit`, `registry`, `global`, `rand`, `vault`.
- `docs/ARCHITECTURE.md`, `docs/CONFIGURATION.md`, `docs/PLATFORM-NOTES.md`.
- CI: every member feature alone, built, linted, and tested on Linux,
  macOS, and Windows.

### Changed

- `audit`, `registry`, and `secrets` imply `kernel`; `secrets` implies
  `config` and `rand`.
- With `secrets`, a secret reference is resolved at step 8 instead of
  being refused at step 5.

### Moved

- The memory watermark (urgent line to `Shed` under memory pressure) to
  0.6: it needs sys-lib 0.4.0, not yet published.

## [0.4.0] - 2026-10-28

Extended: configuration, metrics, the application's subsystems, and the
kernel's services.

### Added

- Configuration (feature `config`, config-lib 1.1 in its `conf` format):
  `config::Config` (`from_text`, `get`, `require`, `get_or`, `contains`,
  `keys`, `source`, `version`, `backend`), the sealed `FromConfig` for
  booleans, integers (range-checked), `f64`, `String`, `PathBuf`,
  `Duration` (`30` or `"500ms"`, `"30s"`, `"5m"`, `"2h"`), and lists;
  `Builder::config_file` and `config_str`; `Foundation::config` and
  `config_snapshot`. Boot step 5 finds the file (builder, `<NAME>_CONFIG`
  from the process environment or the `.env` file, the configuration
  directory, the working directory when not elevated), checks it, parses
  it, and validates the suite's sections (`[guard]`, `[log]`, `[crash]`,
  `[metrics]`, `[kernel]`, `[ids]`), refusing unknown keys.
- Reload: `System::reload` and SIGHUP read the configuration again,
  validate it, let every subsystem prepare its change, and only then
  publish it and commit every change; otherwise nothing changes (T7).
- `Snapshot` and `Reader`: a value replaced whole and read for about a
  nanosecond per call when unchanged.
- Subsystems: `lifecycle::{Subsystem, Position, Link, Mailbox, Failure,
  Commit}`, `Builder::register` and `on_ready`; boot step 16 starts them by
  position, the shutdown stops them in reverse, each within the deadline.
- `Builder::boot` and `Boot` (`foundation`, `kernel`, `start`, `ready`):
  the staged start for frameworks.
- The urgent line: `kernel::{UrgentLine, Level, Watch, Notice}`; every
  change reaches each subsystem's mailbox and is logged and counted.
- `runtime::{Runtime, Worker, WorkerContext, WorkerHandle, WorkerState,
  Restart, Backoff, TimerHandle}`: supervised worker threads with restart
  policies, jittered backoff, restart budgets, criticality, and heartbeats;
  periodic and one-shot timers on one thread that sleeps when nothing is
  due.
- Health: `kernel::{Health, Status, HealthReport, Component,
  ComponentHealth}`, `System::health` and `status` (`ready`, `live`, JSON
  and key-value output); the dump signal writes it.
- Metrics (feature `metrics`, metrics-lib 2): `Foundation::metrics`, the
  `metrics` module, and the kernel's own metrics (`system_suite_*`);
  pass-throughs `metrics-histogram`, `metrics-meter`, `metrics-statsd`,
  `metrics-http`.
- Opt-in interop and formats (accepted by James, D-056): `log-bridge`
  (records from the `log` facade go to the suite's logger), `log-tracing`
  (`log::compat::TracingLayer`), `metrics-tracing`, `config-toml` and
  `config-noml` (`<name>.toml`, `<name>.noml`; the format follows the
  extension, and two candidate files are refused).
- `loom` model tests of the urgent line and worker heartbeats, in their own
  CI job (`cfg(loom)`, development only).
- `System::request_stop`, `System::runtime`, `Foundation::runtime`,
  `Builder::member_reserve`.
- systemd: `STATUS=` lines and the `WATCHDOG=1` keep-alive when
  `WATCHDOG_USEC` is set, sent only while the system is not unhealthy.
- `ErrorKind::{ConfigNotFound, ConfigInsecure, ConfigInvalid, Secret,
  Metrics, Subsystem, Stopping, ReloadRefused, UrgentRefused}`.
- Tests: the subsystem-level T6 test (a `stop` that never returns is
  abandoned, the exit code is 79, and the next subsystem is still
  stopped); T7 (a broken file, an unknown key, a refusing subsystem, and a
  secret reference each change nothing); failure injection at step 5 and
  for the `.env` file; SIGHUP reload and the dump signal in a child
  process; workers, timers, the urgent line, and `Boot`.

### Changed

- The `.env` file is refused when another user can change it, as the
  configuration file is (it can name the configuration file).
- The configuration file's owner and mode are checked on the opened file,
  so the file checked is the file read (D-049).
- The logger, the guard, crash capture, and the Snowflake worker id take
  their settings from the configuration; the builder still wins.
- `Builder::single_instance`, `crash_capture`, and `shutdown_deadline` set
  an override; when not called, the configuration decides.
- SIGHUP reloads the configuration (it logged "nothing to reload" in 0.3).
- Timers are a min-heap of due times rather than a timing wheel (D-046).

### Removed

- `[config] allow_insecure_file` from the design: there is no switch to
  accept a configuration file another user can change (D-048).
- `[metrics] process` (it needs `sysinfo`, which was not accepted), and
  `config-json`, `config-xml`, `config-validation`, `metrics-json`,
  `metrics-otlp`, which wait on first-party crates (D-056).

---

## [0.3.0] - 2026-10-23

The first boot path: a service boots with one call, logs, parks with zero
CPU, and stops in order on a signal.

### Added

- `start!()` and `start!(name)`, `System::builder` and `Builder` (build info,
  directory scope, portable root, `.env` file, test clock, `disable`,
  `process_wide`, `crash_capture`, `single_instance`, `shutdown_deadline`),
  `System::run`, `wait_for_stop`, `shutdown`, and the typed handles
  (`build`, `instance`, `guard_report`, `paths`, `env`, `log`, `clock`,
  `ids`, `kernel`) on `System` and `Foundation`.
- The boot order of `docs/BOOT-ORDER.md`, steps 1 to 4, 6, 7, 9 to 11, 15,
  and 17: a failing step stops the boot and unwinds what started; a stop
  signal during boot is acted on at the next step boundary
  (`ErrorKind::Interrupted`, exit code 0).
- The shutdown order with a deadline (default 25 s); a step that does not
  finish is abandoned and the exit code is 79.
- `kernel::Kernel` (stop requests and waiting) and `lifecycle::{Phase,
  StopReason, Deadline}`.
- Signals through signal-mod's runtime-free back-end: SIGTERM, SIGINT, and
  SIGQUIT stop; SIGHUP and SIGUSR1 are received without stopping; on
  Windows Ctrl+C, Ctrl+Break, console close, logoff, and system shutdown
  stop.
- Crash capture: the panic is logged and flushed (with a bound), and an
  owner-only crash report is written to the log directory with the build,
  the instance, the panic, and the last 64 log lines (redacted by the
  logger); at most 8 reports per process.
- systemd readiness: `READY=1` when ready and `STOPPING=1` when the
  shutdown starts, when `NOTIFY_SOCKET` is set.
- `Error`, `ErrorKind`, `Error::report`, and the `exit` codes
  (`sysexits.h` plus 79).
- Member modules: `sys`, `path`, `env`, `log`, `signal`, `clock`, `id`, and
  `build_info!` / `build::emit`; the `time-serde` pass-through feature.
- Owner-only log and lock directories on every OS (path-lib 1.1), owner-only
  log files (log-io 2.1), the umask set to `0o027` by the guard.
- Tests: every boot step that can fail is made to fail with a real trigger
  in a child process; the attack tests for T1 (a second copy), T3 (file and
  directory permissions, including Windows access control lists), T5 and T9
  (a panic leaves a report without secrets), T8 (a stop at every step
  boundary), and the deadline runner behind T6.

### Changed

- `Foundation::build()` returns an `Option` (`None` when the builder was not
  given the build info, D-038).
- `Builder::crash_capture` belongs to the `kernel` feature (D-039).

---

## [0.2.0] - 2026-10-22

The design lock. No functional code yet.

### Added

- `docs/API.md`: the complete public API (items SUI-01 to SUI-30) with
  signatures, errors, exit codes, costs, examples, and the version each item
  arrives in: `start!`, `System`, `Builder`, `Boot`, `Foundation`, `Error`
  and `ErrorKind`, the `kernel`, `lifecycle`, and `runtime` modules (urgent
  line, health, events, subsystems, workers, timers), the configuration view,
  `AuditLog`,
  `Snapshot`, and the member modules.
- `docs/BOOT-ORDER.md`: the boot and shutdown orders with the reason for
  every position, the deadlines, reload, and crash handling.

### Changed

- The Tier 1 entry point is the macro `start!()`, so the application's build
  info is recorded (`dev/PLAN.md`, D-008).
- The boot order reads paths, the environment, and the configuration before
  the process guard, and takes the guard before anything is created or any
  thread starts (D-012).
- `dev/PLAN.md`: member facts corrected after reading every member's public
  API, member requests filed, open decisions OD-4 to OD-6 decided and OD-1 to
  OD-3 answered by James (the kernel is built here, config-lib is the
  configuration backend, audit is opt-in), decisions D-008 to D-036, a new
  defence idea.
- Dependency policy: the zero-third-party-dependency rule is replaced by a
  reviewed dependency register (`dev/PLAN.md` section 12.4, D-032);
  signal-mod's signal-hook, libc, and windows-sys, config-lib's
  dependencies, and audit-trail's sha2 are accepted. No async runtime, as
  before.
- `docs/API.md`: member features passed through as
  `<suite feature>-<member feature>` features (D-036).
- The kernel's parts are three modules: `kernel` (the controller),
  `lifecycle` (phases and the `Subsystem` contract), and `runtime` (workers
  and timers, through a `Runtime` handle) (D-037).
- `dev/ROADMAP.md`: a review by James before the API freeze in 0.9.
- `dev/ROADMAP.md`: member releases recorded as blockers for 0.3 and 0.5;
  secret-reference resolution moved to 0.5 with key-vault (D-031).

---

## [0.1.0] - 2026-10-22

The scaffold and the plan. No functional code yet.

### Added

- Crate manifest (edition 2024, MSRV 1.89, no dependencies; one feature per
  member, declared empty until each member is wired), `Apache-2.0 OR MIT`
  license, README, REPS, `deny.toml`, `clippy.toml`, `rustfmt.toml`,
  `.editorconfig`, `.gitattributes`.
- `VERSION` constant and the crate-root lint set: `unsafe_code` forbidden,
  `missing_docs`, and `unwrap`/`expect`/`panic` in library code denied.
- CI on Linux, macOS, and Windows, each on x86_64 and aarch64, on stable and
  1.89; documentation built with default and all features; `cargo package`;
  `cargo audit` and `cargo deny`.
- `dev/PLAN.md` (purpose, scope, members and their status, requirements, the
  kernel, the boot and shutdown orders, API outline with item ids, threats T1
  to T9, architecture, dependencies, performance targets, testing, open
  decisions, decision log D-001 to D-007), `dev/DIRECTIVES.md`, and
  `dev/ROADMAP.md`.

[Unreleased]: https://github.com/jamesgober/system-suite/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/jamesgober/system-suite/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/jamesgober/system-suite/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/jamesgober/system-suite/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/jamesgober/system-suite/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/jamesgober/system-suite/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jamesgober/system-suite/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jamesgober/system-suite/releases/tag/v0.1.0
