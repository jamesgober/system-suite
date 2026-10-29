//! Supervised workers.

use std::{
    collections::VecDeque,
    fmt,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering},
    },
    thread::{self, JoinHandle, Thread},
    time::{Duration, Instant},
};

use super::{Backoff, Event, Inner, Name, Restart, Runtime, check_name};
use crate::{
    Error, ErrorKind,
    kernel::{Health, Level, UrgentLine},
    lifecycle::Failure,
};

/// A supervised worker, before it is spawned.
///
/// A named thread the kernel supervises: the services' own housekeeping
/// (log shipping, metrics export, cache sweeps, a job queue's consumer).
///
/// - `run` is called on the worker's thread. Returning `Ok(())` ends the
///   worker (restarted only under [`Restart::Always`], and never once a stop
///   was requested). Returning `Err` or panicking is a failure: it is logged
///   and counted, and the worker restarts per its policy after its backoff.
/// - A worker that fails more often than its budget allows within the
///   window is marked `Failed` and reported. If it is `critical`, the kernel
///   requests a stop (`StopReason::WorkerFailed`, exit code 70).
/// - With `heartbeat(every)`, a worker that has not called
///   [`WorkerContext::heartbeat`] for three periods is reported `Degraded`.
/// - At shutdown step 3 every worker is asked to stop
///   ([`WorkerContext::should_stop`] becomes true and `park` returns) and
///   joined within the deadline; one that does not return is abandoned.
/// - Under `panic = "abort"` a panic ends the process after the crash
///   report; restarts after a panic apply to `panic = "unwind"` builds.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use system_suite::{Member, System, runtime::{Backoff, Restart, Worker}};
///
/// # #[cfg(feature = "sys")] {
/// let system = System::builder("worker-doc")
///     .process_wide(false)
///     .single_instance(false)
///     .disable(Member::Log)
///     .start()?;
/// let worker = Worker::new("sweeper")
///     .restart(Restart::OnFailure)
///     .backoff(Backoff::exponential(Duration::from_millis(10), Duration::from_secs(1)))
///     .spawn(system.runtime(), |cx| {
///         while !cx.should_stop() {
///             cx.heartbeat();
///             cx.park(Some(Duration::from_millis(5)));
///         }
///         Ok(())
///     })?;
/// assert_eq!(worker.name(), "sweeper");
/// let code = system.shutdown();          // stops and joins the worker
/// # let _ = code;
/// # }
/// # Ok::<(), system_suite::Error>(())
/// ```
#[must_use = "a Worker does nothing until spawn()"]
pub struct Worker {
    name: Name,
    restart: Restart,
    backoff: Backoff,
    budget: (u32, Duration),
    critical: bool,
    heartbeat: Option<Duration>,
    stack_size: Option<usize>,
}

impl Worker {
    /// A worker named `name` (the application-name rule; unique among the
    /// system's workers).
    pub fn new(name: impl Into<std::borrow::Cow<'static, str>>) -> Worker {
        Worker {
            name: name.into(),
            restart: Restart::OnFailure,
            backoff: Backoff::default(),
            budget: (5, Duration::from_secs(60)),
            critical: false,
            heartbeat: None,
            stack_size: None,
        }
    }

    /// When to restart (default [`Restart::OnFailure`]).
    pub fn restart(mut self, policy: Restart) -> Self {
        self.restart = policy;
        self
    }

    /// How long to wait before a restart (default exponential, 100 ms to
    /// 30 s).
    pub fn backoff(mut self, backoff: Backoff) -> Self {
        self.backoff = backoff;
        self
    }

    /// At most `restarts` failures within `window` (default 5 within 60 s);
    /// one more marks the worker `Failed`.
    pub fn budget(mut self, restarts: u32, window: Duration) -> Self {
        self.budget = (restarts, window);
        self
    }

    /// Whether exhausting the budget stops the process (default `false`).
    pub fn critical(mut self, on: bool) -> Self {
        self.critical = on;
        self
    }

