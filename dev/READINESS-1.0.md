# system-suite: 1.0 readiness

For James, before any 1.0 release candidate. Kept current with each pre-1.0 pass (`dev/ROADMAP.md` v0.9.x, D-080). Last updated 2026-11-03, after the first pass; 0.7.1 is the latest release, and the 0.8 API changes are on `main`, unreleased.

## Done

- Every planned feature (`dev/PLAN.md` section 8), wired and tested on Linux, macOS, and Windows; every member alone and in combination builds and passes (the feature matrix).
- Threats T1 to T9 each have a passing attack test; `docs/SECURITY.md` states every defence's cost and the known limits.
- Two security reviews of the whole suite (0.7, and the first pre-1.0 pass), every finding fixed.
- Every performance target met and measured (`docs/PERFORMANCE.md`): boot about 0.1 ms, stop about 0.2 ms, under 100 KiB of heap, no idle CPU, a one-nanosecond urgent-line check.
- An 8-hour soak (251,225 cycles) with no growth in memory, descriptors, or threads.
- A real service runs in CI in a container limited to 256 MiB and 1 CPU, as a user with no entry in the user database.
- The API reviewed against the planned consumers; the changes that would break it after 1.0 are made (D-084 to D-086).

## Still open, and why

| Item | Why it is open | What closes it |
|---|---|---|
| Two real consumers (0.8's exit) | nothing outside this repository depends on the suite yet | WASP's boot or a Hive DB start-up adopting it |
| The API review against real use | the review so far was against the consumers' specification | the same consumers; changes then, not after 1.0 |
| Additive API a consumer may need (D-087) | designing it without a caller risks the wrong shape | a consumer asking: `Kernel::reload`, a dump from the kernel, a per-request deadline, error hooks on workers, subsystem restart, a lock beside the data directory |
| `config-json`, `metrics-json` | James held `serde_json` until first-party crates cover it; `jsonx-lib` 0.5.0 now does | a JSON backend on `jsonx-lib` in config-lib and metrics-lib (member releases) |
| `config-xml`, `config-validation`, `metrics-otlp` | they need XML or regex, and no first-party crate covers those yet | first-party XML and regex crates |
| `metrics-system` | `sysinfo` was not accepted | sys-lib 0.4's live readings could stand in; a decision for James |
| Wake-up readings on Windows and macOS | sys-lib has no per-process wake-up reading | a sys-lib request (not filed) |
| The pre-1.0 pass on signal-mod 2.1.0 and audit-trail 1.1.0 | not run yet | the same review and tuning pass in those repositories |
| Documented limits (`docs/SECURITY.md`) | each is the OS's own exposure, or needs a member or a std API | kept unless one of those changes |

## What 1.0 would promise

- The public API in `docs/API.md` does not break within 1.x: new methods, variants (the enums are non-exhaustive), and fields of non-exhaustive structs only.
- The boot and shutdown order of `docs/BOOT-ORDER.md`, the configuration keys of `docs/CONFIGURATION.md`, and the exit codes.
- The defences of `docs/SECURITY.md`, each with an attack test in CI.
- MSRV 1.89, raised only in a minor release with a changelog note.
