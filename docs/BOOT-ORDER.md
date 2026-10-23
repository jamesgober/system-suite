<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br><b>system-suite</b><br>
    <sub><sup>BOOT AND SHUTDOWN ORDER</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="./API.md" title="API Reference"><b>API</b></a>
        <span>&nbsp;│&nbsp;</span>
        <span>BOOT ORDER</span>
        <span>&nbsp;│&nbsp;</span>
        <a href="../dev/PLAN.md" title="Plan"><b>PLAN</b></a>
    </sup>
</div>
<br>

The order in which `system-suite` starts its members and the application's subsystems, the order in which it stops them, and the reason for every position. The order is law: it is tested, and changing it is a recorded decision in the decision log of [`../dev/PLAN.md`](../dev/PLAN.md) (this version: D-012).

Written at the `0.2.0` design lock. Steps arrive with the members that perform them (column **Since**); a step whose member is compiled out is skipped, and the remaining steps keep their order.

<sub>Copyright &copy; 2026 <strong>James Gober</strong>.</sub>

## Table of contents

- [Rules the order follows](#rules-the-order-follows)
- [Boot](#boot)
  - [The steps](#the-steps)
  - [Why each step is where it is](#why-each-step-is-where-it-is)
  - [Failure during boot](#failure-during-boot)
  - [A stop signal during boot](#a-stop-signal-during-boot)
- [Shutdown](#shutdown)
  - [The steps](#the-steps-1)
  - [Why each step is where it is](#why-each-step-is-where-it-is-1)
  - [Deadlines](#deadlines)
- [Reload](#reload)
- [Crash](#crash)
- [Changes from the 0.1.0 plan](#changes-from-the-010-plan)

## Rules the order follows

1. **Nothing is created before the guard.** No directory, file, or thread exists until the single-instance lock is held and the umask is set. A second copy, or a copy with an insecure lock directory, stops having changed nothing on disk.
2. **Read before write.** The steps before the guard only read: they resolve paths, read the environment file, and parse and validate the configuration. The guard's own settings come from that configuration, so it has to be read first.
3. **Logging as early as possible, and before anything that can fail quietly.** Every step after logging opens reports its outcome and duration in the log. Failures before logging opens go to standard error through [`Error::report`](./API.md#errorreport).
4. **Crash capture right after logging.** From that point a panic leaves a crash report and the last log lines on disk.
5. **The kernel last among the foundation, the application after it.** The application's subsystems start only when every member they might use is up, and the door (listeners, anything outward-facing) is the application's last subsystem.
6. **Shutdown is the reverse, and the kernel stops last.** The door closes first; logging is the last member to stop, so every other step is recorded.
7. **Signals are hooked early and acted on at step boundaries.** A stop signal during boot is never lost and never interrupts a step halfway.

## Boot

### The steps

| # | Step (`Step`) | Member | What happens | On failure | Since |
|---|---|---|---|---|---|
| 1 | `BuildInfo` | sys-lib | the build info compiled into the application by `start!` is recorded; it becomes the first log line at step 9 | cannot fail | 0.3 |
| 2 | `Instance` | sys-lib, mod-rand | the instance (id, start time, host, pid, user) is created; with `rand` or `secrets` enabled, one read from the OS random source proves it works | `Entropy`: exit | 0.3 |
| 3 | `Paths` | path-lib | the application's directories are resolved (scope from the builder or `<NAME>_PATH_SCOPE`, overrides from `<NAME>_<KIND>_DIR`); nothing is created | `Paths`: exit | 0.3 |
| 4 | `Environment` | env-lib | the `.env` file in the configuration directory is read, if present; the process environment is never changed | `Environment`: exit | 0.3 |
| 5 | `Configuration` | the configuration backend | the file is found (explicit path, `<NAME>_CONFIG`, the configuration directory, the working directory), its owner and permissions checked, parsed, and every member section validated; secret references are parsed, not resolved | `ConfigNotFound`, `ConfigInsecure`, `ConfigInvalid`: exit | 0.4 |
| 6 | `Guard` | sys-lib | single-instance lock in the runtime directory (or the configured one), pid file, umask `0o027`, open-file limit raised | `AlreadyRunning`, `LockDirInsecure`, `Limits`: exit, nothing created | 0.3 |
| 7 | `Signals` | signal-mod | the kernel's signal intake is installed: stop, reload, and dump signals are recorded from here on and acted on at the next step boundary | `Signals`: exit | 0.3 |
| 8 | `Secrets` | key-vault | secret references in the configuration (`vault:`, `env:`, `file:`) are resolved into the vault; plain-text secrets are refused unless allowed | `Secret`, `PlainSecret`: exit | 0.5 |
| 9 | `Directories` | path-lib | the directories the enabled members write to (logs, data, state, runtime) are created owner-only | `Paths`: exit | 0.3 |
| 10 | `Logging` | log-io | sinks opened; the first lines are the build info, the instance, guard warnings, and the duration of steps 1 to 9 | `Logging`: exit | 0.3 |
| 11 | `CrashCapture` | this crate, log-io | the panic hook is installed with the recent-lines ring | cannot fail | 0.3 |
| 12 | `Metrics` | metrics-lib | the registry is created (and installed process-wide unless `process_wide(false)`) | `Metrics`: unwind, exit | 0.4 |
| 13 | `Audit` | audit-trail | the chain is opened, the existing records verified, and the chain resumed at its tail | `Audit`, `AuditTampered`: unwind, exit | 0.5 |
| 14 | `Registry` | registry-io | the kernel event registry is created | cannot fail | 0.5 |
| 15 | `Kernel` | this crate | the urgent line, the timer thread, worker supervision, and health aggregation start; the lifecycle moves to `Booting` complete | `Kernel`: unwind, exit | 0.3 |
| 16 | `Subsystems` | the application | each registered subsystem is started in ascending `Position`, ties in registration order | `Subsystem`: unwind, exit | 0.4 |
| 17 | `Ready` | this crate | `on_ready` callbacks run; `READY=1` to systemd when `NOTIFY_SOCKET` is set; the phase becomes `Ready`; `start` returns | `Subsystem` (an `on_ready` failure): unwind, exit | 0.3 |

### Why each step is where it is

1. **Build info first.** It costs nothing (it is compiled in) and it is the one fact every later report needs: which build failed.
2. **Instance second.** The instance id names the crash files, the log lines, and the metrics; the start time anchors the boot timeline. The entropy check is here because a sandbox that blocks the OS random source would otherwise surface later as a confusing failure in the vault or in a token generator; failing at step 2 names the cause.
3. **Paths before everything that reads a file.** The configuration directory holds the `.env` file and the configuration file; the runtime directory holds the lock. Resolving paths reads only the environment and creates nothing.
4. **Environment before configuration.** `<NAME>_CONFIG` may come from the `.env` file, and so may the values that `env:` secret references name. The suite reads the environment through env-lib and never writes the process environment (env-lib's `export` is `unsafe` and the suite forbids `unsafe`).
5. **Configuration before the guard.** The guard's settings (lock directory, umask, open-file targets) are part of the configuration. Reading the file before the lock is safe: it changes nothing, and a second copy that reads it and then fails at the guard has done no harm. The permission check (the 0.2 defence idea in [`../dev/PLAN.md`](../dev/PLAN.md) section 18) runs before the file is parsed.
6. **Guard before anything is created.** The lock decides whether this copy may run at all, and the umask must be in force before the first directory or file exists (threat T3). It must also come before the first thread, because the umask is process-wide and a thread started earlier could create a file with the old mask; sys-lib warns `ThreadsAlreadyRunning` if that rule is broken.
7. **Signals right after the guard.** This is the first step that starts a thread (signal-mod's intake thread on Unix), so it comes after the umask. From here a SIGTERM during a slow step (a large audit chain, a slow subsystem) is recorded and honoured at the next boundary (threat T8), instead of killing the process mid-write.
8. **Secrets after the guard and the signal intake.** Building the vault may start a thread and locks memory pages; neither should happen in a copy that is about to be refused by the guard. Secrets are resolved before any member that might need one is opened.
9. **Directories after the guard.** They are created owner-only under the umask the guard set. Only the directories an enabled member writes to are created; a CLI tool without logging files creates nothing.
10. **Logging before every member that can fail after boot.** From here every step records its outcome. The steps before it are recorded too: their durations are held in memory and written as the first lines.
11. **Crash capture right after logging.** The report includes the last log lines, so the ring must exist, and the earlier the hook is in place the fewer panics go unrecorded.
12. **Metrics before audit and the kernel.** The kernel records its own metrics (boot timeline, worker restarts, urgent-line changes) and the audit member counts its writes.
13. **Audit after logging and metrics.** Verifying an existing chain can take time and can fail with tampering; both are logged and counted. It comes before the kernel so that the kernel's own privileged actions (a reload, a lockdown) can be audited.
14. **Registry before the kernel.** The kernel publishes its events (phase changes, urgent-line changes, reloads, restarts) to the registry from the first one.
15. **Kernel last among the foundation.** It supervises the workers and timers that members and the application start, and it owns the shutdown; it needs every member it reports on to exist already.
16. **Subsystems after the foundation.** A subsystem can use every enabled member from its `start`, through the `Link` it is given.
17. **Ready last.** `READY=1` and the return of `start` promise that the whole process is serving; nothing outward-facing exists before this (threat T4). The application's door is its last subsystem, or its first action after `start` returns.

### Failure during boot

A step that fails stops the boot. Everything that started is stopped in exactly the shutdown order below, starting from the last step that completed, and `start` returns an `Error` that names the step, the member, and what to do. Nothing started stays running; the lock is released when the guard is dropped as the error is returned. Every step has a failure-injection test (threat T4).

### A stop signal during boot

A stop signal received after step 7 is recorded. At the next step boundary the boot stops and unwinds as for a failure, and `start` returns an `Error` of kind `Interrupted` whose exit code is `0`: the operator asked for the stop, so it is not a failure. A second stop signal during the unwind does not skip any step; the unwind already has its deadline.

## Shutdown

### The steps

A stop is requested by a stop signal, `System::request_stop`, a critical worker that exhausted its restarts, or a supervisor. The kernel then runs:

| # | Step | What happens | Since |
|---|---|---|---|
| 1 | `Draining` | the phase becomes `Draining`; the urgent line is raised to `Draining` (every worker sees it at its next checkpoint, every subsystem's mailbox is poked); `STOPPING=1` to systemd | 0.3 |
| 2 | subsystems | stopped one at a time in the reverse of their start order, each given the remaining shutdown deadline minus the member reserve | 0.4 |
| 3 | `Stopping` | the urgent line is raised to `Stopping`; workers are asked to stop and joined; the timer thread stops | 0.4 |
| 4 | registry | event handlers are cleared and waited for, so none runs while members close | 0.5 |
| 5 | audit | the chain is flushed and synced to disk | 0.5 |
| 6 | metrics | a final export, when an exporter is configured; process-wide registries stay readable | 0.4 |
| 7 | secrets | the vault is dropped, which wipes every secret | 0.5 |
| 8 | signals | the signal intake is uninstalled; a further signal gets the operating system's default action | 0.3 |
| 9 | logging | the shutdown report (one line per subsystem with its stop time and whether it met the deadline), the exit code, then flush and close | 0.3 |
| 10 | `Stopped` | the phase becomes `Stopped`; `run` returns the exit code; the guard is dropped as `System` is dropped, releasing the lock | 0.3 |

### Why each step is where it is

1. **Draining first, as one atomic write.** Every worker and subsystem learns of the stop at the same moment, from a single cache line (urgent line T0), without waiting for an event loop. `STOPPING=1` goes out at the start, not the end, so systemd knows the service is going down while it drains.
2. **Subsystems in reverse.** The application's door was started last, so it closes first: no new request can arrive while anything behind it is stopping. Providers (databases, caches) were started before the services that use them and stop after them, so in-flight work can still write.
3. **Workers and timers after the subsystems.** Subsystems may rely on kernel workers (a flush thread, a sweeper) while they stop.
4. **Registry before the members.** A handler must not run while the member it writes to is closing.
5. **Audit before logging.** The audit chain is the record that must not be lost; it is synced while logging can still report a failure to sync.
6. **Metrics after audit.** A final export is useful but not essential; it never delays the audit sync.
7. **Secrets after every member that might use one.** Dropping the vault wipes the secrets as soon as nothing needs them.
8. **Signals near the end.** Until here a second stop signal is recorded (and ignored: the shutdown is already running). After the intake is removed, a further signal gets the default action, which is what an operator who sends a third SIGTERM expects.
9. **Logging last.** Every other step reports into it, including the exit code and any abandoned subsystem.
10. **The kernel last.** It ran every step above; `Stopped` is the final state, and the lock is released only after the logs are flushed, so a replacement copy never starts while this one is still writing.

### Deadlines

- The **shutdown deadline** (default 25 s, `Builder::shutdown_deadline` or `[kernel] shutdown_deadline`) covers steps 1 to 9. The default is below Kubernetes' default 30 s grace period and systemd's 90 s `TimeoutStopSec`, so the suite's own deadline fires first and reports which part was late instead of being killed silently.
- The **member reserve** (default 2 s, `[kernel] member_reserve`) is kept back from the subsystems for steps 4 to 9, so a slow subsystem cannot take the time the audit sync and the log flush need.
- A subsystem that does not return from `stop` by its deadline is **abandoned**: its thread cannot be killed safely, so it is left running, reported in the log and in the shutdown report, the next subsystem is stopped, and the exit code becomes `79` (`exit::DEADLINE`) (threat T6).
- A worker that does not exit by the time the deadline is reached is abandoned the same way.

## Reload

A reload (SIGHUP on Unix, `System::reload`, or a Windows service `PARAMCHANGE` once signal-mod supports it) does not change the lifecycle phase:

1. the configuration file is read and validated as in boot step 5; secret references are resolved as in step 8;
2. every member that supports a live change (log level and filters, metrics limits, kernel deadlines) and every subsystem prepares its change and returns a commit; nothing is applied yet;
3. if every preparation succeeded, the new configuration snapshot is published and every commit runs; otherwise nothing changes, the old configuration stays, and the refusal is logged (threat T7).

Settings that cannot change while running (paths, the guard, sink files) are reported as "takes effect at restart" and do not fail the reload.

## Crash

A panic anywhere in the process, once step 11 completed, runs the crash hook:

1. the panic is logged at error level with its location and thread;
2. logging, the audit chain, and metrics are flushed (each with a short bound, so a stuck sink cannot hold the process);
3. the crash report `crash-<instance>-<unix time>.txt` is written owner-only to the log directory: build info, instance info, the panic message and location, the thread, and the last log lines (already redacted by the logger);
4. the previous panic hook runs, and the process ends as the application's profile decides (`panic = "abort"` in release).

A panic in a supervised worker under `panic = "unwind"` also produces a report, and the worker is restarted per its policy. At most `[crash] max_reports` reports (default 8) are written per process, so a panic loop cannot fill the disk.

## Changes from the 0.1.0 plan

The 0.1.0 plan had the guard at step 3, before configuration, and the kernel hooking signals at step 12. Reading the members' APIs at the design lock showed that the guard's settings live in the configuration and that signal-mod starts a thread, so the order above (decision D-012) reads paths, the environment, and the configuration first, takes the guard before anything is created, and hooks signals right after the guard. `STOPPING=1` moved from the end of the shutdown to its start, as systemd expects.