    /// Expect a heartbeat at least this often; three missed periods make
    /// the worker `Degraded` (default: no heartbeat expected).
    pub fn heartbeat(mut self, every: Duration) -> Self {
        self.heartbeat = Some(every);
        self
    }

    /// The thread's stack size (default: the platform's).
    pub fn stack_size(mut self, bytes: usize) -> Self {
        self.stack_size = Some(bytes);
        self
    }

    /// Starts the worker on its own thread.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Setup`] for an invalid or duplicate name or a zero
    /// budget window; [`ErrorKind::Stopping`] once a stop is under way;
    /// [`ErrorKind::Kernel`] if the OS refuses the thread.
    pub fn spawn<F>(self, runtime: &Runtime, run: F) -> Result<WorkerHandle, Error>
    where
        F: FnMut(&WorkerContext) -> Result<(), Failure> + Send + 'static,
    {
        runtime.check_accepting()?;
        check_name("worker", &self.name)?;
        if self.budget.1.is_zero() {
            return Err(Error::new(
                ErrorKind::Setup,
                format!("worker {}: the restart budget window is zero", self.name),
                "give budget() a window longer than zero",
            ));
        }
        let shared = Arc::new(Shared {
            name: self.name.clone(),
            state: AtomicU8::new(WorkerState::Running as u8),
            restarts: AtomicU32::new(0),
            stop: AtomicBool::new(false),
            started: Instant::now(),
            heartbeat_ms: Heartbeat::new(),
            heartbeat: self.heartbeat,
            critical: self.critical,
            thread: OnceLock::new(),
            join: Mutex::new(None),
            last_failure: Mutex::new(None),
            finished: Mutex::new(false),
            finished_cv: std::sync::Condvar::new(),
        });
        let handle = WorkerHandle {
            shared: Arc::clone(&shared),
        };
        {
            let mut workers = runtime
                .inner
                .workers
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // Workers that stopped are dropped from the list, so spawning
            // short-lived workers does not grow it; a failed worker stays
            // (it is part of health) but its name may be spawned again.
            workers.retain(|w| w.state() != WorkerState::Stopped);
            if workers.iter().any(|w| {
                w.name() == self.name
                    && matches!(w.state(), WorkerState::Running | WorkerState::BackingOff)
            }) {
                return Err(Error::new(
                    ErrorKind::Setup,
                    format!("a worker named {} is already running", self.name),
                    "give each worker its own name",
                ));
            }
            workers.push(handle.clone());
        }
        let cx = WorkerContext {
            shared: Arc::clone(&shared),
            runtime: Arc::clone(&runtime.inner),
        };
        let policy = Policy {
            restart: self.restart,
            backoff: self.backoff,
            budget: self.budget,
        };
        let mut builder = thread::Builder::new().name(format!("worker-{}", self.name));
        if let Some(bytes) = self.stack_size {
            builder = builder.stack_size(bytes);
        }
        let spawned = builder.spawn(move || supervise(&cx, &policy, run));
        match spawned {
            Ok(join) => {
                let _first = shared.thread.set(join.thread().clone()).is_ok();
                *shared.join.lock().unwrap_or_else(PoisonError::into_inner) = Some(join);
                Ok(handle)
            }
            Err(e) => {
                shared.set_state(WorkerState::Failed);
                shared.finish();
                Err(Error::new(
                    ErrorKind::Kernel,
                    format!("worker {}: the thread could not be started: {e}", self.name),
                    "check the process and thread limits",
                )
                .caused_by(e))
            }
        }
    }
}

struct Policy {
    restart: Restart,
    backoff: Backoff,
    budget: (u32, Duration),
}

