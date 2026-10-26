//! The urgent line: how the kernel tells the whole process "shed load",
//! "lock down", "drain", or "stop" now, without an event loop in the way.
//!
//! Tier T0 is one cache-line-aligned level and epoch that every worker
//! reads at its checkpoints. Tier T1 is each subsystem's [`Mailbox`],
//! poked and woken directly. Tier T2 (logs, metrics) is the observer the
//! kernel installs.

use std::{
    fmt,
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
};

use crate::{Error, ErrorKind, lifecycle::Mailbox};

/// How urgent things are, process-wide. Values compare by urgency.
///
/// # Examples
///
/// ```
/// use system_suite::kernel::Level;
///
/// assert!(Level::Draining > Level::Lockdown);
/// assert_eq!(Level::Shed.to_string(), "shed");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum Level {
    /// Normal behaviour.
    Normal = 0,
    /// Refuse low-priority work (memory pressure, a supervisor's request).
    Shed = 1,
    /// Strict policies only (the security package, when present).
    Lockdown = 2,
    /// A stop was requested: accept no new work, finish what is in flight.
    Draining = 3,
    /// Stop now.
    Stopping = 4,
}

impl Level {
    /// The level's name, as used in logs and health reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Level::Normal => "normal",
            Level::Shed => "shed",
            Level::Lockdown => "lockdown",
            Level::Draining => "draining",
            Level::Stopping => "stopping",
        }
    }

    const fn from_u8(value: u8) -> Level {
        match value {
            0 => Level::Normal,
            1 => Level::Shed,
            2 => Level::Lockdown,
            3 => Level::Draining,
            _ => Level::Stopping,
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One change of the urgent line: the new level, the epoch it was given,
/// and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Notice {
    /// The level after the change.
    pub level: Level,
    /// The epoch after the change; it grows by one per change.
    pub epoch: u64,
    /// Why the level changed.
    pub reason: &'static str,
}

/// What the kernel does with each change: log, count, publish.
pub(crate) type Observer = Box<dyn Fn(Notice) + Send + Sync + 'static>;

/// The urgent line's handle. Cheap to clone; `Send + Sync`.
///
/// [`set`](Self::set) moves the level among `Normal`, `Shed`, and
/// `Lockdown`. `Draining` and `Stopping` belong to the kernel (a stop
/// request), and once a stop is requested the line only moves forward.
///
/// # Examples
///
/// ```
/// use system_suite::kernel::{Level, UrgentLine};
///
/// let line = UrgentLine::detached();
/// let mut watch = line.watch();
/// assert_eq!(watch.changed(), None);             // about a nanosecond
/// line.set(Level::Shed, "memory above 90%")?;
/// assert_eq!(watch.changed(), Some(Level::Shed));
/// assert_eq!(watch.changed(), None);
/// # Ok::<(), system_suite::Error>(())
/// ```
#[derive(Clone)]
pub struct UrgentLine {
    line: Arc<Line>,
}

struct Line {
    word: Word,
    reason: Mutex<&'static str>,
    mailboxes: Mutex<Vec<Mailbox>>,
    observer: OnceLock<Observer>,
}

/// The level and epoch on their own cache line, so a worker's checkpoint
/// read never shares a line with anything that is written often.
#[repr(align(64))]
struct Word {
    level: AtomicU8,
    epoch: AtomicU64,
}

impl UrgentLine {
    /// A line no kernel drives, for tests and for code that wants the same
    /// checkpoint pattern outside a system. A running system's line is
    /// `Kernel::urgent()`.
    #[must_use]
    pub fn detached() -> UrgentLine {
        UrgentLine {
            line: Arc::new(Line {
                word: Word {
                    level: AtomicU8::new(Level::Normal as u8),
                    epoch: AtomicU64::new(0),
                },
                reason: Mutex::new("start"),
                mailboxes: Mutex::new(Vec::new()),
                observer: OnceLock::new(),
            }),
        }
    }

    /// The current level. One atomic load.
    #[inline]
    #[must_use]
    pub fn level(&self) -> Level {
        Level::from_u8(self.line.word.level.load(Ordering::Acquire))
    }

    /// The current epoch: how many changes the line has seen. One atomic
    /// load.
    #[inline]
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.line.word.epoch.load(Ordering::Acquire)
    }

    /// The current level, epoch, and reason.
    #[must_use]
    pub fn notice(&self) -> Notice {
        let reason = *self
            .line
            .reason
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Notice {
            level: self.level(),
            epoch: self.epoch(),
            reason,
        }
    }

    /// Changes the level to `Normal`, `Shed`, or `Lockdown` and returns the
    /// new epoch. Every subsystem's mailbox is poked and the change is
    /// logged and counted.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::UrgentRefused`] for `Draining` or `Stopping` (use a stop
    /// request), and for any change once a stop was requested.
    pub fn set(&self, level: Level, reason: &'static str) -> Result<u64, Error> {
        if level >= Level::Draining {
            return Err(Error::new(
                ErrorKind::UrgentRefused,
                format!("the urgent line cannot be set to {level} directly"),
                "request a stop instead (Kernel::request_stop)",
            ));
        }
        let word = &self.line.word;
        let mut current = word.level.load(Ordering::Acquire);
        loop {
            if current >= Level::Draining as u8 {
                return Err(Error::new(
                    ErrorKind::UrgentRefused,
                    "the urgent line cannot change once a stop was requested",
                    "nothing: the process is going away",
                ));
            }
            match word.level.compare_exchange_weak(
                current,
                level as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        Ok(self.announce(reason))
    }

    /// Raises the line to `level` (never lowers it): the kernel's way to
    /// `Draining` and `Stopping`. Returns `false` if it was already there.
    pub(crate) fn raise(&self, level: Level, reason: &'static str) -> bool {
        let before = self
            .line
            .word
            .level
            .fetch_max(level as u8, Ordering::AcqRel);
        if before >= level as u8 {
            return false;
        }
        let _epoch = self.announce(reason);
        true
    }

    /// Records the reason, bumps the epoch, and delivers the notice to
    /// every mailbox and the observer.
    fn announce(&self, reason: &'static str) -> u64 {
        *self
            .line
            .reason
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = reason;
        let epoch = self.line.word.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let notice = Notice {
            level: self.level(),
            epoch,
            reason,
        };
        let mailboxes = self
            .line
            .mailboxes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for mailbox in &mailboxes {
            mailbox.post(notice);
        }
        if let Some(observer) = self.line.observer.get() {
            observer(notice);
        }
        epoch
    }

    /// A checkpoint helper that remembers the last epoch it saw.
    #[must_use]
    pub fn watch(&self) -> Watch {
        Watch {
            line: self.clone(),
            seen: self.epoch(),
        }
    }

    /// Delivers every future change to `mailbox` (tier T1).
    pub(crate) fn subscribe(&self, mailbox: Mailbox) {
        self.line
            .mailboxes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(mailbox);
    }

    /// Installs what the kernel does with each change. Only the first call
    /// counts.
    pub(crate) fn observe(&self, observer: Observer) {
        // A second observer is ignored; the kernel installs exactly one.
        let _first = self.line.observer.set(observer).is_ok();
    }
}

impl fmt::Debug for UrgentLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UrgentLine")
            .field("level", &self.level())
            .field("epoch", &self.epoch())
            .finish_non_exhaustive()
    }
}

