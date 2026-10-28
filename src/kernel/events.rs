//! The kernel's record of what happened (urgent line tier T2), delivered
//! to callbacks registered on `Foundation::events()` (feature `registry`).

use std::time::Duration;

use super::Notice;
use crate::lifecycle::Phase;

/// Something the kernel did or saw, delivered synchronously to every
/// callback registered on [`Foundation::events`](crate::Foundation::events)
/// (registry-io's `SyncRegistry`).
///
/// The registry declares and records; it never delivers urgent changes,
/// which reach workers and subsystems through the urgent line first
/// (D-006). A panicking callback is isolated by registry-io, logged, and
/// does not stop the kernel. Callbacks run on the thread that caused the
/// event and must return quickly.
///
/// # Examples
///
/// ```no_run
/// # #[cfg(feature = "registry")] {
/// use system_suite::kernel::Event;
///
/// let system = system_suite::start!()?;
/// let _guard = system.events().register_guard(|event: &Event| {
///     if let Event::WorkerFailed { name } = event {
///         eprintln!("page the on-call engineer: {name} failed");
///     }
/// });
/// # }
/// # Ok::<(), system_suite::Error>(())
/// ```
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Event {
    /// The lifecycle phase changed.
    Phase {
        /// The phase before.
        from: Phase,
        /// The phase now.
        to: Phase,
    },
    /// The urgent line changed.
    Urgent {
        /// The new level, epoch, and reason.
        notice: Notice,
    },
    /// A reload was published and committed.
    Reloaded {
        /// The new configuration version.
        version: u64,
    },
    /// A reload was refused; nothing changed.
    ReloadRefused {
        /// The subsystem that refused, or `"configuration"` when the file
        /// itself did not load or validate.
        by: String,
        /// Why, in the refusing side's words.
        reason: String,
    },
    /// A worker failed and was restarted.
    WorkerRestarted {
        /// The worker's name.
        name: String,
        /// Restarts so far.
        restarts: u32,
    },
    /// A worker exhausted its restart budget.
    WorkerFailed {
        /// The worker's name.
        name: String,
    },
    /// A subsystem's `stop` returned or was abandoned at shutdown.
    SubsystemStopped {
        /// The subsystem's name.
        name: String,
        /// How long its stop took (or ran before it was abandoned).
        took: Duration,
        /// Whether it returned before its deadline.
        met_deadline: bool,
    },
}
