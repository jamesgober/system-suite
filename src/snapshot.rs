//! `Snapshot` and `Reader`: a value read on every request and replaced whole.

use std::{
    fmt,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

use arc_swap::ArcSwap;

/// A value that is read on every request and replaced whole on a reload:
/// profiles, compiled rules, budgets, and the configuration itself.
///
/// [`publish`](Self::publish) swaps in a complete new value in one step.
/// Readers already holding the old value keep it until their next
/// [`Reader::get`], and the old value is freed when the last reader moves
/// on. [`load`](Self::load) takes no lock: a lock-free pointer read and one
/// `Arc` clone, so threads that call it at once do not wait on each other
/// (D-100). A [`Reader`] is cheaper still for a thread that reads on every
/// request: `get` is one atomic load and a compare when nothing changed.
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
    /// The version of `current`, for a reader's fast check. Stored after
    /// `current`, so a reader that sees it finds that value or a newer one.
    version: AtomicU64,
    /// The value and its version, swapped in together.
    current: ArcSwap<Versioned<T>>,
    /// Orders publishers, so versions grow by one per publish.
    publishing: Mutex<()>,
}

/// A value and the version it was published as.
struct Versioned<T> {
    version: u64,
    value: Arc<T>,
}

impl<T: Send + Sync + 'static> Snapshot<T> {
    /// A snapshot holding `value`, at version 1.
    pub fn new(value: T) -> Self {
        Snapshot {
            shared: Arc::new(Shared {
                version: AtomicU64::new(1),
                current: ArcSwap::from_pointee(Versioned {
                    version: 1,
                    value: Arc::new(value),
                }),
                publishing: Mutex::new(()),
            }),
        }
    }

    /// Replaces the value and returns the new version. Readers see it at
    /// their next `get`.
    pub fn publish(&self, value: T) -> u64 {
        let value = Arc::new(value);
        // One publisher at a time; readers never take this lock.
        let publishing = self
            .shared
            .publishing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let version = self.shared.current.load().version.saturating_add(1);
        self.shared
            .current
            .store(Arc::new(Versioned { version, value }));
        self.shared.version.store(version, Ordering::Release);
        drop(publishing);
        version
    }

    /// The current value. Lock-free: a pointer read and one `Arc` clone.
    #[must_use]
    pub fn load(&self) -> Arc<T> {
        Arc::clone(&self.shared.current.load().value)
    }

    /// The current version.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.shared.version.load(Ordering::Acquire)
    }

    /// A reader for one thread.
    #[must_use]
    pub fn reader(&self) -> Reader<T> {
        let current = self.shared.current.load();
        Reader {
            shared: Arc::clone(&self.shared),
            seen: current.version,
            cached: Arc::clone(&current.value),
        }
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
        // The value and its version come from one load, so they match.
        let current = self.shared.current.load();
        self.cached = Arc::clone(&current.value);
        self.seen = current.version;
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
    fn test_a_readers_value_and_version_always_match_under_publishes() {
        // D-100: `load` and `Reader::get` take no lock. The value published
        // as version v is v here, so a mismatch shows directly.
        let snapshot = Snapshot::new(1_u64);
        let publisher = {
            let snapshot = snapshot.clone();
            thread::spawn(move || {
                for _ in 0..20_000 {
                    let next = *snapshot.load() + 1;
                    let version = snapshot.publish(next);
                    assert_eq!(version, next);
                }
            })
        };
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let snapshot = snapshot.clone();
                thread::spawn(move || {
                    let mut reader = snapshot.reader();
                    let mut last = 0;
                    for _ in 0..50_000 {
                        let value = *reader.get();
                        assert_eq!(value, reader.version());
                        assert!(value >= last, "went back from {last} to {value}");
                        last = value;
                        assert!(*snapshot.load() >= value);
                    }
                })
            })
            .collect();
        assert!(publisher.join().is_ok());
        for reader in readers {
            assert!(reader.join().is_ok());
        }
        assert_eq!(snapshot.version(), 20_001);
        assert_eq!(*snapshot.load(), 20_001);
    }

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
