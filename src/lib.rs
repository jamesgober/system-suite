//! # system_suite
//!
//! The foundation package for Rust services. One dependency brings the
//! essentials every long-running program needs (system facts, paths,
//! environment, logging, signals, time, and ids; configuration, metrics,
//! and the rest as they are wired) and a kernel that starts them in the
//! right order and stops them in reverse.
//!
//! Each member is its own published library, behind its own feature, so a
//! project compiles only what it uses. The suite adds the wiring: the boot
//! order, typed handles to every service, the shutdown order, and the
//! kernel.
//!
//! ## Tier 1
//!
//! ```no_run
//! # #[cfg(all(feature = "kernel", feature = "log"))]
//! fn main() -> std::process::ExitCode {
//!     let system = match system_suite::start!() {      // boots every member in order
//!         Ok(system) => system,
//!         Err(error) => return error.report(),        // the failing step, and what to do
//!     };
//!     system_suite::log::info!(system.log(), "ready");  // typed handles, no lookups by name
//!     system.run()                                      // parks until a stop signal, then stops in order
//! }
//! # #[cfg(not(all(feature = "kernel", feature = "log")))]
//! # fn main() {}
//! ```
//!
//! For the build commit and time in the first log line, add the suite as a
//! build dependency with the `sys` feature and call
//! [`build::emit`] from `build.rs`:
//!
//! ```toml
//! [build-dependencies]
//! system-suite = { version = "0.6", default-features = false, features = ["sys"] }
//! ```
//!
//! ```ignore
//! // build.rs: a build script, so it is not run as a doctest here.
//! fn main() {
//!     system_suite::build::emit();
//! }
//! ```
//!
//! ## Boot and shutdown
//!
//! [`start!`] runs the boot order of `docs/BOOT-ORDER.md`: build info,
//! instance, paths, environment, configuration, the process guard, the
//! signal intake, directories, logging, crash capture, metrics, the kernel,
//! the application's subsystems, ready. A failing step stops the boot and
//! unwinds what started; the [`Error`] names the step and what to do.
//! [`System::run`] waits for a stop (SIGTERM, SIGINT, Ctrl+C, a console
//! close, or `System::request_stop`) and stops everything in reverse within
//! a deadline: the subsystems first, then the workers and timers, then the
//! members.
//!
//! ## Status
//!
//! `0.5`: feature complete. Configuration with secret references and
//! two-phase reload, metrics, audit, kernel events, subsystems, the urgent
//! line, supervised workers, timers, health, and Windows service control.
//! `0.6` measured every performance target (`docs/PERFORMANCE.md`). The public API is in
//! `docs/API.md` and the schedule in `dev/ROADMAP.md` (both in the
//! [repository](https://github.com/jamesgober/system-suite)).

#![doc(html_root_url = "https://docs.rs/system-suite")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable,
        clippy::dbg_macro,
        clippy::print_stdout,
        clippy::print_stderr
    )
)]

#[cfg(feature = "audit")]
mod audit_log;
mod boot;
mod builder;
#[cfg(feature = "config")]
#[cfg_attr(docsrs, doc(cfg(feature = "config")))]
pub mod config;
mod error;
pub mod exit;
#[cfg(feature = "globals")]
mod globals;
#[cfg(feature = "kernel")]
pub mod kernel;
#[cfg(feature = "kernel")]
pub mod lifecycle;
mod logging;
mod member;
#[cfg(feature = "metrics")]
mod metrics_handle;
#[cfg(feature = "kernel")]
pub mod runtime;
#[cfg(feature = "secrets")]
mod secrets;
// Without `config` only the defaults exist, so some values are never built.
#[cfg_attr(not(feature = "config"), allow(dead_code))]
mod sections;
#[cfg(any(feature = "env", feature = "config"))]
mod secure_file;
#[cfg(any(feature = "log", feature = "signals", feature = "kernel"))]
mod shutdown;
mod snapshot;
#[cfg(feature = "kernel")]
mod staged;
#[cfg(feature = "kernel")]
mod sync;
mod system;

#[cfg(feature = "audit")]
#[cfg_attr(docsrs, doc(cfg(feature = "audit")))]
pub use crate::audit_log::AuditLog;
#[cfg(feature = "globals")]
#[cfg_attr(docsrs, doc(cfg(feature = "globals")))]
pub use crate::globals::foundation;
#[cfg(feature = "kernel")]
pub use crate::staged::Boot;
pub use crate::{
    builder::Builder,
    error::{Error, ErrorKind, Result},
    member::{Member, Step},
    snapshot::{Reader, Snapshot, SnapshotView},
    system::{Foundation, System},
};

/// Crate version string, populated by Cargo at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------------------
// Member modules (SUI-24): each member re-exported whole.
// ---------------------------------------------------------------------------

/// System facts and the boot kit: [sys-lib](https://docs.rs/sys-lib).
#[cfg(feature = "sys")]
#[cfg_attr(docsrs, doc(cfg(feature = "sys")))]
pub use sys_lib as sys;

/// Application and OS directories: [path-lib](https://docs.rs/path-lib).
#[cfg(feature = "paths")]
#[cfg_attr(docsrs, doc(cfg(feature = "paths")))]
pub use path_lib as path;

/// Environment variables and `.env` files: [env-lib](https://docs.rs/env-lib).
#[cfg(feature = "env")]
#[cfg_attr(docsrs, doc(cfg(feature = "env")))]
pub use env_lib as env;

