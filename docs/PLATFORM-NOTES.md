<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br><b>system-suite</b><br>
    <sub><sup>PLATFORM NOTES</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="./API.md" title="API reference"><b>API</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="./CONFIGURATION.md" title="Configuration"><b>CONFIGURATION</b></a>
        <span>&nbsp;│&nbsp;</span>
        <span>PLATFORM NOTES</span>
    </sup>
</div>
<br>

What differs between Linux, macOS, and Windows. CI runs every test on all three, on x86_64 and aarch64, on stable and the MSRV (1.89).

## Signals and stop requests

| Event | Linux and macOS | Windows |
|---|---|---|
| stop | SIGTERM, SIGINT, SIGQUIT | Ctrl+C, Ctrl+Break, console close, logoff, system shutdown; as a service: stop, shutdown, preshutdown |
| reload | SIGHUP | as a service: parameter change (`sc control <name> paramchange`) |
| dump | SIGUSR1 | as a service: user control code 128 (`sc control <name> 128`) |
| after shutdown step 8 | the OS default action | the OS default action |

On Windows, logoff and system shutdown arrive as signal-mod's `Hangup`, so there `Hangup` stops the process; on Unix SIGHUP reloads. A console process cannot receive Ctrl+C from another process group; Ctrl+Break reaches the same handler.

## Running as a service

| | Linux | macOS | Windows |
|---|---|---|---|
| Manager | systemd | launchd | the service control manager (SCM) |
| Readiness | `READY=1`, `STATUS=`, `STOPPING=1` when `NOTIFY_SOCKET` is set (`Type=notify`) | none (launchd has no readiness protocol) | `SERVICE_RUNNING` |
| Watchdog | `WATCHDOG=1` at half of `WatchdogSec=` while not unhealthy | none | none |
| Main thread | stays with `run` | stays with `run` | `run` hands it to the SCM for the life of the service (signal-mod `service`); a process not started by the SCM falls back to running in the foreground |
| Stop grace | `TimeoutStopSec=` (default 90 s) | `ExitTimeOut` (default 20 s) | the shutdown deadline is reported as the `STOP_PENDING` wait hint |

Keep `[kernel] shutdown_deadline` below the manager's grace period (the default 25 s is below systemd's and Kubernetes' defaults; set it below 20 s under launchd).

## Files and permissions

| | Unix (Linux, macOS) | Windows |
|---|---|---|
| Directories the suite creates | mode `0700` | a protected access control list naming only the user, `SYSTEM`, and `Administrators` (path-lib 1.1) |
| Log files, audit chain | mode `0600` | inherit the directory's protected list |
| Configuration and `.env` check | refused when group- or world-writable or owned by another user (not root) | not checked: the file's directory list governs it |
| Secret files (`file:`) | refused with any group or other bit, or another owner | not checked |
| umask | set by the guard (`[guard] umask`, process-wide mode) | not applicable |
| Open-file limit | raised towards `[guard] open_files_target` | not applicable |

The owner check needs the `sys` feature (it reads the current user).

## Memory and secrets

key-vault locks its pages in memory (`mlock` / `VirtualLock`) as best effort: when the limit (`RLIMIT_MEMLOCK`, the Windows working-set size) is reached, the pages stay unlocked and nothing fails. Core dumps exclude them on Linux and FreeBSD, and Windows error reporting excludes them; macOS has no equivalent. Every buffer is wiped when dropped.

## Randomness

With `rand` or `secrets`, boot step 2 reads from the OS random source once: `getrandom(2)` on Linux, `getentropy` on macOS, `BCryptGenRandom` on Windows. A sandbox that blocks it stops the boot with `Entropy` (exit 69). Early in a Linux boot the call can wait for the kernel's entropy pool.

## Time

Deadlines, timers, and backoff use the OS monotonic clock everywhere. Timer resolution is the OS condition-variable timeout: about 1 ms on Linux, up to the scheduler tick (often 15.6 ms) on Windows. The audit chain uses the wall clock, made strictly increasing so an NTP step back does not stop it.

## Memory pressure

The urgent line's memory watermark (raising it to `Shed` under pressure) needs sys-lib 0.4's `mem::pressure()`; it is not wired in 0.5 (see the roadmap).
