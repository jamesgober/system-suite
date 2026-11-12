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

    /// Raises the level to `Shed` or `Lockdown` and returns the new epoch.
    /// Every subsystem's mailbox is poked and the change is logged and
    /// counted. It never lowers the level: a `Lockdown` set by the security
    /// package cannot be undone by a `set(Normal)` from anywhere else
    /// (D-084). Setting the level the line is already at changes nothing
    /// and returns the current epoch, so a caller that sets `Lockdown` on
    /// every bad request costs one atomic read after the first (D-089).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::UrgentRefused`] for `Draining` or `Stopping` (use a stop
    /// request), for a level below the current one (use
    /// [`shift`](Self::shift)), for any change once a stop was requested,
    /// and for a change made from inside a handler of an urgent change.
    pub fn set(&self, level: Level, reason: impl Into<Cow<'static, str>>) -> Result<u64, Error> {
        if level >= Level::Draining {
            return Err(Error::new(
                ErrorKind::UrgentRefused,
                format!("the urgent line cannot be set to {level} directly"),
                "request a stop instead (Kernel::request_stop)",
            ));
        }
        if self.line.word.level() == level as u8 {
            return Ok(self.epoch());
        }
        refuse_inside_a_handler()?;
        let reason = cap_reason(reason.into());
        let notice = {
            let mut last = self.lock_last();
            match self.line.word.try_raise(level as u8) {
                Ok(previous) if previous == level as u8 => return Ok(self.epoch()),
                Ok(previous) => self.record(&mut last, level, Level::from_u8(previous), reason),
                Err(current) if current >= Level::Draining as u8 => return Err(stopping()),
                Err(current) => {
                    return Err(Error::new(
                        ErrorKind::UrgentRefused,
                        format!(
                            "the urgent line is at {}; set never lowers it",
                            Level::from_u8(current)
                        ),
                        "lower it with shift(from, to), naming the level it is at",
                    ));
                }
            }
        };
        self.deliver(&notice);
        Ok(notice.epoch)
    }

    /// Moves the line from exactly `from` to `to` (both `Normal`, `Shed`, or
    /// `Lockdown`) and returns the new epoch: the way to lower the level,
    /// and to change it without overriding what someone else set since.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::UrgentRefused`] when `from` or `to` is `Draining` or
    /// `Stopping`, when the line is not at `from` (it names the level it is
    /// at), and for a change made from inside a handler of an urgent
    /// change.
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
        refuse_inside_a_handler()?;
        let reason = cap_reason(reason.into());
        let notice = {
            let mut last = self.lock_last();
            match self.line.word.level.compare_exchange(
                from as u8,
                to as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => self.record(&mut last, to, from, reason),
                Err(current) if current >= Level::Draining as u8 => return Err(stopping()),
                Err(current) => {
                    return Err(Error::new(
                        ErrorKind::UrgentRefused,
                        format!(
                            "the urgent line is at {}, not {from}",
                            Level::from_u8(current)
                        ),
                        "read the level again and decide with what is there now",
                    ));
                }
            }
        };
        self.deliver(&notice);
        Ok(notice.epoch)
    }

    /// Raises the line to `level` (never lowers it): the kernel's way to
    /// `Draining` and `Stopping`. Returns `false` if it was already there.
    pub(crate) fn raise(&self, level: Level, reason: &'static str) -> bool {
        let notice = {
            let mut last = self.lock_last();
            let previous = self.line.word.raise(level as u8);
            if previous >= level as u8 {
                return false;
            }
            self.record(
                &mut last,
                level,
                Level::from_u8(previous),
                Cow::Borrowed(reason),
            )
        };
        self.deliver(&notice);
        true
    }

    /// The change's lock. Every change of the level happens under it,
    /// together with its epoch and its reason, so a notice always describes
    /// the change that made it, and epochs follow the order of the changes
    /// (D-089). Readers never take it.
    fn lock_last(&self) -> std::sync::MutexGuard<'_, (Level, Cow<'static, str>)> {
        self.line
            .reason
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Bumps the epoch and records the reason for a change just made
    /// (under the change's lock), and returns its notice.
    fn record(
        &self,
        last: &mut (Level, Cow<'static, str>),
        level: Level,
        previous: Level,
        reason: Cow<'static, str>,
    ) -> Notice {
        let epoch = self.line.word.bump();
        *last = (previous, reason.clone());
        Notice {
            level,
            previous,
            epoch,
            reason,
        }
    }

    /// Delivers a notice to every mailbox and the observer, outside the
    /// change's lock. A handler that tries to change the line from in here
    /// is refused, so it cannot recurse until the stack overflows.
    fn deliver(&self, notice: &Notice) {
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
            DELIVERING.with(|d| d.set(d.get() + 1));
            observer(notice);
            DELIVERING.with(|d| d.set(d.get() - 1));
        }
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

/// The longest reason kept, in bytes: a reason goes to every mailbox, the
/// log, and the audit chain, so a megabyte from a caller must not be
/// copied to each (D-089).
pub(crate) const MAX_REASON: usize = 256;

/// `reason`, cut to [`MAX_REASON`] bytes on a character boundary. Also
/// the cap on every health reason (D-100).
pub(crate) fn cap_reason(reason: Cow<'static, str>) -> Cow<'static, str> {
    if reason.len() <= MAX_REASON {
        return reason;
    }
    match reason {
        Cow::Borrowed(text) => Cow::Borrowed(cut(text, MAX_REASON)),
        Cow::Owned(text) => Cow::Owned(cut(&text, MAX_REASON).to_owned()),
    }
}

/// The longest prefix of `text` that fits in `max` bytes and ends on a
/// character boundary. No allocation.
pub(crate) fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

std::thread_local! {
    /// How deep this thread is in delivering urgent changes.
    static DELIVERING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Refuses a change made while this thread delivers one.
fn refuse_inside_a_handler() -> Result<(), Error> {
    if DELIVERING.with(std::cell::Cell::get) > 0 {
        return Err(Error::new(
            ErrorKind::UrgentRefused,
            "the urgent line cannot be changed from inside a handler of an urgent change",
            "change it from another thread, or after the handler returns",
        ));
    }
    Ok(())
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
        // The same level again changes nothing, not even the reason.
        assert_eq!(
            line.set(Level::Lockdown, "still under attack").ok(),
            Some(1)
        );
        assert_eq!(line.notice().reason, "breach");
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
    fn test_a_long_reason_is_cut_on_a_character_boundary() {
        let line = UrgentLine::detached();
        let long = "\u{e9}".repeat(1_000);
        assert!(line.set(Level::Shed, long).is_ok());
        let reason = line.notice().reason;
        assert!(reason.len() <= MAX_REASON && reason.len() >= MAX_REASON - 1);
        assert!(reason.chars().all(|c| c == '\u{e9}'));
    }

    #[test]
    fn test_concurrent_changes_are_reported_as_they_happened() {
        // Two threads raise and lift a lockdown; every notice must name the
        // level its own change reached, which the audit relies on.
        let line = UrgentLine::detached();
        let seen: Arc<Mutex<Vec<(u64, Level, Level)>>> = Arc::default();
        let sink = Arc::clone(&seen);
        line.observe(Box::new(move |notice| {
            sink.lock().unwrap_or_else(PoisonError::into_inner).push((
                notice.epoch,
                notice.previous,
                notice.level,
            ));
        }));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let line = line.clone();
                thread::spawn(move || {
                    for _ in 0..2_000 {
                        let _raised = line.set(Level::Lockdown, "raise");
                        let _lifted = line.shift(Level::Lockdown, Level::Normal, "lift");
                    }
                })
            })
            .collect();
        for worker in workers {
            assert!(worker.join().is_ok());
        }
        // Delivery may interleave; in epoch order the notices must chain:
        // each one starts where the one before ended.
        let mut seen = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
        seen.sort_unstable_by_key(|(epoch, _, _)| *epoch);
        for (epoch, (at, previous, level)) in seen.iter().enumerate() {
            assert_eq!(*at, epoch as u64 + 1, "an epoch was skipped or repeated");
            assert_ne!(previous, level, "a notice that changed nothing");
        }
        for pair in seen.windows(2) {
            assert_eq!(pair[1].1, pair[0].2, "notices do not chain: {pair:?}");
        }
    }

    #[test]
    fn test_a_handler_cannot_change_the_line() {
        let line = UrgentLine::detached();
        let refused = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&refused);
        let inner = line.clone();
        line.observe(Box::new(move |_notice| {
            if inner.set(Level::Lockdown, "again").is_err() {
                let _seen = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }));
        assert!(line.set(Level::Shed, "first").is_ok());
        assert_eq!(refused.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(line.level(), Level::Shed);
        assert!(line.set(Level::Lockdown, "outside").is_ok());
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
