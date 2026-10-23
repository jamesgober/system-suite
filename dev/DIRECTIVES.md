# system-suite - Engineering Directives

> Engineering standards and the definition of done for this project. Read alongside `REPS.md` (root, authoritative), [`PLAN.md`](./PLAN.md) (design and decisions), and [`ROADMAP.md`](./ROADMAP.md) (current phase). If anything here conflicts with `REPS.md`, `REPS.md` wins, except where `PLAN.md` records an approved exception (D-005: no async runtime).

---

## 0. Philosophy

This package is built and maintained to a production standard and treated as a flagship piece of work. Plan the full path, then build one verified step at a time. "Good enough" is treated as a defect. It is a **suite**: a complete foundation, so a project using it needs no other foundation library. Every service in the portfolio starts here, so its boot must be correct, fast, and fail closed, and its shutdown must never lose data.

---

## 1. What this is

system-suite bundles the foundation libraries (system facts and boot kit, paths, environment, configuration, logging, metrics, audit, registry, signals, time, ids, globals, randomness, secrets), each behind its own feature, and adds the kernel that boots them in order, supervises background work, reports health and readiness, captures crashes, and shuts down in reverse. See `PLAN.md` sections 1 to 4.

---

## 2. Engineering law (non-negotiable)

- **Wiring, not features.** A capability a member should have is added to that member, in its own pull request, never re-implemented here (D-002).
- **Judged, not counted.** Every third-party runtime dependency is a reviewed choice recorded in the dependency register (`PLAN.md` section 12.4, D-032): maintainer, maintenance, `unsafe` record, transitive tree, MSRV, licence. A necessity (OS bindings, audited cryptography, async-signal-safe handling) is taken from its best maintained source rather than rewritten. Members are added by published version, never as a `path` dependency to a sibling repository. No async runtime (D-005).
- **Fail closed.** A boot step that fails stops the boot and unwinds what started; nothing is left half-running. A configuration that cannot be validated is refused.
- **Order is law.** The boot and shutdown orders in `PLAN.md` section 7 are tested and documented; changing them is a recorded decision.
- **Typed handles.** Members are resolved once at boot; nothing is looked up by name at runtime.
- **Zero CPU when idle.** No polling, no timers firing for nothing, no busy waiting.
- **Unsafe.** `#![forbid(unsafe_code)]`. Operating-system calls live in the members.
- **Secrets.** Configuration holds references, not secrets. Nothing secret reaches logs, errors, `Debug`, or crash reports.
- **Cross-platform.** Linux, macOS, and Windows on x86_64 and aarch64, stable and MSRV 1.89, in CI.
- **Error handling.** Every fallible path returns `Result`; `Error` is `#[non_exhaustive]`, names the failing step and member, and says what to do.
- **Production-ready.** `#![deny(missing_docs)]`; no `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!`/`unreachable!`/`dbg!`/printing in library code; every public item has rustdoc with a runnable example; every claim in the docs maps to a test.
- **Performance.** No claim without criterion numbers; the targets in `PLAN.md` section 13 are measured and published.

---

## 3. Definition of done

1. Compiles clean on Linux, macOS, and Windows (x86_64 and aarch64), stable and MSRV 1.89.
2. `fmt`, `clippy -D warnings` (all targets; default, all, and no default features), tests in the same three modes, `cargo doc -D warnings` with default and all features, and `cargo package` in a fresh target directory: clean on Windows and in WSL Ubuntu, stable and 1.89.
3. `cargo audit` and `cargo deny check` pass; `cargo tree` shows no third-party runtime dependency that is missing from the dependency register (`PLAN.md` section 12.4).
4. A Tier-1 API exists and headlines the docs (`system_suite::start`, `System::run`).
5. Every boot and shutdown step has a failure-injection test; every threat in `PLAN.md` section 10 has an attack test that must fail to get through.
6. Hot-path changes carry benchmarks; no regression over 5%.
7. `README.md`, `docs/API.md`, and `CHANGELOG.md` updated in the same change; the matching `docs/release/vX.Y.Z.md` written before the tag; every version string bumped.
8. `PLAN.md` decision log updated for every design decision made during the work; at least one new defence idea per version in `PLAN.md` section 18.
9. v1.0.0 only after the full audit (API, docs, tests, benchmarks, security) is recorded in `docs/STABILITY-1.0.md` and James approves it.

---

## 4. Project-specific invariants

- `start` returns `Ok` only after every enabled member and registered subsystem started; on `Err`, nothing started remains running.
- The kernel is the last to stop; the application's own subsystems stop before any member.
- Signals reach only the kernel.
- A reload either swaps the whole configuration or changes nothing.
- No file the suite creates is readable by other users.
- The crash report never contains a secret.
- Every member can be compiled out without breaking the build.

---

## 5. Writing style

- Plain, specific, human prose in every shipping file. No marketing voice.
- Banned in shipping files: "comprehensive", "robust", "seamless", "leverage", the em-dash character (U+2014) in prose, and emoji.
- Every security measure states its cost (latency, memory); estimates are labelled as estimates, measurements as measured.
- Release notes in the past tense.
- Commits: imperative present tense, descriptive subject line.
