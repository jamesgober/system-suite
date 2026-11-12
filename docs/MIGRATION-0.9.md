<h1 align="center">
    <b>system-suite</b><br>
    <sub><sup>MIGRATING FROM 0.8 TO 0.9</sup></sub>
</h1>
<div align="center">
    <sup>
        <a href="../README.md" title="Project Home"><b>HOME</b></a>
        <span>&nbsp;│&nbsp;</span>
        <a href="./API.md" title="API Reference"><b>API</b></a>
        <span>&nbsp;│&nbsp;</span>
        <span>MIGRATION</span>
    </sup>
</div>
<br>

0.9.0 simplified the API before 1.0: each handle exists once, each task has one way to do it, and the names are plain words. Nothing about the boot, the shutdown, the configuration file, the defaults, or the behaviour changed. Most services need only the version bump; the compiler names every other change, and each one is a rename or a shorter path from the table below.

## The version

```toml
[dependencies]
system-suite = "0.9"
```

## What changed

| 0.8 | 0.9 | Why |
|---|---|---|
| `match start!() { Ok(s) => s, Err(e) => return e.report() }` then `s.run()` | `system_suite::run!()` (the old form still works) | one line for the common case |
| `system.request_stop(reason)` | `system.kernel().request_stop(reason)` | it was a copy of the kernel's |
| `system.wait_for_stop()` | `system.kernel().wait_for_stop()` | same |
| `system.health()`, `system.status()`, `system.phase()` | `system.kernel().health()`, `.status()`, `.phase()` | same |
| `system.build()`, `foundation.build()` | `system.build_info()`, `foundation.build_info()` | `build` read as a verb |
| `Builder::new(name)` | `System::builder(name)` | one way to get a builder |
| `events.register(f)` returning an `EventId`, then `events.unregister(id)` | `events.register(f)` returns an `EventGuard`; drop it to remove the callback | one way to listen |
| `events.register_guard(f)` | `events.register(f)` | same |
| a callback for the whole run | `events.register(f).keep()` | keeps it until the shutdown |
| `report.write_kv(&mut out)` | `write!(out, "{report}")` or `report.to_string()` | it repeated `Display` |
| `link.kernel()`, `link.urgent()` | `link.foundation().kernel()`, `.kernel().urgent()` | copies of the foundation's |
| `boot.kernel()` | `boot.foundation().kernel()` | same |
| `kernel.runtime()` | `foundation.runtime()` (`system.runtime()`) | same |
| `StopReason::exit_code()` returning `u8` | returns `ExitCode`, as `Error::exit_code` does | one type for one name |
| `request_supervisor_stop(reason: impl Into<String>)`, `StopReason::Supervisor { reason: String }` | `impl Into<Cow<'static, str>>` and a `Cow`, as `request_stop` | one text type for one kind of value |
| `runtime.every(name: &'static str, ...)`, `after` | the name is `impl Into<Cow<'static, str>>`, as `Worker::new` (a literal still works) | same |
| `exit::DATAERR` | `exit::DATA_ERROR` | plain words; the value (65) is the same |
| `exit::CANTCREAT` | `exit::CANNOT_CREATE` | 73 |
| `exit::IOERR` | `exit::IO_ERROR` | 74 |
| `exit::TEMPFAIL` | `exit::TEMPORARY_FAILURE` | 75 |
| `exit::NOPERM` | `exit::NO_PERMISSION` | 77 |

Every other accessor on `System` (`log`, `config`, `paths`, `metrics`, `kernel`, `runtime`, and the rest) still works: `System` now dereferences to its `Foundation`, which holds each of them once. Code that names them as paths (`System::log`) uses `Foundation::log`.

`Error`'s `{:?}` now prints the same block as `Error::report` (what failed, at which step, and what to do), so an error returned from `main` with `?` reads as a report. `{:#?}` prints the fields.

### Behaviour that changed

| 0.8 | 0.9 | Why |
|---|---|---|
| Windows read any configuration, `.env`, or secret file | a file another account owns or can change is refused (`ConfigInsecure`), and a secret file another account can read (`Secret`) | any local user could plant one under `%ProgramData%`; fix the file with `icacls` as the error says |
| a Windows service stopped when any user logged off | the logoff is ignored | it stopped every service on the machine |
| SIGHUP always reloaded on Unix | at a terminal with no service manager, or with `reload_signal = false`, it stops | the terminal went away; a reload left a detached process |
| a worker's health reason quoted the error from `?` | it says `error (the message is in the log)`; the log has the text | error text can quote URLs, queries, and responses |
| key-vault 1.1 was accepted | key-vault 1.2 is required (`cargo update -p key-vault`) | a nested `Secret::with` panicked on 1.1 |

A worker that wants a reason in its health report returns `Err(Failure::new("upstream refused the connection"))` instead of the converted error.

## After

A service in one line:

```rust,no_run
fn main() -> std::process::ExitCode {
    system_suite::run!()
}
```

Stopping from another thread, and reading the health:

```rust
use system_suite::{Member, System};

let system = System::builder("migration-stop")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("migration-stop"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .start()?;
let kernel = system.kernel();
assert!(!kernel.health().components().is_empty());
assert!(kernel.request_stop("maintenance"));
let _reason = kernel.wait_for_stop();
let _code = system.shutdown();
# Ok::<(), system_suite::Error>(())
```

Listening to the kernel's events:

```rust
# #[cfg(feature = "registry")] {
use system_suite::kernel::Event;
use system_suite::{Member, System};

let system = System::builder("migration-events")
#     .portable(std::env::temp_dir().join("system-suite-doc").join("migration-events"))
    .process_wide(false)
    .single_instance(false)
    .disable(Member::Log)
    .start()?;
// Removed when `guard` is dropped.
let guard = system.events().register(|event: &Event| {
    if let Event::WorkerFailed { name } = event {
        eprintln!("{name} failed");
    }
});
// Kept until the shutdown.
system.events().register(|_event: &Event| {}).keep();
assert_eq!(system.events().handler_count(), 2);
drop(guard);
assert_eq!(system.events().handler_count(), 1);
let _code = system.shutdown();
# }
# Ok::<(), system_suite::Error>(())
```

