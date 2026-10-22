//! # system_suite
//!
//! The foundation package for Rust services. One dependency brings the
//! essentials every long-running program needs (system facts, paths,
//! environment, configuration, logging, metrics, audit, registry, signals,
//! time, and ids) and a kernel that starts them in the right order, keeps
//! them supervised, and stops them in reverse.
//!
//! Each member is its own published library, behind its own feature, so a
//! project compiles only what it uses. The suite adds the wiring: the boot
//! order, typed handles to every service, the shutdown order, and the
//! kernel.
//!
//! ## Tier 1
//!
//! ```
//! println!("system-suite {}", system_suite::VERSION);
//! ```
//!
//! The Tier 1 entry points arrive from `0.3.0` on:
//!
//! ```text
//! let system = system_suite::start!()?;                 // boots every member in order; returns when ready
//! system_suite::log::info!(system.log(), "ready");      // typed handles, no lookups by name
//! system.run()                                          // parks until a stop signal, then shuts down in order
//! ```
//!
//! ## Status
//!
//! Scaffold. The public API is written in `docs/API.md` and the boot and
//! shutdown orders in `docs/BOOT-ORDER.md`; the design is in `dev/PLAN.md`
//! and the milestone schedule in `dev/ROADMAP.md` (all in the
//! [repository](https://github.com/jamesgober/system-suite)). The first
//! boot path ships in `0.3.0`.

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
        clippy::dbg_macro
    )
)]

/// Crate version string, populated by Cargo at build time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

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