/// Structured logging: [log-io](https://docs.rs/log-io). A project that
/// also uses the `log` crate writes `use system_suite::log as slog;`
/// (D-015).
#[cfg(feature = "log")]
#[cfg_attr(docsrs, doc(cfg(feature = "log")))]
pub use log_io as log;

/// Signals: [signal-mod](https://docs.rs/signal-mod). The kernel owns the
/// process's signal intake; this module is for its types.
#[cfg(feature = "signals")]
#[cfg_attr(docsrs, doc(cfg(feature = "signals")))]
pub use signal_mod as signal;

/// Metrics: [metrics-lib](https://docs.rs/metrics-lib). The registry the
/// system uses is [`Foundation::metrics`].
#[cfg(feature = "metrics")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics")))]
pub use metrics_lib as metrics;

/// Tamper-evident audit records: [audit-trail](https://docs.rs/audit-trail).
/// The system's chain is [`Foundation::audit`].
#[cfg(feature = "audit")]
#[cfg_attr(docsrs, doc(cfg(feature = "audit")))]
pub use audit_trail as audit;

/// Event and callback registry: [registry-io](https://docs.rs/registry-io).
/// The kernel's events arrive on [`Foundation::events`].
#[cfg(feature = "registry")]
#[cfg_attr(docsrs, doc(cfg(feature = "registry")))]
pub use registry_io as registry;

/// Process-wide typed values: [global-lib](https://docs.rs/global-lib).
#[cfg(feature = "globals")]
#[cfg_attr(docsrs, doc(cfg(feature = "globals")))]
pub use global_lib as global;

/// Tiered randomness: [mod-rand](https://docs.rs/mod-rand). Tokens and
/// nonces come from `rand::tier3` (the OS CSPRNG). A project that also
/// uses the `rand` crate writes `use system_suite::rand as srand;`.
#[cfg(feature = "rand")]
#[cfg_attr(docsrs, doc(cfg(feature = "rand")))]
pub use mod_rand as rand;

/// Page-locked secret storage: [key-vault](https://docs.rs/key-vault). The
/// system's vault is [`Foundation::vault`].
#[cfg(feature = "secrets")]
#[cfg_attr(docsrs, doc(cfg(feature = "secrets")))]
pub use key_vault as vault;

/// Monotonic and wall clocks, mockable: [clock-lib](https://docs.rs/clock-lib).
#[cfg(feature = "time")]
#[cfg_attr(docsrs, doc(cfg(feature = "time")))]
pub use clock_lib as clock;

/// ULID, UUID, Snowflake, and NanoID: [id-forge](https://docs.rs/id-forge).
#[cfg(feature = "ids")]
#[cfg_attr(docsrs, doc(cfg(feature = "ids")))]
pub use id_forge as id;

/// The application's build info as a `&'static sys::BuildInfo`. It must
/// expand in the application crate, which is why [`start!`] is a macro
/// (D-008). See [`build::emit`].
#[cfg(feature = "sys")]
#[cfg_attr(docsrs, doc(cfg(feature = "sys")))]
pub use sys_lib::build_info;

/// The build-script side of build info: call [`build::emit`] from the
/// application's `build.rs`.
#[cfg(feature = "sys")]
#[cfg_attr(docsrs, doc(cfg(feature = "sys")))]
pub mod build {
    pub use sys_lib::build::{Emitter, emit, emitter};
}

/// Boots every enabled member and the kernel, in the boot order, and
/// returns `Result<System, Error>`.
///
/// `start!()` names the application after its package; `start!("name")`
/// names it explicitly. It records the application's build info (which is
/// why it is a macro, D-008); everything else it does is available through
/// [`Builder`].
///
/// # Examples
///
/// ```no_run
/// # #[cfg(feature = "kernel")]
/// fn main() -> std::process::ExitCode {
///     let system = match system_suite::start!("hive") {
///         Ok(system) => system,
///         Err(error) => return error.report(),
///     };
///     system.run()
/// }
/// # #[cfg(not(feature = "kernel"))]
/// # fn main() {}
/// ```
#[cfg(feature = "sys")]
#[macro_export]
macro_rules! start {
    () => {
        $crate::start!(::core::env!("CARGO_PKG_NAME"))
    };
    ($name:expr $(,)?) => {
        $crate::System::builder($name)
            .build_info($crate::build_info!())
            .start()
    };
}

/// Boots every enabled member and the kernel, in the boot order, and
/// returns `Result<System, Error>`.
///
/// `start!()` names the application after its package; `start!("name")`
/// names it explicitly. Without the `sys` feature there is no build info to
/// record; everything else it does is available through [`Builder`].
///
/// # Examples
///
/// ```no_run
/// let system = system_suite::start!("tool");
/// # let _ = system;
/// ```
#[cfg(not(feature = "sys"))]
#[macro_export]
macro_rules! start {
    () => {
        $crate::start!(::core::env!("CARGO_PKG_NAME"))
    };
    ($name:expr $(,)?) => {
        $crate::System::builder($name).start()
    };
}

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn test_version_matches_manifest() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        // Pre-release and build metadata (`1.0.0-rc.1`, `+build`) follow the core.
        let core = VERSION.split(['-', '+']).next().unwrap_or_default();
        assert_eq!(core.split('.').count(), 3, "expected MAJOR.MINOR.PATCH");
        assert!(
            core.split('.').all(|part| part.parse::<u64>().is_ok()),
            "every version component is a number"
        );
    }
}
