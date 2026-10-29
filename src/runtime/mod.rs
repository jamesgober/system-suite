//! Where background work runs: supervised worker threads and timers,
//! without an async runtime and with zero CPU when idle.
//!
//! The kernel owns the runtime, asks it to stop at shutdown step 3, and
//! reports its health.

use std::{
    borrow::Cow,
    fmt,
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[cfg(not(feature = "rand"))]
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, ErrorKind, kernel::UrgentLine};

mod timer;
mod worker;

pub use timer::TimerHandle;
pub(crate) use timer::Timers;
pub use worker::{Worker, WorkerContext, WorkerHandle, WorkerState};

/// What happened in the runtime, for the kernel to log, count, and act on.
#[derive(Clone, Debug)]
pub(crate) enum Event {
    WorkerRestarted {
        name: String,
        restarts: u32,
        reason: String,
    },
    WorkerFailed {
        name: String,
        critical: bool,
        reason: String,
    },
    TimerPanicked {
        name: &'static str,
    },
}

pub(crate) type Observer = Box<dyn Fn(Event) + Send + Sync + 'static>;

/// The handle to the runtime. Cheap to clone; `Send + Sync`.
///
/// Workers are spawned with [`Worker::spawn`], timers with
/// [`every`](Self::every) and [`after`](Self::after).
///
/// # Examples
///
/// ```
/// use std::{sync::{Arc, atomic::{AtomicU32, Ordering}}, time::Duration};
/// use system_suite::{Member, System};
///
/// # #[cfg(feature = "sys")] {
/// let system = System::builder("runtime-doc")
///     .process_wide(false)
///     .single_instance(false)
///     .disable(Member::Log)
///     .start()?;
/// let ticks = Arc::new(AtomicU32::new(0));
/// let counter = Arc::clone(&ticks);
/// let timer = system.runtime().every("tick", Duration::from_millis(5), move || {
///     counter.fetch_add(1, Ordering::Relaxed);
/// })?;
/// std::thread::sleep(Duration::from_millis(50));
/// assert!(timer.cancel());
/// assert!(ticks.load(Ordering::Relaxed) > 0);
/// let code = system.shutdown();
/// # let _ = code;
/// # }
/// # Ok::<(), system_suite::Error>(())
/// ```
#[derive(Clone)]
pub struct Runtime {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) stopping: AtomicBool,
    pub(crate) urgent: UrgentLine,
    pub(crate) workers: Mutex<Vec<WorkerHandle>>,
    pub(crate) timers: Timers,
    pub(crate) observer: OnceLock<Observer>,
}

impl Runtime {
    pub(crate) fn new(urgent: UrgentLine) -> Runtime {
        Runtime {
            inner: Arc::new(Inner {
                stopping: AtomicBool::new(false),
                urgent,
                workers: Mutex::new(Vec::new()),
                timers: Timers::new(),
                observer: OnceLock::new(),
            }),
        }
    }

    /// Calls `tick` every `period` on the timer thread. The first call is
    /// one period from now; ticks missed while a callback ran long are
    /// skipped, not replayed. Callbacks must return quickly (well under a
    /// millisecond); heavier work wakes a worker. A callback that panics
    /// cancels its timer and is reported `Degraded`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Setup`] for a zero period or one over 30 days;
    /// [`ErrorKind::Stopping`] once a stop is under way; [`ErrorKind::Kernel`]
    /// if the timer thread cannot be started.
    pub fn every<F>(
        &self,
        name: &'static str,
        period: Duration,
        tick: F,
    ) -> Result<TimerHandle, Error>
    where
        F: FnMut() + Send + 'static,
    {
        self.check_accepting()?;
        timer::check_interval(name, period)?;
        self.inner
            .timers
            .every(name, period, Box::new(tick), self.observer())
    }

    /// Calls `fire` once, `delay` from now, on the timer thread.
    ///
    /// # Errors
    ///
    /// As for [`every`](Self::every).
    pub fn after<F>(
        &self,
        name: &'static str,
        delay: Duration,
        fire: F,
    ) -> Result<TimerHandle, Error>
    where
        F: FnOnce() + Send + 'static,
    {
        self.check_accepting()?;
        timer::check_interval(name, delay)?;
        self.inner
            .timers
            .after(name, delay, Box::new(fire), self.observer())
    }

    /// The workers spawned so far, for a dump or a status page.
    #[must_use]
    pub fn workers(&self) -> Vec<WorkerHandle> {
        self.inner
            .workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn check_accepting(&self) -> Result<(), Error> {
        if self.inner.stopping.load(Ordering::Acquire) {
            return Err(Error::new(
                ErrorKind::Stopping,
                "the runtime no longer accepts work: a stop is under way",
                "stop scheduling work; the process is going away",
            ));
        }
        Ok(())
    }

    /// Installs what the kernel does with runtime events. Only the first
    /// call counts.
    pub(crate) fn observe(&self, observer: Observer) {
        // The kernel installs exactly one observer.
        let _first = self.inner.observer.set(observer).is_ok();
    }

    fn observer(&self) -> Arc<Inner> {
        Arc::clone(&self.inner)
    }

    /// Asks every worker and the timer thread to stop, and waits for them
    /// until `deadline`. Returns the names of workers still running then.
    pub(crate) fn stop(&self, deadline: Instant) -> Vec<String> {
        self.inner.stopping.store(true, Ordering::Release);
        let workers = self.workers();
        for worker in &workers {
            worker.stop();
        }
        let mut late = Vec::new();
        if !self.inner.timers.stop(deadline) {
            late.push("the timer thread".to_owned());
        }
        for worker in workers {
            if !worker.join_until(deadline) {
                late.push(worker.name().to_owned());
            }
        }
        late
    }
}

impl Inner {
    pub(crate) fn emit(&self, event: Event) {
        if let Some(observer) = self.observer.get() {
            observer(event);
        }
    }
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runtime")
            .field("stopping", &self.inner.stopping.load(Ordering::Acquire))
            .field("workers", &self.workers().len())
            .finish_non_exhaustive()
    }
}

