//! `Snapshot` and `Reader`: a value read on every request and replaced whole.

use std::{
    fmt,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

/// A value that is read on every request and replaced whole on a reload:
/// profiles, compiled rules, budgets, and the configuration itself.
///
/// [`publish`](Self::publish) swaps in a complete new value in one step.
/// Readers already holding the old value keep it until their next
/// [`Reader::get`], and the old value is freed when the last reader moves
/// on. Give each worker thread its own [`Reader`]: `get` is one atomic load
/// and a compare when nothing changed, and a short lock plus an `Arc` clone
/// only right after a publish.
///
/// `Snapshot` is `Send + Sync` and cheap to clone (the clones share one
/// slot). Versions start at 1 and grow by one per publish.
///
/// # Examples
///
/// ```
/// use system_suite::Snapshot;
///
/// let limits = Snapshot::new(vec![100_u32, 200]);
/// let mut reader = limits.reader();
/// assert_eq!(reader.get()[0], 100);
///
/// let version = limits.publish(vec![150, 250]);   // a reload
/// assert_eq!(reader.get()[0], 150);
/// assert_eq!(reader.version(), version);
/// ```
pub struct Snapshot<T> {
    shared: Arc<Shared<T>>,
}

struct Shared<T> {
    version: AtomicU64,
    current: Mutex<Arc<T>>,
}

impl<T: Send + Sync + 'static> Snapshot<T> {
    /// A snapshot holding `value`, at version 1.
    pub fn new(value: T) -> Self {
        Snapshot {
            shared: Arc::new(Shared {
                version: AtomicU64::new(1),
                current: Mutex::new(Arc::new(value)),
            }),
        }
    }

    /// Replaces the value and returns the new version. Readers see it at
    /// their next `get`.
    pub fn publish(&self, value: T) -> u64 {
        let value = Arc::new(value);
        let mut current = self.lock();
        *current = value;
        // Bumped while the lock is held, so the version a reader sees after
        // taking the lock always matches the value it finds.
        let version = self.shared.version.fetch_add(1, Ordering::AcqRel) + 1;
        drop(current);
        version
    }

    /// The current value.
    #[must_use]
    pub fn load(&self) -> Arc<T> {
        Arc::clone(&self.lock())
    }

    /// The current version.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.shared.version.load(Ordering::Acquire)
    }

    /// A reader for one thread.
    #[must_use]
    pub fn reader(&self) -> Reader<T> {
        let current = self.lock();
        let seen = self.shared.version.load(Ordering::Acquire);
        let cached = Arc::clone(&current);
        drop(current);
        Reader {
            shared: Arc::clone(&self.shared),
            seen,
            cached,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Arc<T>> {
        // The lock guards one `Arc` assignment; it cannot be left half-done.
        self.shared
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl<T: Send + Sync + 'static> Snapshot<T> {
    /// A read-only view: the current value, the version, and readers, but
    /// no `publish`. What a snapshot's owner hands out when only it may
    /// replace the value.
    #[must_use]
    pub fn view(&self) -> SnapshotView<T> {
        SnapshotView {
            snapshot: self.clone(),
        }
    }
}

/// A read-only handle to a [`Snapshot`]: [`load`](Self::load),
/// [`version`](Self::version), and [`reader`](Self::reader), never
/// `publish`. The configuration's snapshot is handed out this way, so only
/// a reload (validated, all or nothing) replaces it (D-074).
///
/// # Examples
///
/// ```
/// use system_suite::Snapshot;
///
/// let limits = Snapshot::new(100_u32);
/// let view = limits.view();
/// let mut reader = view.reader();
/// let _version = limits.publish(150);
/// assert_eq!(*reader.get(), 150);
/// assert_eq!(*view.load(), 150);
/// ```
pub struct SnapshotView<T> {
    snapshot: Snapshot<T>,
}

impl<T: Send + Sync + 'static> SnapshotView<T> {
    /// The current value.
    #[must_use]
    pub fn load(&self) -> Arc<T> {
        self.snapshot.load()
    }

    /// The current version (1 for the first value, +1 per publish).
    #[must_use]
    pub fn version(&self) -> u64 {
        self.snapshot.version()
    }

    /// A reader for one thread.
    #[must_use]
    pub fn reader(&self) -> Reader<T> {
        self.snapshot.reader()
    }
}

impl<T> Clone for SnapshotView<T> {
    fn clone(&self) -> Self {
        SnapshotView {
            snapshot: self.snapshot.clone(),
        }
    }
}

impl<T> fmt::Debug for SnapshotView<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapshotView").finish_non_exhaustive()
    }
}

impl<T> Clone for Snapshot<T> {
    fn clone(&self) -> Self {
        Snapshot {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> fmt::Debug for Snapshot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Snapshot")
            .field("version", &self.shared.version.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

/// One thread's view of a [`Snapshot`]: the value it last saw, refreshed
/// by [`get`](Self::get) only when a new one was published.
///
/// `Send`; not `Sync` (it belongs to one thread).
pub struct Reader<T> {
    shared: Arc<Shared<T>>,
    seen: u64,
    cached: Arc<T>,
}

impl<T: Send + Sync + 'static> Reader<T> {
    /// The newest value. About a nanosecond when nothing was published
    /// since the last call [est.]; no allocation.
    #[inline]
    pub fn get(&mut self) -> &T {
        let version = self.shared.version.load(Ordering::Acquire);
        if version != self.seen {
            self.refresh();
        }
        &self.cached
    }

    #[cold]
    fn refresh(&mut self) {
        let current = self
            .shared
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.cached = Arc::clone(&current);
        self.seen = self.shared.version.load(Ordering::Acquire);
    }

    /// The version of the value `get` last returned.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.seen
    }
}

impl<T> fmt::Debug for Reader<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reader")
            .field("version", &self.seen)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn test_snapshot_starts_at_version_one() {
        let snapshot = Snapshot::new(1);
        assert_eq!(snapshot.version(), 1);
        assert_eq!(*snapshot.load(), 1);
    }

    #[test]
    fn test_reader_sees_a_publish_at_its_next_get() {
        let snapshot = Snapshot::new(String::from("old"));
        let mut reader = snapshot.reader();
        assert_eq!(reader.get(), "old");
        let version = snapshot.publish(String::from("new"));
        assert_eq!(version, 2);
        assert_eq!(reader.get(), "new");
        assert_eq!(reader.version(), 2);
    }

    #[test]
    fn test_a_held_value_survives_a_publish() {
        let snapshot = Snapshot::new(vec![1, 2, 3]);
        let held = snapshot.load();
        let _version = snapshot.publish(vec![9]);
        assert_eq!(*held, vec![1, 2, 3]);
        assert_eq!(*snapshot.load(), vec![9]);
    }

    #[test]
    fn test_readers_on_many_threads_converge_on_the_last_publish() {
        let snapshot = Snapshot::new(0_u64);
        let mut handles = Vec::new();
        for _ in 0..4 {
            let mut reader = snapshot.reader();
            handles.push(thread::spawn(move || {
                let mut last = 0;
                for _ in 0..10_000 {
                    let value = *reader.get();
                    assert!(value >= last, "a reader went backwards");
                    last = value;
                }
            }));
        }
        for value in 1..=1_000 {
            let _version = snapshot.publish(value);
        }
        for handle in handles {
            assert!(handle.join().is_ok());
        }
        assert_eq!(*snapshot.reader().get(), 1_000);
    }

    #[test]
    fn test_snapshot_and_reader_are_send() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<Snapshot<String>>();
        assert_sync::<Snapshot<String>>();
        assert_send::<Reader<String>>();
    }
}
