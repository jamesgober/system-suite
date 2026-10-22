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
        Each part is its own published library behind its own feature, so a project compiles only what it uses. The suite adds the wiring: the boot order, typed handles to every service, the shutdown order, and the kernel. It has no async runtime, no unsafe code of its own, and no third-party runtime dependencies in its default features.
    </p>
    <br>
    <hr>
    <p>
        <strong>MSRV is 1.89+</strong> (Rust 2024 edition). <code>#![forbid(unsafe_code)]</code>.
    </p>
    <blockquote>
        <strong>Status: scaffold (<code>0.1.0</code>).</strong> No functional code yet. The design is written down in <a href="./dev/PLAN.md"><code>dev/PLAN.md</code></a> and the schedule in <a href="./dev/ROADMAP.md"><code>dev/ROADMAP.md</code></a>; the public API is locked in <code>0.2.0</code> and the first boot path ships in <code>0.3.0</code>.
    </blockquote>
</div>

<hr>
<br>

## Scope

The shape of the Tier 1 API, arriving from `0.3.0`:

```rust
fn main() -> std::process::ExitCode {
    let system = match system_suite::start("site") {   // boots every enabled part, in order
        Ok(system) => system,
        Err(error) => return error.exit_code(),       // names the failing step and what to do
    };
    system.log().info("ready");
    system.run()                                       // parks until a stop signal, then shuts down in order
}
```

| Part | Library | Feature |
|---|---|---|
| System facts, build info, instance info, process guard | sys-lib | `sys` (default) |
| Application and OS directories | path-lib | `paths` (default) |
| Environment variables, `.env` files | env-lib | `env` (default) |
| Configuration | to be decided at the design lock | `config` (default) |
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
