//! The contract every application subsystem follows.

use std::{
    borrow::Cow,
    error::Error as StdError,
    fmt,
    sync::{Arc, Condvar, Mutex, PoisonError},
    time::{Duration, Instant},
};

use super::Deadline;
use crate::{
    Foundation,
    kernel::{Health, Kernel, Notice, UrgentLine},
};

/// A part of the application that the kernel starts after the members and
/// stops before them: a database pool, a cache, a job runner, a listener.
///
/// Registration is the subscription: a registered subsystem receives every
/// stop and every urgent change through its [`Link`], and it cannot forget
/// to listen.
///
/// | Method | Called | Contract |
/// |---|---|---|
/// | `name` | once, at registration | 1 to 64 ASCII letters, digits, `-`, `_`, `.`; unique in the system |
/// | `position` | once, at registration | the start order among registered subsystems ([`Position`]) |
/// | `start` | boot step 16, on the booting thread | open what it needs; keep the `Link` (clone it) for mailbox notices; an error or a panic unwinds the boot |
/// | `stop` | shutdown step 2, on a kernel stop thread | finish in-flight work and release resources by `deadline`; one that has not returned at the deadline is abandoned (exit code 79) |
/// | `health` | on `health()`, `status()`, and the dump signal | cheap and non-blocking; while `start`, `stop`, or `reload` runs, the last reported health is used |
/// | `reload` | on a reload, before anything is applied | validate and build what changes; return a [`Commit`] that applies it, or a [`Failure`] that refuses the whole reload |
///
/// The trait needs `Send` (stop runs on a kernel thread) and not `Sync`.
///
/// # Examples
///
/// ```
/// use system_suite::kernel::Health;
/// use system_suite::lifecycle::{Deadline, Failure, Link, Position, Subsystem};
///
/// struct Cache { entries: usize }
///
/// impl Subsystem for Cache {
///     fn name(&self) -> &str { "cache" }
///     fn position(&self) -> Position { Position::new(1_000) }   // before the door
///
///     fn start(&mut self, _link: &Link) -> Result<(), Failure> {
///         self.entries = 0;
///         Ok(())
///     }
///
///     fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> {
///         self.entries = 0;                                    // flush, close
///         Ok(())
///     }
///
///     fn health(&self) -> Health {
///         if self.entries < 1_000_000 { Health::HEALTHY } else { Health::degraded("cache is full") }
///     }
/// }
/// ```
pub trait Subsystem: Send + 'static {
    /// The subsystem's name: 1 to 64 ASCII letters, digits, `-`, `_`, `.`;
    /// unique in the system.
    fn name(&self) -> &str;

    /// The declared start order (default [`Position::DEFAULT`]).
    fn position(&self) -> Position {
        Position::DEFAULT
    }

    /// Starts the subsystem. Called once, at boot step 16.
    ///
    /// # Errors
    ///
    /// A `Failure` stops the boot; everything started so far is stopped in
    /// reverse.
    fn start(&mut self, link: &Link) -> Result<(), Failure>;

    /// Stops the subsystem by `deadline`. Called once, at shutdown step 2.
    ///
    /// # Errors
    ///
    /// A `Failure` is logged; the shutdown continues with the next
    /// subsystem.
    fn stop(&mut self, deadline: Deadline) -> Result<(), Failure>;

    /// The subsystem's health; cheap and non-blocking.
    fn health(&self) -> Health {
        Health::HEALTHY
    }

    /// Prepares a reload: validate `config` and build what changes, then
    /// return a [`Commit`] that only swaps the prepared value in.
    ///
    /// # Errors
    ///
    /// A `Failure` refuses the whole reload; nothing changes anywhere.
    #[cfg(feature = "config")]
    fn reload(&mut self, config: &crate::config::Config) -> Result<Commit, Failure> {
        let _unused = config;
        Ok(Commit::none())
    }
}

/// The declared start order: lower starts first, equal positions start in
/// registration order, and the stop order is always the exact reverse of
/// the order in which subsystems actually started (FR-8, D-011).
///
/// Providers that others depend on go early, the door (anything that
/// accepts outside work) at [`LAST`](Self::LAST).
///
/// # Examples
///
/// ```
/// use system_suite::lifecycle::Position;
///
/// assert!(Position::FIRST < Position::DEFAULT && Position::DEFAULT < Position::LAST);
/// assert_eq!(Position::new(1_000).get(), 1_000);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position(u16);

impl Position {
    /// Starts before everything else.
    pub const FIRST: Position = Position(0);
    /// The default, in the middle.
    pub const DEFAULT: Position = Position(32_768);
    /// Starts after everything else: the door.
    pub const LAST: Position = Position(u16::MAX);

