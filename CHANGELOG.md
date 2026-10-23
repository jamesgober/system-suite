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

The design lock. No functional code yet.

### Added

- `docs/API.md`: the complete public API (items SUI-01 to SUI-30) with
  signatures, errors, exit codes, costs, examples, and the version each item
  arrives in: `start!`, `System`, `Builder`, `Boot`, `Foundation`, `Error`
  and `ErrorKind`, the `kernel` module (lifecycle, urgent line, subsystems,
  workers, timers, health, events), the configuration view, `AuditLog`,
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

[Unreleased]: https://github.com/jamesgober/system-suite/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/jamesgober/system-suite/releases/tag/v0.1.0
