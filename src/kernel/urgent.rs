//! The urgent line: how the kernel tells the whole process "shed load",
//! "lock down", "drain", or "stop" now, without an event loop in the way.
//!
//! Tier T0 is one cache-line-aligned level and epoch that every worker
//! reads at its checkpoints. Tier T1 is each subsystem's [`Mailbox`],
//! poked and woken directly. Tier T2 (logs, metrics) is the observer the
//! kernel installs.

use std::{
    borrow::Cow,
    fmt,
    sync::{Arc, Mutex, OnceLock, PoisonError},
};

use crate::{
    Error, ErrorKind,
    lifecycle::Mailbox,
    sync::{AtomicU8, AtomicU64, Ordering},
};

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

/// One change of the urgent line: the new level, the level before it, the
/// epoch it was given, and why. More fields may come (a drain deadline);
/// it is built only by the line.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Notice {
    /// The level after the change.
    pub level: Level,
    /// The level before the change.
    pub previous: Level,
    /// The epoch after the change; it grows by one per change.
    pub epoch: u64,
    /// Why the level changed: a literal costs nothing, owned text (a
    /// security finding, an operator's words) one allocation.
    pub reason: Cow<'static, str>,
}

impl Notice {
    /// A notice for tests and for code that wants the same pattern outside
    /// a line.
    #[must_use]
    pub fn new(
        level: Level,
        previous: Level,
        epoch: u64,
        reason: impl Into<Cow<'static, str>>,
    ) -> Notice {
        Notice {
            level,
            previous,
            epoch,
            reason: reason.into(),
        }
    }
}

/// What the kernel does with each change: log, count, publish.
pub(crate) type Observer = Box<dyn Fn(&Notice) + Send + Sync + 'static>;

/// The urgent line's handle. Cheap to clone; `Send + Sync`.
///
/// [`set`](Self::set) raises the level to `Shed` or `Lockdown`;
/// [`shift`](Self::shift) moves it from one named level to another, which
/// is the only way down (D-084). `Draining` and `Stopping` belong to the
/// kernel (a stop request), and once a stop is requested the line only
/// moves forward.
///
/// # Examples
///
/// ```
/// use system_suite::kernel::{Level, UrgentLine};
///
/// let line = UrgentLine::detached();
/// let mut watch = line.watch();
/// assert_eq!(watch.changed(), None);             // about a nanosecond
/// line.set(Level::Shed, "upstream is failing")?;
/// assert_eq!(watch.changed(), Some(Level::Shed));
/// assert_eq!(watch.changed(), None);
/// assert!(line.set(Level::Normal, "too early").is_err()); // set never lowers
/// line.shift(Level::Shed, Level::Normal, "upstream recovered")?;
/// # Ok::<(), system_suite::Error>(())
/// ```
#[derive(Clone)]
pub struct UrgentLine {
    line: Arc<Line>,
}

struct Line {
    word: Word,
    reason: Mutex<(Level, Cow<'static, str>)>,
    mailboxes: Mutex<Vec<Mailbox>>,
    observer: OnceLock<Observer>,
}

/// The level and epoch on their own cache line, so a worker's checkpoint
/// read never shares a line with anything that is written often.
///
/// A change writes the level first and then bumps the epoch (both
/// acquire-release), so a reader that sees an epoch sees that change's
/// level or a later one. Once the level reaches `Draining` it only moves
/// forward. Both properties are model-checked with loom (D-056).
#[repr(align(64))]
struct Word {
    level: AtomicU8,
    epoch: AtomicU64,
}

impl Word {
    fn new() -> Word {
        Word {
            level: AtomicU8::new(Level::Normal as u8),
            epoch: AtomicU64::new(0),
        }
    }

    #[inline]
    fn level(&self) -> u8 {
        self.level.load(Ordering::Acquire)
    }

    #[inline]
    fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Moves the level up to `level`, or keeps it at `level`, unless that
    /// would lower it. Returns the level before, or the level that stopped
    /// it (a higher one, or a stop).
    fn try_raise(&self, level: u8) -> Result<u8, u8> {
        let mut current = self.level.load(Ordering::Acquire);
        loop {
            if current >= Level::Draining as u8 || current > level {
                return Err(current);
            }
            match self.level.compare_exchange_weak(
                current,
                level,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(current),
                Err(actual) => current = actual,
            }
        }
    }

    /// Raises the level to `level`, never lowering it. Returns the level
    /// before.
    fn raise(&self, level: u8) -> u8 {
        self.level.fetch_max(level, Ordering::AcqRel)
    }

