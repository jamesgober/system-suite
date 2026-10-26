//! The suite's controller.
//!
//! The kernel receives the signals, decides when the process stops, raises
//! the urgent line, runs the boot and shutdown orders, aggregates health,
//! reports readiness, and captures crashes. The parts it drives are their
//! own modules: [`lifecycle`](crate::lifecycle) (the phases and the contract
//! every subsystem follows) and [`runtime`](crate::runtime) (supervised
//! workers and timers) (D-037). Everything here is `std` only, with no
//! async runtime (D-005).

use std::{
    borrow::Cow,
    error::Error as StdError,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use crate::{
    Member,
    lifecycle::{Mailbox, Phase, StopReason, Subsystem},
    logging::{LogSlot, Val},
    runtime::Runtime,
};

pub(crate) mod crash;
mod health;
pub(crate) mod metrics;
pub(crate) mod notify;
#[cfg(feature = "signals")]
pub(crate) mod signals;
mod urgent;

pub use health::{Component, ComponentHealth, Health, HealthReport, Status};
pub use urgent::{Level, Notice, UrgentLine, Watch};

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
/// use system_suite::{Member, System, lifecycle::StopReason};
///
/// # #[cfg(feature = "sys")] {
/// let system = System::builder("kernel-doc")
///     .process_wide(false)
///     .single_instance(false)
///     .disable(Member::Log)
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
    urgent: UrgentLine,
    runtime: Runtime,
    subsystems: Mutex<Vec<Arc<Slot>>>,
    members: OnceLock<Vec<Member>>,
    log: OnceLock<LogSlot>,
    metrics: OnceLock<metrics::KernelMetrics>,
}

/// A started subsystem, as the kernel holds it.
pub(crate) struct Slot {
    pub(crate) name: String,
    pub(crate) subsystem: Mutex<Box<dyn Subsystem>>,
    pub(crate) last: Mutex<Health>,
    pub(crate) mailbox: Mailbox,
}

impl Slot {
    /// The subsystem's health without waiting: while `start`, `stop`, or
    /// `reload` holds the subsystem, the last reported health is used.
    fn health(&self) -> Health {
        match self.subsystem.try_lock() {
            Ok(subsystem) => {
                let health = subsystem.health();
                *self.last.lock().unwrap_or_else(PoisonError::into_inner) = health.clone();
                health
            }
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                let health = poisoned.into_inner().health();
                *self.last.lock().unwrap_or_else(PoisonError::into_inner) = health.clone();
                health
            }
            Err(std::sync::TryLockError::WouldBlock) => self
                .last
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        }
    }
}