/// The worker thread: run, and restart per the policy.
fn supervise<F>(cx: &WorkerContext, policy: &Policy, mut run: F)
where
    F: FnMut(&WorkerContext) -> Result<(), Failure>,
{
    let shared = &cx.shared;
    let mut failures: VecDeque<Instant> = VecDeque::new();
    loop {
        shared.set_state(WorkerState::Running);
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| run(cx)));
        let failure = match outcome {
            Ok(Ok(())) => None,
            Ok(Err(failure)) => Some(failure.message().to_owned()),
            Err(payload) => Some(panic_text(payload.as_ref())),
        };
        if cx.should_stop() {
            break;
        }
        let reason = match (failure, policy.restart) {
            (None, Restart::Always) => "returned; restarting (Restart::Always)".to_owned(),
            (None, _) => break,
            (Some(reason), Restart::Never) => {
                give_up(cx, reason);
                return;
            }
            (Some(reason), _) => {
                let now = Instant::now();
                failures.push_back(now);
                while failures
                    .front()
                    .is_some_and(|t| now.duration_since(*t) > policy.budget.1)
                {
                    let _old = failures.pop_front();
                }
                if failures.len() > usize::try_from(policy.budget.0).unwrap_or(usize::MAX) {
                    give_up(cx, reason);
                    return;
                }
                reason
            }
        };
        *shared
            .last_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(reason.clone());
        let attempt = shared.restarts.fetch_add(1, Ordering::AcqRel);
        cx.runtime.emit(Event::WorkerRestarted {
            name: shared.name.to_string(),
            restarts: attempt + 1,
            reason,
        });
        shared.set_state(WorkerState::BackingOff);
        let wake_at = Instant::now().checked_add(policy.backoff.delay(attempt));
        while let Some(wake_at) = wake_at {
            if cx.should_stop() {
                break;
            }
            let left = wake_at.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            thread::park_timeout(left);
        }
        if cx.should_stop() {
            break;
        }
    }
    shared.set_state(WorkerState::Stopped);
    shared.finish();
}

fn give_up(cx: &WorkerContext, reason: String) {
    let shared = &cx.shared;
    *shared
        .last_failure
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(reason.clone());
    shared.set_state(WorkerState::Failed);
    shared.finish();
    cx.runtime.emit(Event::WorkerFailed {
        name: shared.name.to_string(),
        critical: shared.critical,
        reason,
    });
}

/// The reason recorded for a panic. Not the payload: health reports are
/// served to whoever can reach the endpoint, and a panic message from std
/// or a dependency can quote data. The message itself is in the log (the
/// crash hook) and the crash report.
fn panic_text(_payload: &(dyn std::any::Any + Send)) -> String {
    "panicked (the message is in the log)".to_owned()
}

/// A worker's last heartbeat, in milliseconds since it started: written
/// only by the worker, read by health checks. Relaxed is enough because
/// one writer stores rising values and readers only compare them with the
/// clock; a reader never sees it go back (model-checked with loom, D-056).
struct Heartbeat(crate::sync::AtomicU64);

impl Heartbeat {
    fn new() -> Heartbeat {
        Heartbeat(crate::sync::AtomicU64::new(0))
    }

    #[inline]
    fn beat(&self, ms: u64) {
        self.0.store(ms, crate::sync::Ordering::Relaxed);
    }

    #[inline]
    fn last(&self) -> u64 {
        self.0.load(crate::sync::Ordering::Relaxed)
    }
}

struct Shared {
    name: Name,
    state: AtomicU8,
    restarts: AtomicU32,
    stop: AtomicBool,
    started: Instant,
    /// Milliseconds since `started`, written only by the worker.
    heartbeat_ms: Heartbeat,
    heartbeat: Option<Duration>,
    critical: bool,
    thread: OnceLock<Thread>,
    join: Mutex<Option<JoinHandle<()>>>,
    last_failure: Mutex<Option<String>>,
    finished: Mutex<bool>,
    finished_cv: std::sync::Condvar,
}

impl Shared {
    fn set_state(&self, state: WorkerState) {
        self.state.store(state as u8, Ordering::Release);
    }

    fn finish(&self) {
        *self.finished.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.finished_cv.notify_all();
    }

    fn wake(&self) {
        if let Some(thread) = self.thread.get() {
            thread.unpark();
        }
    }
}

/// What a worker's function receives: its name, its restart count, the
/// heartbeat, the stop signal, and a zero-CPU park.
pub struct WorkerContext {
    shared: Arc<Shared>,
    runtime: Arc<Inner>,
}