/// A checkpoint on the urgent line: [`changed`](Self::changed) returns the
/// new level once per change, and costs about a nanosecond when nothing
/// changed [est.].
#[derive(Clone, Debug)]
pub struct Watch {
    line: UrgentLine,
    seen: u64,
}

impl Watch {
    /// The new level if the line changed since the last call, else `None`.
    #[inline]
    pub fn changed(&mut self) -> Option<Level> {
        let epoch = self.line.epoch();
        if epoch == self.seen {
            return None;
        }
        self.seen = epoch;
        Some(self.line.level())
    }

    /// The current level.
    #[must_use]
    pub fn level(&self) -> Level {
        self.line.level()
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::atomic::AtomicUsize, thread, time::Duration};

    use super::*;

    #[test]
    fn test_set_moves_among_normal_shed_lockdown() {
        let line = UrgentLine::detached();
        assert_eq!(line.level(), Level::Normal);
        assert_eq!(line.set(Level::Shed, "test").ok(), Some(1));
        assert_eq!(line.set(Level::Lockdown, "test").ok(), Some(2));
        assert_eq!(line.set(Level::Normal, "clear").ok(), Some(3));
        assert_eq!(line.notice().reason, "clear");
    }

    #[test]
    fn test_set_refuses_stop_levels() {
        let line = UrgentLine::detached();
        for level in [Level::Draining, Level::Stopping] {
            let error = line.set(level, "test").err();
            assert_eq!(error.map(|e| e.kind()), Some(ErrorKind::UrgentRefused));
        }
        assert_eq!(line.epoch(), 0);
    }

