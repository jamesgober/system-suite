//! Running a shutdown step within its deadline.
//!
//! A thread cannot be killed safely, so a step that does not finish by its
//! deadline is abandoned: left running, reported, and the process exits
//! with code 79 (threat T6).

use std::{
    sync::{Arc, Mutex, PoisonError, mpsc},
    thread,
    time::{Duration, Instant},
};

/// How a shutdown step ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// It finished, in this long.
    Done(Duration),
    /// It panicked.
    Panicked,
    /// It was still running at the deadline and was left behind.
    Abandoned,
}

/// Runs `work` on its own thread and waits for it until `deadline`.
///
/// If the OS refuses a thread, the work runs on the calling thread without
/// a bound: running it late is better than not running it at all.
pub(crate) fn run_with_deadline<F>(name: &str, deadline: Instant, work: F) -> Outcome
where
    F: FnOnce() + Send + 'static,
{
    let started = Instant::now();
    let slot = Arc::new(Mutex::new(Some(work)));
    let (done, finished) = mpsc::channel::<()>();
    let thread_slot = Arc::clone(&slot);
    let spawned = thread::Builder::new()
        .name(format!("system-suite-stop-{name}"))
        .spawn(move || {
            let work = thread_slot
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(work) = work {
                work();
            }
            // The receiver may have given up at the deadline; nothing to do then.
            let _delivered = done.send(());
        });
    if spawned.is_err() {
        let work = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(work) = work {
            work();
        }
        return Outcome::Done(started.elapsed());
    }
    match finished.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(()) => Outcome::Done(started.elapsed()),
        Err(mpsc::RecvTimeoutError::Disconnected) => Outcome::Panicked,
        Err(mpsc::RecvTimeoutError::Timeout) => Outcome::Abandoned,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    fn after(duration: Duration) -> Instant {
        Instant::now() + duration
    }

    #[test]
    fn test_run_with_deadline_finishes_quick_work() {
        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        let outcome = run_with_deadline("quick", after(Duration::from_secs(5)), move || {
            flag.store(true, Ordering::SeqCst);
        });
        assert!(matches!(outcome, Outcome::Done(_)));
        assert!(ran.load(Ordering::SeqCst));
    }

    #[test]
    fn test_run_with_deadline_abandons_stuck_work() {
        // Threat T6: a step that never returns cannot hold the shutdown.
        let started = Instant::now();
        let outcome = run_with_deadline("stuck", after(Duration::from_millis(100)), || {
            thread::sleep(Duration::from_secs(3600));
        });
        assert_eq!(outcome, Outcome::Abandoned);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn test_run_with_deadline_reports_a_panic() {
        let outcome = run_with_deadline("panics", after(Duration::from_secs(5)), || {
            std::panic::panic_any("stop step failed");
        });
        assert_eq!(outcome, Outcome::Panicked);
    }

    #[test]
    fn test_run_with_deadline_past_deadline_abandons_slow_work() {
        let outcome = run_with_deadline("late", Instant::now(), || {
            thread::sleep(Duration::from_millis(200));
        });
        assert_eq!(outcome, Outcome::Abandoned);
    }
}