impl WorkerContext {
    /// The worker's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.shared.name
    }

    /// How many times the worker has been restarted.
    #[must_use]
    pub fn restarts(&self) -> u32 {
        self.shared.restarts.load(Ordering::Acquire)
    }

    /// Records that the worker is alive. One relaxed atomic store.
    pub fn heartbeat(&self) {
        let ms = u64::try_from(self.shared.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.shared.heartbeat_ms.beat(ms);
    }

    /// Whether the worker should return: it was asked to stop, the runtime
    /// is stopping, or the urgent line is at `Stopping`.
    #[must_use]
    pub fn should_stop(&self) -> bool {
        self.shared.stop.load(Ordering::Acquire)
            || self.runtime.stopping.load(Ordering::Acquire)
            || self.runtime.urgent.level() >= Level::Stopping
    }

    /// The urgent line.
    #[must_use]
    pub fn urgent(&self) -> &UrgentLine {
        &self.runtime.urgent
    }

    /// Parks the thread with zero CPU until it is woken
    /// ([`WorkerHandle::wake`]), asked to stop, or `timeout` passes. Returns
    /// at once when a stop is already requested. Like any park it may
    /// return early, so call it in a loop that checks its own condition.
    pub fn park(&self, timeout: Option<Duration>) {
        if self.should_stop() {
            return;
        }
        match timeout {
            Some(timeout) => thread::park_timeout(timeout),
            None => thread::park(),
        }
    }
}

impl fmt::Debug for WorkerContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkerContext")
            .field("name", &self.shared.name)
            .field("restarts", &self.restarts())
            .finish_non_exhaustive()
    }
}

/// A spawned worker. Cheap to clone; `Send + Sync`. Dropping it does not
/// stop the worker: the kernel stops every worker at shutdown.
#[derive(Clone)]
pub struct WorkerHandle {
    shared: Arc<Shared>,
}

impl WorkerHandle {
    /// The worker's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.shared.name
    }

    /// The worker's state.
    #[must_use]
    pub fn state(&self) -> WorkerState {
        WorkerState::from_u8(self.shared.state.load(Ordering::Acquire))
    }

    /// How many times the worker has been restarted.
    #[must_use]
    pub fn restarts(&self) -> u32 {
        self.shared.restarts.load(Ordering::Acquire)
    }

    /// Wakes the worker from [`WorkerContext::park`].
    pub fn wake(&self) {
        self.shared.wake();
    }

    /// Asks this worker to stop; does not wait.
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.wake();
    }

    /// The worker's health.
    pub(crate) fn health(&self) -> Health {
        let reason = || {
            self.shared
                .last_failure
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .unwrap_or_default()
        };
        match self.state() {
            WorkerState::Failed if self.shared.critical => {
                Health::unhealthy(format!("failed: {}", reason()))
            }
            WorkerState::Failed => Health::degraded(format!("failed: {}", reason())),
            WorkerState::BackingOff => Health::degraded(format!("restarting: {}", reason())),
            WorkerState::Stopped => Health::HEALTHY,
            WorkerState::Running => match self.shared.heartbeat {
                Some(every) => {
                    let silent = self.silent_for();
                    if silent > every.saturating_mul(3) {
                        Health::degraded(format!("no heartbeat for {} s", silent.as_secs()))
                    } else {
                        Health::HEALTHY
                    }
                }
                None => Health::HEALTHY,
            },
        }
    }

    /// The worker's status, without building a reason: no allocation.
    pub(crate) fn status(&self) -> crate::kernel::Status {
        use crate::kernel::Status;
        match self.state() {
            WorkerState::Failed if self.shared.critical => Status::Unhealthy,
            WorkerState::Failed | WorkerState::BackingOff => Status::Degraded,
            WorkerState::Stopped => Status::Healthy,
            WorkerState::Running => match self.shared.heartbeat {
                Some(every) if self.silent_for() > every.saturating_mul(3) => Status::Degraded,
                _ => Status::Healthy,
            },
        }
    }

    fn silent_for(&self) -> Duration {
        let last = Duration::from_millis(self.shared.heartbeat_ms.last());
        self.shared.started.elapsed().saturating_sub(last)
    }

    /// Waits until the worker's thread has ended, or `deadline` passes.
    /// Returns whether it ended.
    pub(crate) fn join_until(&self, deadline: Instant) -> bool {
        let mut finished = self
            .shared
            .finished
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while !*finished {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            finished = self
                .shared
                .finished_cv
                .wait_timeout(finished, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        drop(finished);
        let join = self
            .shared
            .join
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(join) = join {
            // The thread has finished its loop; a panic in it was already
            // caught and reported by the supervisor.
            let _ended = join.join();
        }
        true
    }
}

impl fmt::Debug for WorkerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkerHandle")
            .field("name", &self.shared.name)
            .field("state", &self.state())
            .field("restarts", &self.restarts())
            .finish()
    }
}

