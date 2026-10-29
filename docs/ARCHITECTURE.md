<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br><b>system-suite</b><br>
    <sub><sup>ARCHITECTURE</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="./API.md" title="API reference"><b>API</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="./BOOT-ORDER.md" title="Boot and shutdown order"><b>BOOT ORDER</b></a>
        <span>&nbsp;│&nbsp;</span>
        <span>ARCHITECTURE</span>
    </sup>
</div>
<br>

How the suite is put together: what each part does, who owns what, which threads exist, and where process-wide state lives. The public API is in [`API.md`](./API.md); the order things start and stop in, with the reasons, is in [`BOOT-ORDER.md`](./BOOT-ORDER.md).

## Shape

```text
 Builder ──start()/boot()──▶ boot driver (steps 1..17, boot.rs)
                               │  each step adds to Parts
                               ▼
                             Parts ──(steps 1..15)──▶ Foundation   (Arc: typed handles)
                               │                         │
                               │                         ├──▶ Link (per subsystem)
                               │                         └──▶ foundation() (globals)
                               ▼
                             System  ──run()/shutdown()──▶ teardown (shutdown steps 1..10)
```

- **Members** are separate published crates behind their own features. The suite re-exports each whole (`system_suite::log`, `::config`, `::audit`, ...) and never copies member code: if a member lacks something, the member grows it in its own release.
- **`Parts`** holds everything that started, in boot order. The same `teardown` stops a running system and unwinds a failed boot, so there is one shutdown path.
- **`Foundation`** is the typed handle set built once after step 15: one `Arc`, a field read per accessor. Subsystems, frameworks, and threads receive it; `System` delegates to it.
- **`System`** owns `Parts` and runs the teardown exactly once (`run`, `shutdown`, or `Drop`).

## The kernel

The kernel (`kernel` feature) is the controller, split into three modules (D-037):

| Module | Holds |
|---|---|
| `kernel` | `Kernel` (phase, stop requests, health), the urgent line, health aggregation, kernel metrics, events (with `registry`), crash capture, the signal intake, systemd notification |
| `lifecycle` | `Phase`, `StopReason`, `Deadline`, and the subsystem contract: `Subsystem`, `Position`, `Link`, `Mailbox`, `Failure`, `Commit` |
| `runtime` | supervised `Worker`s and timers on one thread |

Signals reach only the kernel (D-006): the intake thread turns each into a stop request, a reload, or a dump. A stop request is an atomic store plus a condition-variable wake; everything else learns of it from the urgent line.

### The urgent line

| Tier | What | Cost [est.] |
|---|---|---|
| T0 | one cache-line-aligned word: level and epoch | about 1 ns per checkpoint read |
| T1 | each subsystem's `Mailbox`, posted and woken directly | a mutex and a condition-variable notify per subsystem |
| T2 | the observer: log line, metric, audit record (lockdown), `Event` | as each sink costs |

A change writes the level, then bumps the epoch (acquire-release), so a reader that sees an epoch sees that change's level or a later one; once the level reaches `Draining` it only moves forward. Both properties are model-checked with loom (D-058).

## Configuration

`config::Config` is an immutable snapshot (an `Arc` around config-lib's parsed document), published through `Snapshot<Config>`. Readers on hot paths keep a `Reader`, which costs one atomic load and a compare when nothing changed. A reload builds a complete new snapshot, lets every subsystem prepare its change, and only then publishes and commits; secret references for the new version are stored in the vault under per-version names, so a refused reload never changes what the old version reads.

## Secrets

With `secrets`, step 8 builds a key-vault vault (normalization off, so the original bytes come back), runs the `on_vault` callbacks, and resolves every `env:`, `file:`, and `vault:` reference into it. `Config::secret` returns a `Secret` handle; the bytes are only visible inside `Secret::with`, in a buffer wiped on return. The configuration in force and the one before it keep their values; older versions are removed (wiped). Shutdown step 7 removes the rest.

## Audit

With `audit`, step 13 opens the chain with audit-trail's verified open: every record is checked before the first new one is appended, a broken chain stops the boot, and a record cut short by a crash is set aside only when `[audit] repair_tail` allows it. The kernel records its own privileged actions (reloads, lockdowns, stops). The clock is the wall clock made strictly increasing, so an NTP step back cannot stop the chain.

## Threads

| Thread | Started by | Lives |
|---|---|---|
| `system-suite-signals` | step 7 (process-wide mode) | until shutdown step 8 |
| `system-suite-timer` | the first timer (`runtime.every` / `after`; the audit flush and systemd keep-alive use timers) | until shutdown step 3 |
| `worker-<name>` | each `Worker::spawn` | until it returns and is not restarted, or shutdown step 3 |
| `system-suite-stop-<step>` | each bounded shutdown step | the step; abandoned at the deadline |
| the service manager's thread | Windows service mode (`run`) | the life of the service |

An idle system wakes for nothing: the signal thread blocks on the OS, the timer thread waits with no timeout when nothing is due, workers park.

## Process-wide state

Installed only when `Builder::process_wide(true)` (the default), and removed at shutdown where the OS allows:

| State | Where | Removed |
|---|---|---|
| signal handlers | step 7 | step 8 |
| panic hook | step 11 | step 10 (the previous hook is put back) |
| umask, open-file limit | step 6 | not restored (they belong to the process) |
| log-io's global logger, the `log` crate's logger (`log-bridge`) | step 10 | set once per process |
| metrics-lib's global registry | step 12 | set once per process |
| `system_suite::foundation()` | step 17 | shutdown step 1 |
| member internals: key-vault's page table, registry-io's hazard records, global-lib's store | first use | never (by those crates' design) |

With `process_wide(false)` several systems run side by side in one process (tests, embedding hosts); none of the rows above except member internals is touched.

## Errors

One `Error` type carries the kind (`ErrorKind`), the step, the member, the subsystem, a message, and what to do, with the source error chained. Every kind maps to a `sysexits.h` exit code (plus 79 for a missed deadline), so a supervisor can tell a configuration error from a crash. Messages name keys, files, and positions, never configuration values or secrets.

## Features

```text
kernel ◀── signals, registry, audit, secrets
paths  ◀── env, config, audit
config ◀── secrets, config-toml, config-noml
rand   ◀── secrets
log    ◀── log-bridge, log-tracing
metrics◀── metrics-histogram, -meter, -statsd, -http, -tracing
```

Every combination builds; CI checks each member alone, default, all, and none.
