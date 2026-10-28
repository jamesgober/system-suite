//! The atomics behind the urgent line and worker heartbeats: std's, or
//! loom's in a `--cfg loom` test build, so the model tests check the code
//! that ships (D-056). Run them with
//! `RUSTFLAGS="--cfg loom" cargo test --lib --release --no-default-features --features kernel loom_`
//! (only the kernel feature: clock-lib reads `cfg(loom)` too and does not
//! build as a dependency under it).

#[cfg(all(loom, test))]
pub(crate) use loom::sync::atomic::{AtomicU8, AtomicU64, Ordering};
#[cfg(not(all(loom, test)))]
pub(crate) use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
