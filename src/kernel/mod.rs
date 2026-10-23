//! The suite's controller.
//!
//! The kernel receives the signals, decides when the process stops, runs
//! the boot and shutdown orders, and captures crashes. Its parts are their
//! own modules: [`lifecycle`](crate::lifecycle) (phases and, from `0.4`,
//! the subsystem contract) and, from `0.4`, `runtime` (workers and timers).
//! Everything here is `std` only, with no async runtime (D-005).

use std::{
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use crate::lifecycle::{Phase, StopReason};

pub(crate) mod crash;
pub(crate) mod notify;
#[cfg(feature = "signals")]
pub(crate) mod signals;

/// The kernel's control handle.
///
/// What a framework's core receives instead of raw signals: signals reach
/// only the kernel (D-006), and the rest of the process asks it for a stop
/// and waits for one. Cheap to clone (`Arc`), `Send + Sync`; every method is
/// safe from any thread.
///
/// # Examples
///
/// ```
/// use system_suite::{System, lifecycle::StopReason};
///
/// # #[cfg(feature = "sys")] {
/// let system = System::builder("kernel-doc")
///     .process_wide(false)
///     .single_instance(false)
///     .start()?;
/// let kernel = system.kernel().clone();
/// std::thread::spawn(move || {
///     kernel.request_stop("done");
/// });
/// assert_eq!(system.wait_for_stop(), StopReason::Requested { reason: "done" });
/// let code = system.shutdown();
/// # let _ = code;
/// # }
/// # Ok::<(), system_suite::Error>(())
/// ```
#[derive(Clone)]
pub struct Kernel {
    control: Arc<Control>,
}

struct Control {
    phase: AtomicU8,
    stop: Mutex<Option<StopReason>>,
    stopped: Condvar,
}

impl Kernel {
    pub(crate) fn new() -> Kernel {
        Kernel {
            control: Arc::new(Control {
                phase: AtomicU8::new(Phase::Booting as u8),
                stop: Mutex::new(None),
                stopped: Condvar::new(),
            }),
        }
    }

    /// The current lifecycle phase. One atomic load.
    #[must_use]
    pub fn phase(&self) -> Phase {
        Phase::from_u8(self.control.phase.load(Ordering::Acquire))
    }

    pub(crate) fn set_phase(&self, phase: Phase) {
        self.control.phase.store(phase as u8, Ordering::Release);
    }

    /// Asks for a clean stop, exactly as a SIGTERM would. Returns `false`
    /// if a stop was already requested (the first reason is kept). Never
    /// blocks.
    pub fn request_stop(&self, reason: &'static str) -> bool {
        self.stop_for(StopReason::Requested { reason })
    }

    /// Records `reason` as the stop reason if none is set, and wakes every
    /// thread waiting for a stop. Returns `false` if one was already set.
    pub(crate) fn stop_for(&self, reason: StopReason) -> bool {
        let mut stop = self.lock_stop();
        if stop.is_some() {
            return false;
        }
        *stop = Some(reason);
        drop(stop);
        self.control.stopped.notify_all();
        true
    }

    /// Why a stop was requested, if one was.
    #[must_use]
    pub fn stop_reason(&self) -> Option<StopReason> {
        self.lock_stop().clone()
    }

    /// Parks the calling thread, with zero CPU, until a stop is requested,
    /// and returns why. Several threads may wait at once.
    #[must_use = "the stop reason says why the system is stopping"]
    pub fn wait_for_stop(&self) -> StopReason {
        let mut stop = self.lock_stop();
        loop {
            if let Some(reason) = stop.as_ref() {
                return reason.clone();
            }
            stop = self
                .control
                .stopped
                .wait(stop)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Like [`wait_for_stop`](Self::wait_for_stop), but returns `None` if no
    /// stop was requested within `timeout`.
    #[must_use = "the stop reason says why the system is stopping"]
    pub fn wait_for_stop_timeout(&self, timeout: Duration) -> Option<StopReason> {
        let stop = self.lock_stop();
        let (stop, _) = self
            .control
            .stopped
            .wait_timeout_while(stop, timeout, |stop| stop.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        stop.clone()
    }

    fn lock_stop(&self) -> MutexGuard<'_, Option<StopReason>> {
        // The lock guards a plain Option written in one assignment; a panic
        // elsewhere while it was held cannot leave it half-written.
        self.control
            .stop
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl std::fmt::Debug for Kernel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Kernel")
            .field("phase", &self.phase())
            .field("stop_reason", &self.stop_reason())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Instant};

    use super::*;

    #[test]
    fn test_kernel_starts_booting_without_stop() {
        let kernel = Kernel::new();
        assert_eq!(kernel.phase(), Phase::Booting);
        assert_eq!(kernel.stop_reason(), None);
    }

    #[test]
    fn test_kernel_first_stop_reason_wins() {
        let kernel = Kernel::new();
        assert!(kernel.request_stop("first"));
        assert!(!kernel.request_stop("second"));
        assert_eq!(
            kernel.stop_reason(),
            Some(StopReason::Requested { reason: "first" })
        );
    }

    #[test]
    fn test_kernel_wait_for_stop_wakes_on_request() {
        let kernel = Kernel::new();
        let waiter = kernel.clone();
        let handle = thread::spawn(move || waiter.wait_for_stop());
        thread::sleep(Duration::from_millis(20));
        assert!(kernel.request_stop("test"));
        let reason = handle.join().ok();
        assert_eq!(reason, Some(StopReason::Requested { reason: "test" }));
    }

    #[test]
    fn test_kernel_wait_for_stop_timeout_returns_none() {
        let kernel = Kernel::new();
        let started = Instant::now();
        assert_eq!(
            kernel.wait_for_stop_timeout(Duration::from_millis(30)),
            None
        );
        assert!(started.elapsed() >= Duration::from_millis(30));
    }

    #[test]
    fn test_kernel_phase_is_shared_between_clones() {
        let kernel = Kernel::new();
        let other = kernel.clone();
        kernel.set_phase(Phase::Ready);
        assert_eq!(other.phase(), Phase::Ready);
    }
}
