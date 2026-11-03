//! The kernel's record of what happened (urgent line tier T2), delivered
//! to callbacks registered on `Foundation::events()` (feature `registry`).

use std::{fmt, sync::Arc, time::Duration};

use registry_io::{HandlerGuard, HandlerId, SyncRegistry};

use super::Notice;
use crate::lifecycle::Phase;

/// Where callbacks for the kernel's [`Event`]s are registered
/// (`Foundation::events`). Only the kernel delivers events: application
/// code can listen, never publish one (D-085).
#[derive(Clone)]
pub struct Events {
    registry: Arc<SyncRegistry<Event>>,
}

/// A registered callback, removed when the guard is dropped.
#[must_use = "the callback is removed when the guard is dropped"]
pub struct EventGuard {
    _guard: HandlerGuard<Event>,
}

/// A callback registered for the life of the process, until
/// [`Events::unregister`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EventId(HandlerId);

impl Events {
    pub(crate) fn new() -> Events {
        Events {
            registry: Arc::new(SyncRegistry::new()),
        }
    }

    pub(crate) fn notify(&self, event: &Event) {
        self.registry.notify(event);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }

    /// Removes every callback and waits for running ones (shutdown).
    pub(crate) fn clear_and_wait(&self) {
        self.registry.clear_and_wait();
    }

    /// What to do when a callback panics (the kernel logs it).
    pub(crate) fn on_panic<F>(&self, callback: F)
    where
        F: Fn(&registry_io::PanicInfo<'_>) + Send + Sync + 'static,
    {
        self.registry.on_panic(callback);
    }

    /// Calls `handler` for every event until the returned guard is
    /// dropped.
    pub fn register_guard<F>(&self, handler: F) -> EventGuard
    where
        F: Fn(&Event) + Send + Sync + 'static,
    {
        EventGuard {
            _guard: self.registry.register_guard(handler),
        }
    }

    /// Calls `handler` for every event until [`unregister`](Self::unregister).
    pub fn register<F>(&self, handler: F) -> EventId
    where
        F: Fn(&Event) + Send + Sync + 'static,
    {
        EventId(self.registry.register(handler))
    }

    /// Removes a callback; `false` if it was already gone.
    pub fn unregister(&self, id: EventId) -> bool {
        self.registry.unregister(id.0)
    }

    /// How many callbacks are registered.
    #[must_use]
    pub fn handler_count(&self) -> usize {
        self.registry.handler_count()
    }

    /// How many callbacks have panicked so far (each panic is isolated).
    #[must_use]
    pub fn panic_count(&self) -> u64 {
        self.registry.panic_count()
    }
}

impl fmt::Debug for Events {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Events")
            .field("handlers", &self.handler_count())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for EventGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventGuard").finish_non_exhaustive()
    }
}

/// Something the kernel did or saw, delivered synchronously to every
/// callback registered on [`Foundation::events`](crate::Foundation::events).
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