    /// Counts a change and returns the new epoch.
    fn bump(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    }
}

impl UrgentLine {
    /// A line no kernel drives, for tests and for code that wants the same
    /// checkpoint pattern outside a system. A running system's line is
    /// `Kernel::urgent()`.
    #[must_use]
    pub fn detached() -> UrgentLine {
        UrgentLine {
            line: Arc::new(Line {
                word: Word::new(),
                reason: Mutex::new((Level::Normal, Cow::Borrowed("start"))),
                mailboxes: Mutex::new(Vec::new()),
                observer: OnceLock::new(),
            }),
        }
    }

    /// The current level. One atomic load.
    #[inline]
    #[must_use]
    pub fn level(&self) -> Level {
        Level::from_u8(self.line.word.level())
    }

    /// The current epoch: how many changes the line has seen. One atomic
    /// load.
    #[inline]
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.line.word.epoch()
    }

    /// The current level, epoch, and reason (the last change's).
    #[must_use]
    pub fn notice(&self) -> Notice {
        let (previous, reason) = self
            .line
            .reason
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Notice {
            level: self.level(),
            previous,
            epoch: self.epoch(),
            reason,
        }
    }

    /// Raises the level to `Shed` or `Lockdown` (or announces the current
    /// level again with a new reason) and returns the new epoch. Every
    /// subsystem's mailbox is poked and the change is logged and counted.
    /// It never lowers the level: a `Lockdown` set by the security package
    /// cannot be undone by a `set(Normal)` from anywhere else (D-084).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::UrgentRefused`] for `Draining` or `Stopping` (use a stop
    /// request), for a level below the current one (use
    /// [`shift`](Self::shift)), and for any change once a stop was
    /// requested.
    pub fn set(&self, level: Level, reason: impl Into<Cow<'static, str>>) -> Result<u64, Error> {
        if level >= Level::Draining {
            return Err(Error::new(
                ErrorKind::UrgentRefused,
                format!("the urgent line cannot be set to {level} directly"),
                "request a stop instead (Kernel::request_stop)",
            ));
        }
        match self.line.word.try_raise(level as u8) {
            Ok(previous) => Ok(self.announce(Level::from_u8(previous), reason.into())),
            Err(current) if current >= Level::Draining as u8 => Err(stopping()),
            Err(current) => Err(Error::new(
                ErrorKind::UrgentRefused,
                format!(
                    "the urgent line is at {}; set never lowers it",
                    Level::from_u8(current)
                ),
                "lower it with shift(from, to), naming the level it is at",
            )),
        }
    }

    /// Moves the line from exactly `from` to `to` (both `Normal`, `Shed`, or
    /// `Lockdown`) and returns the new epoch: the way to lower the level,
    /// and to change it without overriding what someone else set since.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::UrgentRefused`] when `from` or `to` is `Draining` or
    /// `Stopping`, or when the line is not at `from` (it names the level
    /// it is at).
    pub fn shift(
        &self,
        from: Level,
        to: Level,
        reason: impl Into<Cow<'static, str>>,
    ) -> Result<u64, Error> {
        if from >= Level::Draining || to >= Level::Draining {
            return Err(Error::new(
                ErrorKind::UrgentRefused,
                "shift moves only among normal, shed, and lockdown",
                "request a stop instead (Kernel::request_stop)",
            ));
        }
        match self.line.word.level.compare_exchange(
            from as u8,
            to as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(self.announce(from, reason.into())),
            Err(current) if current >= Level::Draining as u8 => Err(stopping()),
            Err(current) => Err(Error::new(
                ErrorKind::UrgentRefused,
                format!(
                    "the urgent line is at {}, not {from}",
                    Level::from_u8(current)
                ),
                "read the level again and decide with what is there now",
            )),
        }
    }

    /// Raises the line to `level` (never lowers it): the kernel's way to
    /// `Draining` and `Stopping`. Returns `false` if it was already there.
    pub(crate) fn raise(&self, level: Level, reason: &'static str) -> bool {
        let previous = self.line.word.raise(level as u8);
        if previous >= level as u8 {
            return false;
        }
        let _epoch = self.announce(Level::from_u8(previous), Cow::Borrowed(reason));
        true
    }