/// A worker's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum WorkerState {
    /// Its function is running.
    Running = 0,
    /// It failed and waits for its backoff before a restart.
    BackingOff = 1,
    /// It ended normally or was stopped.
    Stopped = 2,
    /// It exhausted its restart budget (or failed under `Restart::Never`).
    Failed = 3,
}

impl WorkerState {
    const fn from_u8(value: u8) -> WorkerState {
        match value {
            0 => WorkerState::Running,
            1 => WorkerState::BackingOff,
            2 => WorkerState::Stopped,
            _ => WorkerState::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::kernel::Status;

    fn runtime() -> Runtime {
        Runtime::new(UrgentLine::detached())
    }

    fn wait_for(handle: &WorkerHandle, state: WorkerState) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if handle.state() == state {
                return true;
            }
            thread::sleep(Duration::from_millis(2));
        }
        false
    }

    #[test]
    fn test_worker_that_returns_ok_stops() {
        let rt = runtime();
        let handle = Worker::new("once").spawn(&rt, |_| Ok(()));
        let handle = handle.ok();
        assert!(
            handle
                .as_ref()
                .is_some_and(|h| wait_for(h, WorkerState::Stopped))
        );
    }

    #[test]
    fn test_worker_restarts_after_failure_then_fails_past_its_budget() {
        let rt = runtime();
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&runs);
        let handle = Worker::new("flaky")
            .backoff(Backoff::None)
            .budget(3, Duration::from_secs(60))
            .spawn(&rt, move |_| {
                let _run = counter.fetch_add(1, Ordering::SeqCst);
                Err(Failure::new("boom"))
            })
            .ok();
        assert!(
            handle
                .as_ref()
                .is_some_and(|h| wait_for(h, WorkerState::Failed))
        );
        // One first run plus three restarts; the fourth failure exceeds the budget.
        assert_eq!(runs.load(Ordering::SeqCst), 4);
        assert_eq!(handle.as_ref().map(WorkerHandle::restarts), Some(3));
        assert_eq!(handle.map(|h| h.health().status()), Some(Status::Degraded));
    }

    #[test]
    fn test_worker_restarts_after_a_panic() {
        let rt = runtime();
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&runs);
        let handle = Worker::new("panicky")
            .backoff(Backoff::None)
            .spawn(&rt, move |_| {
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("first run fails");
                }
                Ok(())
            })
            .ok();
        assert!(
            handle
                .as_ref()
                .is_some_and(|h| wait_for(h, WorkerState::Stopped))
        );
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_restart_never_fails_on_the_first_error() {
        let rt = runtime();
        let handle = Worker::new("strict")
            .restart(Restart::Never)
            .critical(true)
            .spawn(&rt, |_| Err(Failure::new("no")))
            .ok();
        assert!(
            handle
                .as_ref()
                .is_some_and(|h| wait_for(h, WorkerState::Failed))
        );
        assert_eq!(handle.map(|h| h.health().status()), Some(Status::Unhealthy));
    }

    #[test]
    fn test_stop_ends_a_parked_worker_and_join_returns() {
        let rt = runtime();
        let handle = Worker::new("parked")
            .spawn(&rt, |cx| {
                while !cx.should_stop() {
                    cx.park(None);
                }
                Ok(())
            })
            .ok();
        thread::sleep(Duration::from_millis(20));
        let late = rt.stop(Instant::now() + Duration::from_secs(5));
        assert!(late.is_empty(), "{late:?}");
        assert_eq!(handle.map(|h| h.state()), Some(WorkerState::Stopped));
    }

    #[test]
    fn test_stop_reports_a_worker_that_ignores_it() {
        let rt = runtime();
        let _handle = Worker::new("stubborn").spawn(&rt, |_| {
            thread::sleep(Duration::from_secs(3_600));
            Ok(())
        });
        let late = rt.stop(Instant::now() + Duration::from_millis(100));
        assert_eq!(late, vec!["stubborn".to_owned()]);
    }

    #[test]
    fn test_wake_unparks_a_worker() {
        let rt = runtime();
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&woken);
        let handle = Worker::new("consumer")
            .spawn(&rt, move |cx| {
                while !cx.should_stop() {
                    cx.park(None);
                    let _seen = counter.fetch_add(1, Ordering::SeqCst);
                }
                Ok(())
            })
            .ok();
        thread::sleep(Duration::from_millis(20));
        let before = woken.load(Ordering::SeqCst);
        if let Some(handle) = &handle {
            handle.wake();
        }
        thread::sleep(Duration::from_millis(50));
        assert!(woken.load(Ordering::SeqCst) > before);
        let _late = rt.stop(Instant::now() + Duration::from_secs(5));
    }

    #[test]
    fn test_spawn_refuses_bad_and_duplicate_names_and_late_work() {
        let rt = runtime();
        assert!(Worker::new("bad name").spawn(&rt, |_| Ok(())).is_err());
        let _first = Worker::new("dup").spawn(&rt, |cx| {
            while !cx.should_stop() {
                cx.park(None);
            }
            Ok(())
        });
        let second = Worker::new("dup").spawn(&rt, |_| Ok(())).err();
        assert_eq!(second.map(|e| e.kind()), Some(ErrorKind::Setup));
        let _late = rt.stop(Instant::now() + Duration::from_secs(5));
        let after = Worker::new("late").spawn(&rt, |_| Ok(())).err();
        assert_eq!(after.map(|e| e.kind()), Some(ErrorKind::Stopping));
    }

    #[test]
    fn test_a_name_is_free_again_once_its_worker_stopped() {
        let rt = runtime();
        let first = Worker::new("again")
            .spawn(&rt, |_| Ok(()))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(wait_for(&first, WorkerState::Stopped));
        let second = Worker::new("again").spawn(&rt, |_| Ok(()));
        assert!(second.is_ok(), "a stopped worker's name stayed taken");
        let _late = rt.stop(Instant::now() + Duration::from_secs(5));
    }

    #[test]
    fn test_missing_heartbeats_degrade_the_worker() {
        let rt = runtime();
        let handle = Worker::new("silent")
            .heartbeat(Duration::from_millis(5))
            .spawn(&rt, |cx| {
                while !cx.should_stop() {
                    cx.park(None);
                }
                Ok(())
            })
            .ok();
        thread::sleep(Duration::from_millis(60));
        assert_eq!(handle.map(|h| h.health().status()), Some(Status::Degraded));
        let _late = rt.stop(Instant::now() + Duration::from_secs(5));
    }
}

/// Model tests: every interleaving of a heartbeat and its readers (D-056).
#[cfg(all(loom, test))]
mod loom_tests {
    use loom::{sync::Arc, thread};

    use super::Heartbeat;

    #[test]
    fn loom_a_reader_never_sees_a_heartbeat_go_back() {
        loom::model(|| {
            let beat = Arc::new(Heartbeat::new());
            let worker = {
                let beat = Arc::clone(&beat);
                thread::spawn(move || {
                    beat.beat(5);
                    beat.beat(9);
                })
            };
            let first = beat.last();
            let second = beat.last();
            assert!(second >= first);
            assert!(worker.join().is_ok());
            assert_eq!(beat.last(), 9);
        });
    }
}
