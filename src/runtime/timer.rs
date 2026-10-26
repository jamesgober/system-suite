//! Timers on one kernel thread.
//!
//! A min-heap of due times (D-046): arming and firing are O(log n), a
//! cancel is O(1) (the entry is skipped when it comes due), and with nothing
//! due the thread waits with no timeout, so an idle system wakes for
//! nothing (NFR-2). The thread starts with the first timer.

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    fmt,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU8, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use super::{Event, Inner};
use crate::{Error, ErrorKind, kernel::Health};

/// The longest period or delay accepted.
const MAX_INTERVAL: Duration = Duration::from_secs(30 * 24 * 3600);

const ACTIVE: u8 = 0;
const FIRED: u8 = 1;
const CANCELLED: u8 = 2;

pub(crate) fn check_interval(name: &str, interval: Duration) -> Result<(), Error> {
    super::check_name("timer", name)?;
    if interval.is_zero() || interval > MAX_INTERVAL {
        return Err(Error::new(
            ErrorKind::Setup,
            format!("timer {name}: the interval must be more than zero and at most 30 days"),
            "pass an interval between 1 ms and 30 days",
        ));
    }
    Ok(())
}

/// A timer. Cheap to clone; every clone controls the same timer.
#[derive(Clone)]
pub struct TimerHandle {
    name: &'static str,
    state: Arc<AtomicU8>,
}

impl TimerHandle {
    /// The timer's name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Cancels the timer. Returns `false` if it already fired (one-shot) or
    /// was cancelled.
    pub fn cancel(&self) -> bool {
        self.state
            .compare_exchange(ACTIVE, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Whether the timer will still fire.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.state.load(Ordering::Acquire) == ACTIVE
    }
}

impl fmt::Debug for TimerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TimerHandle")
            .field("name", &self.name)
            .field("active", &self.is_active())
            .finish()
    }
}