    /// Records the reason, bumps the epoch, and delivers the notice to
    /// every mailbox and the observer.
    fn announce(&self, previous: Level, reason: Cow<'static, str>) -> u64 {
        *self
            .line
            .reason
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = (previous, reason.clone());
        let epoch = self.line.word.bump();
        let notice = Notice {
            level: self.level(),
            previous,
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
            mailbox.post(notice.clone());
        }
        if let Some(observer) = self.line.observer.get() {
            observer(&notice);
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

/// The refusal for any change once a stop was requested.
fn stopping() -> Error {
    Error::new(
        ErrorKind::UrgentRefused,
        "the urgent line cannot change once a stop was requested",
        "nothing: the process is going away",
    )
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
        assert_eq!(
            line.shift(Level::Lockdown, Level::Normal, "clear").ok(),
            Some(3)
        );
        assert_eq!(line.notice().previous, Level::Lockdown);
        assert_eq!(line.notice().reason, "clear");
    }

    #[test]
    fn test_set_never_lowers_and_shift_needs_the_level_named() {
        let line = UrgentLine::detached();
        assert!(line.set(Level::Lockdown, "breach").is_ok());
        for level in [Level::Normal, Level::Shed] {
            let error = line.set(level, "sneak").err();
            assert_eq!(error.map(|e| e.kind()), Some(ErrorKind::UrgentRefused));
        }
        assert_eq!(line.level(), Level::Lockdown);
        // The same level again is a new reason, not a change of level.
        assert_eq!(
            line.set(Level::Lockdown, "still under attack").ok(),
            Some(2)
        );
        // shift needs the level the line is at.
        let wrong = line.shift(Level::Shed, Level::Normal, "guess").err();
        assert_eq!(wrong.map(|e| e.kind()), Some(ErrorKind::UrgentRefused));
        assert_eq!(line.level(), Level::Lockdown);
        assert!(
            line.shift(Level::Lockdown, Level::Shed, "contained")
                .is_ok()
        );
        assert_eq!(line.level(), Level::Shed);
        let stop = line.shift(Level::Shed, Level::Draining, "no");
        assert!(stop.is_err());
    }

    #[test]
    fn test_an_owned_reason_reaches_the_notice() {
        let line = UrgentLine::detached();
        let source = String::from("10.0.0.7");
        assert!(
            line.set(Level::Lockdown, format!("scan from {source}"))
                .is_ok()
        );
        assert_eq!(line.notice().reason, "scan from 10.0.0.7");
        assert_eq!(line.notice().previous, Level::Normal);
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
                        let _result = line.shift(Level::Shed, Level::Normal, "clear");
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

/// Model tests: every interleaving of the urgent line's word (D-056).
#[cfg(all(loom, test))]
mod loom_tests {
    use loom::{sync::Arc, thread};

    use super::{Level, Word};

    #[test]
    fn loom_a_stop_is_never_undone_by_a_concurrent_set() {
        loom::model(|| {
            let word = Arc::new(Word::new());
            let setter = {
                let word = Arc::clone(&word);
                thread::spawn(move || {
                    if word.try_raise(Level::Shed as u8).is_ok() {
                        let _epoch = word.bump();
                    }
                })
            };
            if word.raise(Level::Draining as u8) < Level::Draining as u8 {
                let _epoch = word.bump();
            }
            assert!(setter.join().is_ok());
            assert!(word.level() >= Level::Draining as u8);
        });
    }

    #[test]
    fn loom_a_lockdown_is_never_lowered_by_a_concurrent_set() {
        loom::model(|| {
            let word = Arc::new(Word::new());
            let shedder = {
                let word = Arc::clone(&word);
                thread::spawn(move || {
                    let _moved = word.try_raise(Level::Shed as u8);
                })
            };
            assert!(word.try_raise(Level::Lockdown as u8).is_ok());
            assert!(shedder.join().is_ok());
            assert_eq!(word.level(), Level::Lockdown as u8);
        });
    }

    #[test]
    fn loom_a_reader_sees_the_level_of_the_epoch_it_read() {
        loom::model(|| {
            let word = Arc::new(Word::new());
            let writer = {
                let word = Arc::clone(&word);
                thread::spawn(move || {
                    assert!(word.try_raise(Level::Shed as u8).is_ok());
                    let _epoch = word.bump();
                })
            };
            let epoch = word.epoch();
            let level = word.level();
            if epoch == 1 {
                assert_eq!(level, Level::Shed as u8);
            }
            assert!(writer.join().is_ok());
        });
    }

    #[test]
    fn loom_concurrent_raises_count_each_rise_once() {
        loom::model(|| {
            let word = Arc::new(Word::new());
            let draining = {
                let word = Arc::clone(&word);
                thread::spawn(move || {
                    if word.raise(Level::Draining as u8) < Level::Draining as u8 {
                        let _epoch = word.bump();
                    }
                })
            };
            let rose = word.raise(Level::Stopping as u8) < Level::Stopping as u8;
            if rose {
                let _epoch = word.bump();
            }
            assert!(draining.join().is_ok());
            assert!(rose);
            assert_eq!(word.level(), Level::Stopping as u8);
            assert!((1..=2).contains(&word.epoch()));
        });
    }
}
