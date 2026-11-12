# system-suite: 1.0 readiness

For James, before any 1.0 release candidate. Kept current with each pre-1.0 pass (`dev/ROADMAP.md` v0.9.x, D-080). Last updated 2026-11-11, with the simplicity pass; 0.9.0 is the latest release.

## Done

- Every planned feature (`dev/PLAN.md` section 8), wired and tested on Linux, macOS, and Windows; every member alone and in combination builds and passes (the feature matrix).
- Threats T1 to T9 each have a passing attack test; `docs/SECURITY.md` states every defence's cost and the known limits.
- Three security reviews of the whole suite (0.7, and two pre-1.0 passes, the second independent: D-089, D-094), every finding fixed or recorded as a limit with its reason; the same review of signal-mod and audit-trail, the members released for the suite (D-090), and of config-lib (1.3.0 and 1.3.1: strict TOML and NOML, and four ways the INI and CONF parsers misread a file). After the second pass the Linux gate found the stop's audit record spending the subsystems' part of the deadline on a slow disk; fixed in 0.8.1 (D-095). signal-mod 2.1.1's service status ordering passed its CI job that installs a real Windows service and drives it with `sc.exe`.
- JSON configuration files and JSON and OTLP metrics on James's own jsonx-lib, without serde_json (D-090). Writing a metrics snapshot takes 0.65 to 0.77 of serde_json's time with the same bytes (jsonx-lib 0.5.1, metrics-lib 2.1.1, D-092).
- Every performance target met and measured (`docs/PERFORMANCE.md`): boot about 0.1 ms, stop about 0.2 ms, under 100 KiB of heap, no idle CPU, a one-nanosecond urgent-line check.
- An 8-hour soak (251,225 cycles) with no growth in memory, descriptors, or threads.
- A real service runs in CI in a container limited to 256 MiB and 1 CPU, as a user with no entry in the user database.
- The API reviewed against the planned consumers; the changes that would break it after 1.0 are made (D-084 to D-086).
- The simplicity pass (0.9.0, D-096): a service is `run!()`, `start!()?` works in `main`, each handle exists once, one way to get a builder and to listen, plain names; the README, `docs/API.md`, and the migration note run as doctests.

## Still open, and why

| Item | Why it is open | What closes it |
|---|---|---|
| Two real consumers (0.8's exit) | one gateway boots on the suite; a database does not yet | a database start-up adopting it, and both run on every listed environment |
| The API review against real use | the review so far was against the consumers' specification | the same consumers; changes then, not after 1.0 |
| Additive API a consumer may need (D-087) | designing it without a caller risks the wrong shape | a consumer asking: `Kernel::reload`, a dump from the kernel, a per-request deadline, error hooks on workers, subsystem restart, a lock beside the data directory |
| `config-xml`, `config-validation` | they need XML or regex, and no first-party crate covers those yet | first-party XML and regex crates |
| jsonx-lib is pre-1.0 | config-lib pins it exactly (`=0.5.1`), so every jsonx-lib release needs a config-lib release, and metrics-lib may not require a newer one than that pin (D-092) | jsonx-lib 1.0 |
| `metrics-system` | `sysinfo` was not accepted | sys-lib 0.4's live readings could stand in; a decision for James |
| Wake-up readings on Windows and macOS | sys-lib has no per-process wake-up reading | a sys-lib request (not filed) |
| audit-trail's open questions | a keyed hasher (new API), an opt-out for old chains holding `0x1f`, length-prefixed fields (a format change for 2.0) | James's call; none blocks 1.0 of the suite |
| Documented limits (`docs/SECURITY.md`) | each is the OS's own exposure, or needs a member or a std API | kept unless one of those changes |

## What 1.0 would promise

- The public API in `docs/API.md` does not break within 1.x: new methods, variants (the enums are non-exhaustive), and fields of non-exhaustive structs only.
- The boot and shutdown order of `docs/BOOT-ORDER.md`, the configuration keys of `docs/CONFIGURATION.md`, and the exit codes.
- The defences of `docs/SECURITY.md`, each with an attack test in CI.
- MSRV 1.89, raised only in a minor release with a changelog note.