    #[test]
    fn test_raise_only_moves_forward_and_blocks_set() {
        let line = UrgentLine::detached();
        assert!(line.raise(Level::Draining, "stop"));
        assert!(!line.raise(Level::Draining, "again"));
        assert!(line.set(Level::Shed, "late").is_err());
        assert!(line.raise(Level::Stopping, "deadline"));
        assert!(!line.raise(Level::Draining, "backwards"));
        assert_eq!(line.level(), Level::Stopping);
    }

    #[test]
    fn test_watch_reports_each_change_once() {
        let line = UrgentLine::detached();
        let mut watch = line.watch();
        assert_eq!(watch.changed(), None);
        let _epoch = line.set(Level::Shed, "x");
        assert_eq!(watch.changed(), Some(Level::Shed));
        assert_eq!(watch.changed(), None);
    }

    #[test]
    fn test_mailboxes_and_observer_receive_every_change() {
        let line = UrgentLine::detached();
        let mailbox = Mailbox::new();
        line.subscribe(mailbox.clone());
        let seen = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&seen);
        line.observe(Box::new(move |_| {
            let _count = counter.fetch_add(1, Ordering::SeqCst);
        }));
        let _epoch = line.set(Level::Lockdown, "attack");
        assert_eq!(mailbox.try_take().map(|n| n.level), Some(Level::Lockdown));
        assert_eq!(seen.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_a_parked_mailbox_wakes_on_a_stop() {
        let line = UrgentLine::detached();
        let mailbox = Mailbox::new();
        line.subscribe(mailbox.clone());
        let waiter = thread::spawn(move || mailbox.wait_timeout(Duration::from_secs(10)));
        thread::sleep(Duration::from_millis(20));
        assert!(line.raise(Level::Draining, "stop"));
        let notice = waiter.join().ok().flatten();
        assert_eq!(notice.map(|n| n.level), Some(Level::Draining));
    }

    #[test]
    fn test_concurrent_sets_never_undo_a_stop() {
        let line = UrgentLine::detached();
        let setters: Vec<_> = (0..4)
            .map(|_| {
                let line = line.clone();
                thread::spawn(move || {
                    for _ in 0..1_000 {
                        let _result = line.set(Level::Shed, "load");
                        let _result = line.set(Level::Normal, "clear");
                    }
                })
            })
            .collect();
        assert!(line.raise(Level::Draining, "stop"));
        for setter in setters {
            assert!(setter.join().is_ok());
        }
        assert_eq!(line.level(), Level::Draining);
    }

    #[test]
    fn test_word_has_its_own_cache_line() {
        assert_eq!(std::mem::align_of::<Word>(), 64);
    }
}