    /// A position.
    #[must_use]
    pub const fn new(value: u16) -> Position {
        Position(value)
    }

    /// The position's value.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// What a subsystem receives at `start`: everything it is allowed to reach.
///
/// Cheap to clone (`Arc`); `Send + Sync`. Keep a clone to read the
/// [`Mailbox`] from the subsystem's own thread.
#[derive(Clone)]
pub struct Link {
    inner: Arc<LinkInner>,
}

struct LinkInner {
    name: String,
    foundation: Foundation,
    mailbox: Mailbox,
}

impl Link {
    pub(crate) fn new(name: &str, foundation: Foundation, mailbox: Mailbox) -> Link {
        Link {
            inner: Arc::new(LinkInner {
                name: name.to_owned(),
                foundation,
                mailbox,
            }),
        }
    }

    /// The subsystem's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// The typed handles of every enabled member.
    #[must_use]
    pub fn foundation(&self) -> &Foundation {
        &self.inner.foundation
    }

    /// The kernel's control handle.
    #[must_use]
    pub fn kernel(&self) -> &Kernel {
        self.inner.foundation.kernel()
    }

    /// The urgent line (tier T0).
    #[must_use]
    pub fn urgent(&self) -> &UrgentLine {
        self.kernel().urgent()
    }

    /// This subsystem's mailbox (tier T1): every urgent change is posted
    /// here and wakes whoever waits on it.
    #[must_use]
    pub fn mailbox(&self) -> &Mailbox {
        &self.inner.mailbox
    }

    /// Records an error the subsystem handled but the operator should see:
    /// logged at error level with the subsystem's name, and counted
    /// (`system_suite_subsystem_errors_total{subsystem}`).
    pub fn report_error(&self, error: &(dyn StdError + 'static)) {
        self.kernel()
            .report_subsystem_error(&self.inner.name, error);
    }
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link")
            .field("name", &self.inner.name)
            .finish_non_exhaustive()
    }
}

/// Where the kernel posts urgent changes for one subsystem (tier T1).
///
/// Notices coalesce: a waiter gets the latest one. Cheap to clone; every
/// clone is the same mailbox.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use system_suite::lifecycle::Mailbox;
///
/// let mailbox = Mailbox::new();
/// assert!(mailbox.try_take().is_none());
/// assert!(mailbox.wait_timeout(Duration::from_millis(1)).is_none());
/// ```
#[derive(Clone, Default)]
pub struct Mailbox {
    inner: Arc<MailboxInner>,
}

#[derive(Default)]
struct MailboxInner {
    latest: Mutex<Option<Notice>>,
    posted: Condvar,
}

impl Mailbox {
    /// An empty mailbox. Subsystems receive theirs from the kernel; this
    /// constructor is for tests and for code that wants the same pattern.
    #[must_use]
    pub fn new() -> Mailbox {
        Mailbox::default()
    }

    pub(crate) fn post(&self, notice: Notice) {
        *self.lock() = Some(notice);
        self.inner.posted.notify_all();
    }