impl Kernel {
    pub(crate) fn new() -> Kernel {
        let urgent = UrgentLine::detached();
        Kernel {
            control: Arc::new(Control {
                phase: AtomicU8::new(Phase::Booting as u8),
                stop: Mutex::new(None),
                stopped: Condvar::new(),
                runtime: Runtime::new(urgent.clone()),
                urgent,
                subsystems: Mutex::new(Vec::new()),
                members: OnceLock::new(),
                log: OnceLock::new(),
                metrics: OnceLock::new(),
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

    /// The urgent line.
    #[must_use]
    pub fn urgent(&self) -> &UrgentLine {
        &self.control.urgent
    }

    /// The runtime: supervised workers and timers.
    #[must_use]
    pub fn runtime(&self) -> &Runtime {
        &self.control.runtime
    }

    /// Every member, subsystem, worker, and the timer thread. Allocates;
    /// for health endpoints and the dump signal.
    #[must_use]
    pub fn health(&self) -> HealthReport {
        let mut components = Vec::new();
        for member in self.members() {
            components.push(ComponentHealth {
                kind: Component::Member(*member),
                name: Cow::Borrowed(member.name()),
                health: Health::HEALTHY,
            });
        }
        for slot in self.slots() {
            components.push(ComponentHealth {
                kind: Component::Subsystem,
                name: Cow::Owned(slot.name.clone()),
                health: slot.health(),
            });
        }
        for worker in self.control.runtime.workers() {
            components.push(ComponentHealth {
                kind: Component::Worker,
                name: Cow::Owned(worker.name().to_owned()),
                health: worker.health(),
            });
        }
        components.push(ComponentHealth {
            kind: Component::Timers,
            name: Cow::Borrowed("timers"),
            health: self.control.runtime.inner.timers.health(),
        });
        HealthReport {
            status: health::aggregate(components.iter().map(|c| c.health.status())),
            phase: self.phase(),
            urgent: self.control.urgent.level(),
            components,
        }
    }

    /// The aggregate status alone. Builds no report: no allocation of its
    /// own (a subsystem's `health` may allocate a reason).
    #[must_use]
    pub fn status(&self) -> Status {
        let mut status = self.control.runtime.inner.timers.status();
        {
            let slots = self
                .control
                .subsystems
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for slot in slots.iter() {
                status = status.max(slot.health().status());
            }
        }
        let workers = self
            .control
            .runtime
            .inner
            .workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for worker in workers.iter() {
            status = status.max(worker.status());
        }
        status
    }

    /// Records a subsystem's handled error: logged and counted.
    pub(crate) fn report_subsystem_error(&self, name: &str, error: &(dyn StdError + 'static)) {
        if let Some(log) = self.control.log.get() {
            log.error(
                "subsystem error",
                &[
                    ("subsystem", Val::Str(name)),
                    ("error", Val::Str(&error.to_string())),
                ],
            );
        }
        if let Some(metrics) = self.control.metrics.get() {
            metrics.subsystem_error(name);
        }
    }

    /// Connects the kernel to the system's log and metrics, and installs
    /// what happens on urgent changes and runtime events. Called once, at
    /// boot step 15.
    pub(crate) fn wire(&self, members: Vec<Member>, log: LogSlot, metrics: metrics::KernelMetrics) {
        let _first = self.control.members.set(members).is_ok();
        let _first = self.control.log.set(log.clone()).is_ok();
        let _first = self.control.metrics.set(metrics.clone()).is_ok();
        let urgent_log = log.clone();
        let urgent_metrics = metrics.clone();
        self.control.urgent.observe(Box::new(move |notice| {
            urgent_log.warn(
                "urgent line changed",
                &[
                    ("level", Val::Str(notice.level.name())),
                    ("epoch", Val::U64(notice.epoch)),
                    ("reason", Val::Str(notice.reason)),
                ],
            );
            urgent_metrics.urgent(notice.level);
        }));
        let weak = Arc::downgrade(&self.control);
        self.control
            .runtime
            .observe(Box::new(move |event| match event {
                crate::runtime::Event::WorkerRestarted {
                    name,
                    restarts,
                    reason,
                } => {
                    log.warn(
                        "worker restarted",
                        &[
                            ("worker", Val::Str(&name)),
                            ("restarts", Val::U64(u64::from(restarts))),
                            ("reason", Val::Str(&reason)),
                        ],
                    );
                    metrics.worker_restarted(&name);
                }
                crate::runtime::Event::WorkerFailed {
                    name,
                    critical,
                    reason,
                } => {
                    log.error(
                        "worker failed",
                        &[("worker", Val::Str(&name)), ("reason", Val::Str(&reason))],
                    );
                    metrics.worker_failed(&name);
                    if critical && let Some(control) = weak.upgrade() {
                        let kernel = Kernel { control };
                        let _first = kernel.stop_for(StopReason::WorkerFailed { name });
                    }
                }
                crate::runtime::Event::TimerPanicked { name } => {
                    log.error(
                        "timer panicked and was cancelled",
                        &[("timer", Val::Str(name))],
                    );
                }
            }));
    }

    /// Registers a started subsystem, so health sees it and the urgent line
    /// reaches its mailbox.
    pub(crate) fn add_slot(&self, slot: Arc<Slot>) {
        self.control.urgent.subscribe(slot.mailbox.clone());
        self.control
            .subsystems
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(slot);
    }

    /// The started subsystems, in start order.
    pub(crate) fn slots(&self) -> Vec<Arc<Slot>> {
        self.control
            .subsystems
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Removes and returns the last started subsystem (for the shutdown).
    pub(crate) fn pop_slot(&self) -> Option<Arc<Slot>> {
        self.control
            .subsystems
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
    }

    pub(crate) fn metrics(&self) -> Option<&metrics::KernelMetrics> {
        self.control.metrics.get()
    }

    fn members(&self) -> &[Member] {
        self.control.members.get().map(Vec::as_slice).unwrap_or(&[])
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
            .field("urgent", &self.control.urgent.level())
            .finish_non_exhaustive()
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
        assert_eq!(kernel.urgent().level(), Level::Normal);
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

    #[test]
    fn test_critical_worker_failure_requests_a_stop() {
        use crate::runtime::{Restart, Worker};

        let kernel = Kernel::new();
        kernel.wire(
            Vec::new(),
            LogSlot::default(),
            metrics::KernelMetrics::none(),
        );
        let _worker = Worker::new("vital")
            .restart(Restart::Never)
            .critical(true)
            .spawn(kernel.runtime(), |_| {
                Err(crate::lifecycle::Failure::new("cannot continue"))
            });
        let reason = kernel.wait_for_stop_timeout(Duration::from_secs(10));
        assert_eq!(
            reason,
            Some(StopReason::WorkerFailed {
                name: "vital".to_owned()
            })
        );
        assert_eq!(kernel.status(), Status::Unhealthy);
    }

    #[test]
    fn test_health_lists_members_workers_and_timers() {
        let kernel = Kernel::new();
        kernel.wire(
            vec![Member::Log],
            LogSlot::default(),
            metrics::KernelMetrics::none(),
        );
        let report = kernel.health();
        assert_eq!(report.status(), Status::Healthy);
        assert!(
            report
                .components()
                .iter()
                .any(|c| c.kind == Component::Member(Member::Log))
        );
        assert!(
            report
                .components()
                .iter()
                .any(|c| c.kind == Component::Timers)
        );
    }
}