/// When a worker is started again after its function returns.
///
/// # Examples
///
/// ```
/// use system_suite::runtime::Restart;
///
/// assert_eq!(Restart::default(), Restart::OnFailure);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Restart {
    /// Never: the worker ends when its function returns, whatever it
    /// returned.
    Never,
    /// After an `Err` or a panic (the default).
    #[default]
    OnFailure,
    /// After an `Err`, a panic, or `Ok(())`.
    Always,
}

/// How long to wait before a restart.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use system_suite::runtime::Backoff;
///
/// let b = Backoff::exponential(Duration::from_millis(100), Duration::from_secs(30));
/// assert!(b.delay(0) <= Duration::from_millis(125));
/// assert!(b.delay(20) <= Duration::from_millis(37_500));
/// assert_eq!(Backoff::Fixed(Duration::from_secs(1)).delay(5), Duration::from_secs(1));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Backoff {
    /// Restart at once.
    None,
    /// Always the same delay.
    Fixed(Duration),
    /// Doubling from `min` to `max`, with up to 25% random jitter so many
    /// workers failing together do not restart together.
    Exponential {
        /// The first delay.
        min: Duration,
        /// The longest delay, before jitter.
        max: Duration,
    },
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff::exponential(Duration::from_millis(100), Duration::from_secs(30))
    }
}

impl Backoff {
    /// Exponential backoff from `min` to `max`.
    #[must_use]
    pub const fn exponential(min: Duration, max: Duration) -> Backoff {
        Backoff::Exponential { min, max }
    }

    /// The delay before restart number `attempt` (0 for the first).
    /// Saturating: never panics, whatever the attempt.
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Duration {
        match *self {
            Backoff::None => Duration::ZERO,
            Backoff::Fixed(delay) => delay,
            Backoff::Exponential { min, max } => {
                let factor = 1_u32.checked_shl(attempt.min(31)).unwrap_or(u32::MAX);
                let base = min.saturating_mul(factor).min(max.max(min));
                base.saturating_add(base.mul_f64(jitter_fraction(attempt)))
            }
        }
    }
}

/// A fraction in `[0, 0.25)`, different from call to call. Jitter only
/// spreads restarts out; it needs no cryptographic quality.
fn jitter_fraction(attempt: u32) -> f64 {
    // mod-rand's process-unique value when compiled in, else the clock's
    // nanoseconds; mixed with the attempt either way.
    #[cfg(feature = "rand")]
    let seed = mod_rand::tier2::unique_u64();
    #[cfg(not(feature = "rand"))]
    let seed = u64::from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0),
    );
    let mut x = seed ^ (u64::from(attempt) << 32) ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    // 24 bits of the mix, scaled to [0, 0.25).
    ((x >> 40) as f64 / f64::from(1_u32 << 24)) * 0.25
}

/// Checks a worker's or timer's name: the application-name rule.
pub(crate) fn check_name(what: &str, name: &str) -> Result<(), Error> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::Setup,
            format!("the {what} name {name:?} is not valid"),
            "use 1 to 64 ASCII letters, digits, '-', '_', or '.', not starting with '.'",
        ))
    }
}

/// A name for logs and handles: borrowed when static.
pub(crate) type Name = Cow<'static, str>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_doubles_and_caps_with_bounded_jitter() {
        let b = Backoff::exponential(Duration::from_millis(100), Duration::from_secs(1));
        for attempt in 0..40 {
            let delay = b.delay(attempt);
            let base = Duration::from_millis(100)
                .saturating_mul(1 << attempt.min(31))
                .min(Duration::from_secs(1));
            assert!(delay >= base, "{attempt}: {delay:?} < {base:?}");
            assert!(delay <= base.mul_f64(1.25), "{attempt}: {delay:?}");
        }
        assert_eq!(Backoff::None.delay(3), Duration::ZERO);
    }

    #[test]
    fn test_jitter_fraction_stays_in_range() {
        for attempt in 0..1_000 {
            let f = jitter_fraction(attempt);
            assert!((0.0..0.25).contains(&f), "{f}");
        }
    }

    #[test]
    fn test_check_name_uses_the_application_rule() {
        assert!(check_name("worker", "log-shipper").is_ok());
        assert!(check_name("worker", "").is_err());
        assert!(check_name("worker", "has space").is_err());
    }
}
