<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <b>system-suite</b>
    <br>
    <sub><sup>SERVICE FOUNDATION</sup></sub>
</h1>

<div align="center">
    <a href="https://crates.io/crates/system-suite"><img alt="Crates.io" src="https://img.shields.io/crates/v/system-suite"></a>
    <a href="https://crates.io/crates/system-suite"><img alt="Downloads" src="https://img.shields.io/crates/d/system-suite?color=%230099ff"></a>
    <a href="https://docs.rs/system-suite"><img alt="docs.rs" src="https://img.shields.io/docsrs/system-suite"></a>
    <a href="https://github.com/jamesgober/system-suite/actions"><img alt="CI" src="https://github.com/jamesgober/system-suite/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://github.com/rust-lang/rfcs/blob/master/text/2495-min-rust-version.md"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.89%2B-blue"></a>
</div>

<br>

<div align="left">
    <p>
        <strong>system-suite</strong> is the foundation for Rust services. One dependency brings what every long-running program needs: system facts and a process guard, application paths, environment variables, configuration, logging, metrics, audit records, a registry, signal handling, time, and ids. A kernel starts them in the right order, supervises background work, reports health and readiness, captures crashes without losing the last log lines, and shuts everything down in reverse.
    </p>
    <p>
        Each part is its own published library behind its own feature, so a project compiles only what it uses. The suite adds the wiring: the boot order, typed handles to every service, the shutdown order, and the kernel. It has no async runtime and no unsafe code of its own; the few third-party crates its members bring (OS bindings, audited cryptography) are reviewed and recorded, one by one.
    </p>
    <br>
    <hr>
    <p>
        <strong>MSRV is 1.89+</strong> (Rust 2024 edition). <code>#![forbid(unsafe_code)]</code>.
    </p>
    <blockquote>
        <strong>Status: extended (<code>0.4.0</code>).</strong> A service boots with one call, reads its configuration (and reloads it on SIGHUP, all or nothing), starts its own subsystems in a declared order, runs supervised workers and timers with zero idle CPU, reports health, and stops in order on SIGTERM, SIGINT, or Ctrl+C within a deadline. Audit, registry, globals, randomness, and secrets arrive in <code>0.5.0</code>. The public API is in <a href="./docs/API.md"><code>docs/API.md</code></a>, the boot and shutdown orders in <a href="./docs/BOOT-ORDER.md"><code>docs/BOOT-ORDER.md</code></a>, and the schedule in <a href="./dev/ROADMAP.md"><code>dev/ROADMAP.md</code></a>.
    </blockquote>
</div>

<hr>
<br>

## Quick start

```toml
[dependencies]
system-suite = "0.4"

[build-dependencies]
# Optional: the build commit and time in the first log line.
system-suite = { version = "0.4", default-features = false, features = ["sys"] }
```

```rust
// src/main.rs
fn main() -> std::process::ExitCode {
    let system = match system_suite::start!() {        // boots every enabled part, in order
        Ok(system) => system,
        Err(error) => return error.report(),          // names the failing step and what to do
    };
    system_suite::log::info!(system.log(), "ready");
    system.run()                                       // parks until a stop signal, then shuts down in order
}
```

```rust
// build.rs (optional)
fn main() {
    system_suite::build::emit();
}
```

`start!()` names the service after its package, takes the single-instance lock, opens the log (standard error and `<logs>/<name>.log`), installs the crash hook, and returns only when everything is up. A second copy exits with code 75; a boot failure prints what failed and what to do, and exits with a code that says why ([exit codes](./docs/API.md#exit-codes)). `run()` returns `0` after a clean stop.

The configuration is `<config dir>/<name>.conf` (or the file named by `<NAME>_CONFIG`); every key has a secure default, so the file is optional. A file another user can change is refused.

```ini
# site.conf
[log]
level = "debug"

[kernel]
shutdown_deadline = "40s"

[http]
port = 8080
```

```rust
use system_suite::lifecycle::{Deadline, Failure, Link, Position, Subsystem};

struct Http { port: u16 }

impl Subsystem for Http {
    fn name(&self) -> &str { "http" }
    fn position(&self) -> Position { Position::LAST }      // the door opens last, closes first
    fn start(&mut self, link: &Link) -> Result<(), Failure> {
        self.port = link.foundation().config().get_or("http.port", 8080)?;
        Ok(())
    }
    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> { Ok(()) }
}

fn main() -> std::process::ExitCode {
    let system = match system_suite::System::builder("site")
        .register(Http { port: 0 })
        .start()
    {
        Ok(system) => system,
        Err(error) => return error.report(),
    };
    system.run()
}
```

`System::builder("name")` sets the rest: the directory scope (`Scope::Global` for system services), a portable root, the `.env` and configuration files, a test clock, which members to disable, the shutdown deadline, the subsystems, and `on_ready` callbacks. Frameworks that build their subsystems from the foundation use `builder.boot()`, `Boot::start`, and `Boot::ready`.

## Scope

| Part | Library | Feature |
|---|---|---|
| System facts, build info, instance info, process guard | sys-lib | `sys` (default) |
| Application and OS directories | path-lib | `paths` (default) |
| Environment variables, `.env` files | env-lib | `env` (default) |
| Configuration | config-lib | `config` (default) |
| Logging | log-io | `log` (default) |
| Metrics | metrics-lib | `metrics` (default) |
| Signals | signal-mod | `signals` (default) |
| Time | clock-lib | `time` (default) |
| Ids | id-forge | `ids` (default) |
| Audit records | audit-trail | `audit` |
| Registry | registry-io | `registry` |
| Globals | global-lib | `globals` |
| Randomness | mod-rand | `rand` |
| Secret storage | key-vault | `secrets` |
| Lifecycle, boot and shutdown order, workers, timers, health, crash capture | this crate | `kernel` (default) |

See [`dev/PLAN.md`](./dev/PLAN.md) for the full scope, the boot and shutdown orders, what is deliberately left to other crates, and the decision log.

<hr>
<br>

## Contributing

See <a href="./dev/DIRECTIVES.md"><code>dev/DIRECTIVES.md</code></a> for engineering standards and the definition of done. Before a PR: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features` must be clean.

<br>

<div id="license">
    <h2>License</h2>
    <p>Licensed under either of</p>
    <ul>
        <li><b>Apache License, Version 2.0</b>: <a href="./LICENSE-APACHE">LICENSE-APACHE</a></li>
        <li><b>MIT License</b>: <a href="./LICENSE-MIT">LICENSE-MIT</a></li>
    </ul>
    <p>at your option.</p>
</div>

<div align="center">
  <h2></h2>
  <sup>COPYRIGHT <small>&copy;</small> 2026 <strong>James Gober.</strong></sup>
</div>
