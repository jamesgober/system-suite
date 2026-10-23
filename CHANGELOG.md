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

[Unreleased]: https://github.com/jamesgober/system-suite/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/jamesgober/system-suite/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/jamesgober/system-suite/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jamesgober/system-suite/releases/tag/v0.1.0