enum Callback {
    Every(Box<dyn FnMut() + Send + 'static>),
    After(Box<dyn FnOnce() + Send + 'static>),
}

struct Entry {
    name: &'static str,
    period: Option<Duration>,
    callback: Option<Callback>,
    state: Arc<AtomicU8>,
}

#[derive(Default)]
struct State {
    due: BinaryHeap<Reverse<(Instant, u64)>>,
    entries: HashMap<u64, Entry>,
    next_id: u64,
    stopping: bool,
    panicked: Option<&'static str>,
}

/// Every timer of a runtime: a heap, one thread, one condition variable.
pub(crate) struct Timers {
    state: Mutex<State>,
    wake: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Timers {
    pub(crate) fn new() -> Timers {
        Timers {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            thread: Mutex::new(None),
        }
    }

    pub(crate) fn every(
        &self,
        name: &'static str,
        period: Duration,
        tick: Box<dyn FnMut() + Send + 'static>,
        runtime: Arc<Inner>,
    ) -> Result<TimerHandle, Error> {
        self.arm(name, period, Some(period), Callback::Every(tick), runtime)
    }

    pub(crate) fn after(
        &self,
        name: &'static str,
        delay: Duration,
        fire: Box<dyn FnOnce() + Send + 'static>,
        runtime: Arc<Inner>,
    ) -> Result<TimerHandle, Error> {
        self.arm(name, delay, None, Callback::After(fire), runtime)
    }

    fn arm(
        &self,
        name: &'static str,
        delay: Duration,
        period: Option<Duration>,
        callback: Callback,
        runtime: Arc<Inner>,
    ) -> Result<TimerHandle, Error> {
        self.ensure_thread(runtime)?;
        let state = Arc::new(AtomicU8::new(ACTIVE));
        let due = Instant::now()
            .checked_add(delay)
            .unwrap_or_else(Instant::now);
        let mut guard = self.lock();
        if guard.stopping {
            return Err(Error::new(
                ErrorKind::Stopping,
                "the timer thread no longer accepts timers: a stop is under way",
                "stop scheduling work; the process is going away",
            ));
        }
        let id = guard.next_id;
        guard.next_id = guard.next_id.wrapping_add(1);
        let _previous = guard.entries.insert(
            id,
            Entry {
                name,
                period,
                callback: Some(callback),
                state: Arc::clone(&state),
            },
        );
        guard.due.push(Reverse((due, id)));
        drop(guard);
        self.wake.notify_one();
        Ok(TimerHandle { name, state })
    }

    fn ensure_thread(&self, runtime: Arc<Inner>) -> Result<(), Error> {
        let mut thread = self.thread.lock().unwrap_or_else(PoisonError::into_inner);
        if thread.is_some() {
            return Ok(());
        }
        let spawned = thread::Builder::new()
            .name("system-suite-timer".to_owned())
            .spawn(move || run(&runtime));
        match spawned {
            Ok(handle) => {
                *thread = Some(handle);
                Ok(())
            }
            Err(e) => Err(Error::new(
                ErrorKind::Kernel,
                format!("the timer thread could not be started: {e}"),
                "check the process and thread limits",
            )
            .caused_by(e)),
        }
    }

    /// Stops the timer thread and waits for it until `deadline`. Returns
    /// whether it ended in time.
    pub(crate) fn stop(&self, deadline: Instant) -> bool {
        {
            let mut state = self.lock();
            state.stopping = true;
            // No timer fires after a stop, so none is reported active.
            for entry in state.entries.values() {
                let _was_active = entry.state.compare_exchange(
                    ACTIVE,
                    CANCELLED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
        }
        self.wake.notify_all();
        let handle = self
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(handle) = handle else {
            return true;
        };
        while !handle.is_finished() {
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
        // The thread finished its loop; callback panics were caught there.
        let _ended = handle.join();
        true
    }

    /// The timers' health: degraded once a callback has panicked.
    pub(crate) fn health(&self) -> Health {
        match self.lock().panicked {
            Some(name) => Health::degraded(format!("timer {name} panicked and was cancelled")),
            None => Health::HEALTHY,
        }
    }

    /// The timers' status, without building a reason: no allocation.
    pub(crate) fn status(&self) -> crate::kernel::Status {
        if self.lock().panicked.is_some() {
            crate::kernel::Status::Degraded
        } else {
            crate::kernel::Status::Healthy
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Every update under this lock is a few field writes that cannot be
        // left half-done; callbacks run outside it.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The timer thread.
fn run(runtime: &Inner) {
    let timers = &runtime.timers;
    let mut state = timers.lock();
    loop {
        if state.stopping {
            return;
        }
        let Some(&Reverse((due, id))) = state.due.peek() else {
            state = timers
                .wake
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
            continue;
        };
        let now = Instant::now();
        if due > now {
            state = timers
                .wake
                .wait_timeout(state, due - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            continue;
        }
        let _popped = state.due.pop();
        let Some(entry) = state.entries.get_mut(&id) else {
            continue;
        };
        if entry.state.load(Ordering::Acquire) != ACTIVE {
            let _removed = state.entries.remove(&id);
            continue;
        }
        let name = entry.name;
        let period = entry.period;
        let callback = entry.callback.take();
        let handle_state = Arc::clone(&entry.state);
        drop(state);
        let (callback, panicked) = fire(callback);
        state = timers.lock();
        if panicked {
            handle_state.store(CANCELLED, Ordering::Release);
            let _removed = state.entries.remove(&id);
            state.panicked = Some(name);
            drop(state);
            runtime.emit(Event::TimerPanicked { name });
            state = timers.lock();
            continue;
        }
        match (period, callback) {
            (Some(period), Some(callback)) if handle_state.load(Ordering::Acquire) == ACTIVE => {
                // Missed ticks are skipped, not replayed in a burst.
                let now = Instant::now();
                let mut next = due.checked_add(period).unwrap_or(now);
                if next <= now {
                    next = now.checked_add(period).unwrap_or(now);
                }
                if let Some(entry) = state.entries.get_mut(&id) {
                    entry.callback = Some(callback);
                }
                state.due.push(Reverse((next, id)));
            }
            _ => {
                let _fired = handle_state.compare_exchange(
                    ACTIVE,
                    FIRED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                let _removed = state.entries.remove(&id);
            }
        }
    }
}

/// Runs a callback and catches a panic. A periodic callback is handed back
/// to be scheduled again.
fn fire(callback: Option<Callback>) -> (Option<Callback>, bool) {
    match callback {
        Some(Callback::Every(mut tick)) => {
            let ok = panic::catch_unwind(AssertUnwindSafe(&mut tick)).is_ok();
            (Some(Callback::Every(tick)), !ok)
        }
        Some(Callback::After(fire)) => {
            let ok = panic::catch_unwind(AssertUnwindSafe(fire)).is_ok();
            (None, !ok)
        }
        None => (None, false),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::{
        kernel::{Status, UrgentLine},
        runtime::Runtime,
    };

    fn runtime() -> Runtime {
        Runtime::new(UrgentLine::detached())
    }

    fn eventually(check: impl Fn() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if check() {
                return true;
            }
            thread::sleep(Duration::from_millis(2));
        }
        false
    }

    #[test]
    fn test_every_fires_repeatedly_until_cancelled() {
        let rt = runtime();
        let ticks = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&ticks);
        let handle = rt.every("tick", Duration::from_millis(2), move || {
            let _n = counter.fetch_add(1, Ordering::SeqCst);
        });
        assert!(eventually(|| ticks.load(Ordering::SeqCst) >= 3));
        assert!(handle.as_ref().is_ok_and(TimerHandle::cancel));
        let after_cancel = ticks.load(Ordering::SeqCst);
        thread::sleep(Duration::from_millis(30));
        assert!(ticks.load(Ordering::SeqCst) <= after_cancel + 1);
        assert!(
            rt.inner
                .timers
                .stop(Instant::now() + Duration::from_secs(5))
        );
    }

    #[test]
    fn test_after_fires_once_then_is_inactive() {
        let rt = runtime();
        let fired = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&fired);
        let handle = rt
            .after("once", Duration::from_millis(5), move || {
                let _n = counter.fetch_add(1, Ordering::SeqCst);
            })
            .ok();
        assert!(eventually(|| fired.load(Ordering::SeqCst) == 1));
        assert!(eventually(|| handle
            .as_ref()
            .is_some_and(|h| !h.is_active())));
        assert!(handle.is_some_and(|h| !h.cancel()));
        thread::sleep(Duration::from_millis(20));
        assert_eq!(fired.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_cancelled_after_never_fires() {
        let rt = runtime();
        let fired = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&fired);
        let handle = rt
            .after("never", Duration::from_millis(30), move || {
                let _n = counter.fetch_add(1, Ordering::SeqCst);
            })
            .ok();
        assert!(handle.is_some_and(|h| h.cancel()));
        thread::sleep(Duration::from_millis(60));
        assert_eq!(fired.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_timers_fire_in_due_order() {
        let rt = runtime();
        let order = Arc::new(Mutex::new(Vec::new()));
        for (name, ms) in [("c", 30_u64), ("a", 10), ("b", 20)] {
            let order = Arc::clone(&order);
            let _handle = rt.after(name, Duration::from_millis(ms), move || {
                order
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(name);
            });
        }
        assert!(eventually(|| order
            .lock()
            .map(|o| o.len() == 3)
            .unwrap_or(false)));
        let seen = order.lock().map(|o| o.clone()).unwrap_or_default();
        assert_eq!(seen, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_a_panicking_timer_is_cancelled_and_degrades_health() {
        let rt = runtime();
        let handle = rt
            .every("bad", Duration::from_millis(2), || {
                panic!("timer callback failed")
            })
            .ok();
        assert!(eventually(
            || rt.inner.timers.health().status() == Status::Degraded
        ));
        assert!(handle.is_some_and(|h| !h.is_active()));
    }

    #[test]
    fn test_intervals_are_checked() {
        let rt = runtime();
        assert!(rt.every("zero", Duration::ZERO, || {}).is_err());
        assert!(
            rt.after("huge", Duration::from_secs(31 * 24 * 3600), || {})
                .is_err()
        );
        assert!(rt.after("bad name", Duration::from_secs(1), || {}).is_err());
    }

    #[test]
    fn test_no_timers_starts_no_thread() {
        let rt = runtime();
        assert!(
            rt.inner
                .timers
                .thread
                .lock()
                .map(|t| t.is_none())
                .unwrap_or(false)
        );
        assert!(rt.inner.timers.stop(Instant::now()));
    }
}