    /// Parks the calling thread until a notice arrives, and takes it.
    #[must_use = "the notice says what changed"]
    pub fn wait(&self) -> Notice {
        let mut latest = self.lock();
        loop {
            if let Some(notice) = latest.take() {
                return notice;
            }
            latest = self
                .inner
                .posted
                .wait(latest)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Like [`wait`](Self::wait), but returns `None` if nothing arrived
    /// within `timeout`.
    #[must_use = "the notice says what changed"]
    pub fn wait_timeout(&self, timeout: Duration) -> Option<Notice> {
        let end = Instant::now().checked_add(timeout);
        let mut latest = self.lock();
        loop {
            if let Some(notice) = latest.take() {
                return Some(notice);
            }
            let left = match end {
                Some(end) => end.saturating_duration_since(Instant::now()),
                None => Duration::from_secs(u64::from(u32::MAX)),
            };
            if left.is_zero() {
                return None;
            }
            latest = self
                .inner
                .posted
                .wait_timeout(latest, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Takes the pending notice, if any, without waiting.
    #[must_use = "the notice says what changed"]
    pub fn try_take(&self) -> Option<Notice> {
        self.lock().take()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Notice>> {
        // One assignment under the lock; it cannot be left half-done.
        self.inner
            .latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl fmt::Debug for Mailbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mailbox").finish_non_exhaustive()
    }
}

/// The error a subsystem, worker, or callback returns.
///
/// Any error type converts into it with `?`, so application code keeps its
/// own error types; the kernel wraps it into an [`Error`](crate::Error) of
/// kind `Subsystem` that names the subsystem. `Failure` does not implement
/// `std::error::Error` itself (that is what makes the blanket `From`
/// possible); it is reached as the `source()` of that `Error`.
///
/// # Examples
///
/// ```
/// use system_suite::lifecycle::Failure;
///
/// fn open(path: &str) -> Result<std::fs::File, Failure> {
///     if path.is_empty() {
///         return Err(Failure::new("no path configured"));
///     }
///     Ok(std::fs::File::open(path)?)                 // io::Error converts
/// }
/// assert_eq!(open("").err().map(|f| f.to_string()), Some("no path configured".into()));
/// ```
pub struct Failure {
    message: Cow<'static, str>,
    source: Option<Box<dyn StdError + Send + Sync + 'static>>,
}

impl Failure {
    /// A failure with a message and no underlying error.
    pub fn new(message: impl Into<Cow<'static, str>>) -> Failure {
        Failure {
            message: message.into(),
            source: None,
        }
    }

    /// The message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The failure as a `std::error::Error`, for an `Error`'s source.
    pub(crate) fn into_error(self) -> FailureError {
        FailureError(self)
    }
}

impl<E: StdError + Send + Sync + 'static> From<E> for Failure {
    fn from(error: E) -> Failure {
        Failure {
            message: Cow::Owned(error.to_string()),
            source: Some(Box::new(error)),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl fmt::Debug for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Failure")
            .field("message", &self.message)
            .field("source", &self.source)
            .finish()
    }
}

/// A [`Failure`] as a `std::error::Error`.
#[derive(Debug)]
pub(crate) struct FailureError(Failure);

impl fmt::Display for FailureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl StdError for FailureError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0
            .source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
    }
}

/// The second half of a two-phase reload (D-019).
///
/// In `reload`, a subsystem validates the new configuration and builds
/// everything that can fail, then returns a `Commit` whose closure only
/// swaps the prepared value in. The kernel runs the commits only when every
/// member and subsystem prepared successfully, so a reload changes
/// everything or nothing. The closure cannot fail and should not block.
#[must_use = "a Commit applies nothing unless it is returned to the kernel"]
pub struct Commit {
    apply: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl Commit {
    /// A commit that runs `apply` once the whole reload is accepted.
    pub fn new(apply: impl FnOnce() + Send + 'static) -> Commit {
        Commit {
            apply: Some(Box::new(apply)),
        }
    }

    /// A commit with nothing to apply.
    pub fn none() -> Commit {
        Commit { apply: None }
    }

    /// Runs the commit.
    #[cfg_attr(not(feature = "config"), allow(dead_code))]
    pub(crate) fn apply(self) {
        if let Some(apply) = self.apply {
            apply();
        }
    }
}

impl fmt::Debug for Commit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Commit")
            .field("has_apply", &self.apply.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        thread,
    };

    use super::*;
    use crate::kernel::Level;

    fn notice(level: Level) -> Notice {
        Notice {
            level,
            epoch: 1,
            reason: "test",
        }
    }

    #[test]
    fn test_mailbox_coalesces_to_the_latest_notice() {
        let mailbox = Mailbox::new();
        mailbox.post(notice(Level::Shed));
        mailbox.post(notice(Level::Lockdown));
        assert_eq!(mailbox.try_take().map(|n| n.level), Some(Level::Lockdown));
        assert!(mailbox.try_take().is_none());
    }

    #[test]
    fn test_mailbox_wait_wakes_on_post() {
        let mailbox = Mailbox::new();
        let poster = mailbox.clone();
        let waiter = thread::spawn(move || mailbox.wait());
        thread::sleep(Duration::from_millis(20));
        poster.post(notice(Level::Draining));
        assert_eq!(waiter.join().ok().map(|n| n.level), Some(Level::Draining));
    }

    #[test]
    fn test_mailbox_wait_timeout_returns_none() {
        let mailbox = Mailbox::new();
        let started = Instant::now();
        assert!(mailbox.wait_timeout(Duration::from_millis(30)).is_none());
        assert!(started.elapsed() >= Duration::from_millis(30));
    }

    #[test]
    fn test_failure_from_an_error_keeps_the_source() {
        let failure = Failure::from(std::io::Error::other("disk gone"));
        assert_eq!(failure.message(), "disk gone");
        let error = failure.into_error();
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("disk gone")
        );
    }

    #[test]
    fn test_commit_runs_once_and_none_does_nothing() {
        let ran = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        Commit::new(move || flag.store(true, Ordering::SeqCst)).apply();
        assert!(ran.load(Ordering::SeqCst));
        Commit::none().apply();
    }

    #[test]
    fn test_position_orders_first_default_last() {
        assert!(Position::FIRST < Position::DEFAULT);
        assert!(Position::DEFAULT < Position::LAST);
    }
}
