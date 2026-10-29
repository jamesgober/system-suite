//! `AuditLog`: the application's tamper-evident record of privileged
//! actions, an audit-trail chain over an owner-only file (boot step 13).

use std::{
    fmt,
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use audit_trail::{
    Action, Actor, Chain, Digest, FileReader, FileSink, HASH_LEN, Hasher, MonotonicClock, Outcome,
    RecordId, Sha256Hasher, SystemClock, Target,
};

use crate::{Error, ErrorKind, Member};

/// The hash a chain is written with (`[audit] hash`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HashKind {
    Sha256,
    #[cfg(feature = "audit-blake3")]
    Blake3,
}

/// The configured hasher, behind one type so the chain's type does not
/// depend on the configuration.
enum AuditHasher {
    Sha256(Sha256Hasher),
    #[cfg(feature = "audit-blake3")]
    // Boxed: the BLAKE3 state is about 2 KiB, SHA-256's about 100 bytes.
    Blake3(Box<audit_trail::Blake3Hasher>),
}

impl AuditHasher {
    fn new(kind: HashKind) -> AuditHasher {
        match kind {
            HashKind::Sha256 => AuditHasher::Sha256(Sha256Hasher::new()),
            #[cfg(feature = "audit-blake3")]
            HashKind::Blake3 => AuditHasher::Blake3(Box::new(audit_trail::Blake3Hasher::new())),
        }
    }
}

impl Hasher for AuditHasher {
    fn reset(&mut self) {
        match self {
            AuditHasher::Sha256(h) => h.reset(),
            #[cfg(feature = "audit-blake3")]
            AuditHasher::Blake3(h) => h.reset(),
        }
    }

    fn update(&mut self, bytes: &[u8]) {
        match self {
            AuditHasher::Sha256(h) => h.update(bytes),
            #[cfg(feature = "audit-blake3")]
            AuditHasher::Blake3(h) => h.update(bytes),
        }
    }

    fn finalize(&mut self, out: &mut Digest) {
        match self {
            AuditHasher::Sha256(h) => h.finalize(out),
            #[cfg(feature = "audit-blake3")]
            AuditHasher::Blake3(h) => h.finalize(out),
        }
    }
}

type AuditChain = Chain<AuditHasher, FileSink<BufWriter<File>>, MonotonicClock<SystemClock>>;

struct Open {
    chain: Mutex<AuditChain>,
    path: PathBuf,
    sync_every: bool,
    /// Where the chain's tail is anchored, outside the chain file (D-072).
    anchor: Option<PathBuf>,
}

/// The application's tamper-evident record of privileged actions: who did
/// what to which target, and the outcome, never sensitive content.
///
/// Every record is hash-linked to the one before it, so a changed,
/// removed, or reordered record is found when the chain is opened at the
/// next boot (which then stops with [`ErrorKind::AuditTampered`]). With
/// `[audit] sync = "every"` (the default) a record is on disk before
/// [`record`](Self::record) returns. The kernel records its own
/// privileged actions here: reloads, urgent-line changes, and stop
/// requests.
///
/// Cheap to clone; `Send + Sync`. With [`Member::Audit`] disabled (or
/// `[audit] enabled = false`) every operation fails with
/// [`ErrorKind::Disabled`].
///
/// # Examples
///
/// ```no_run
/// # #[cfg(feature = "audit")] {
/// use system_suite::audit::Outcome;
///
/// let system = system_suite::start!()?;
/// system.audit().record("admin:42", "user.delete", "user:1009", Outcome::Success)?;
/// # }
/// # Ok::<(), system_suite::Error>(())
/// ```
#[derive(Clone)]
pub struct AuditLog {
    open: Option<Arc<Open>>,
}

impl AuditLog {
    pub(crate) fn disabled() -> AuditLog {
        AuditLog { open: None }
    }

    /// Opens (or creates) the chain at `path`, verifies every record, and
    /// resumes at its tail.
    pub(crate) fn open(
        path: &Path,
        hash: HashKind,
        sync_every: bool,
        repair_tail: bool,
        anchor: Option<&Path>,
    ) -> Result<(AuditLog, OpenSummary), Error> {
        // An existing chain (perhaps at a configured `[audit] file`) is
        // refused when another user could rewrite it; a new one is created
        // owner-only by audit-trail.
        if path.exists() {
            let existing = File::open(path).map_err(|e| {
                Error::new(
                    ErrorKind::Audit,
                    format!("the audit chain {} cannot be opened: {e}", path.display()),
                    "check that the data directory is readable by the service's user",
                )
                .member_of(Member::Audit)
            })?;
            crate::secure_file::check_open(&existing, path, "audit chain")?;
        }
        let clock = MonotonicClock::new(SystemClock::new());
        let opened = if repair_tail {
            Chain::open_file_repairing_tail(path, AuditHasher::new(hash), clock)
        } else {
            Chain::open_file(path, AuditHasher::new(hash), clock)
        };
        let (chain, report) = opened.map_err(|e| open_error(path, e))?;
        // A wall clock that is now behind the last record must not stop
        // the chain: timestamps continue from the tail.
        chain.clock().advance_past(chain.last_timestamp());
        if let Some(anchor) = anchor {
            check_anchor(anchor, path, &chain)?;
            write_anchor(anchor, chain.next_id(), chain.last_hash())?;
        }
        let summary = OpenSummary {
            records: report.records(),
            created: report.created(),
            cut: report
                .cut()
                .map(|(bytes, kept)| (bytes, kept.to_path_buf())),
        };
        let log = AuditLog {
            open: Some(Arc::new(Open {
                chain: Mutex::new(chain),
                path: path.to_path_buf(),
                sync_every,
                anchor: anchor.map(Path::to_path_buf),
            })),
        };
        Ok((log, summary))
    }

    /// Appends one hash-linked record: `actor` did `action` to `target`
    /// with `outcome`. Returns the record's id. With `[audit] sync =
    /// "every"` the record is on disk when this returns.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Audit`] when the record cannot be written or synced;
    /// [`ErrorKind::Disabled`] when audit is disabled.
    pub fn record(
        &self,
        actor: &str,
        action: &str,
        target: &str,
        outcome: Outcome,
    ) -> Result<RecordId, Error> {
        let open = self.chain_file()?;
        let mut chain = lock(&open.chain);
        let id = chain
            .append(
                Actor::new(actor),
                Action::new(action),
                Target::new(target),
                outcome,
            )
            .map_err(|e| write_error(&open.path, &e.to_string()))?;
        if open.sync_every {
            chain
                .sink_mut()
                .sync()
                .map_err(|e| write_error(&open.path, &e.to_string()))?;
            open.anchor_tail(&chain)?;
        }
        Ok(id)
    }

    /// Flushes buffered records and syncs the file to disk.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Audit`] when the sync fails; [`ErrorKind::Disabled`]
    /// when audit is disabled.
    pub fn sync(&self) -> Result<(), Error> {
        let open = self.chain_file()?;
        let mut chain = lock(&open.chain);
        chain
            .sink_mut()
            .sync()
            .map_err(|e| write_error(&open.path, &e.to_string()))?;
        open.anchor_tail(&chain)
    }

    /// The chain file; `None` when audit is disabled.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.open.as_ref().map(|open| open.path.as_path())
    }

    /// The id the next record will receive; `None` when audit is disabled.
    #[must_use]
    pub fn next_id(&self) -> Option<RecordId> {
        self.open.as_ref().map(|open| lock(&open.chain).next_id())
    }

    /// Whether audit is on for this system.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.open.is_some()
    }

    fn chain_file(&self) -> Result<&Open, Error> {
        self.open.as_deref().ok_or_else(|| {
            Error::new(
                ErrorKind::Disabled,
                "audit is disabled for this system",
                "enable it: remove disable(Member::Audit) or set [audit] enabled = true",
            )
            .member_of(Member::Audit)
        })
    }
}

impl Open {
    /// Moves the anchor to the chain's tail, after the tail is on disk.
    fn anchor_tail(&self, chain: &AuditChain) -> Result<(), Error> {
        match &self.anchor {
            Some(anchor) => write_anchor(anchor, chain.next_id(), chain.last_hash()),
            None => Ok(()),
        }
    }
}

/// `v1 <next id> <hex hash of the last record>`
fn format_anchor(next_id: RecordId, last_hash: Digest) -> String {
    let mut hex = String::with_capacity(HASH_LEN * 2);
    for byte in last_hash.as_bytes() {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("v1 {} {hex}\n", next_id.as_u64())
}

fn parse_anchor(text: &str) -> Option<(u64, Digest)> {
    let mut parts = text.split_whitespace();
    if parts.next()? != "v1" {
        return None;
    }
    let next_id = parts.next()?.parse().ok()?;
    let hex = parts.next()?;
    if hex.len() != HASH_LEN * 2 {
        return None;
    }
    let mut bytes = [0_u8; HASH_LEN];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some((next_id, Digest::from_bytes(bytes)))
}

/// Writes the anchor owner-only, through a temporary file and a rename, so
/// a crash never leaves a half-written anchor.
fn write_anchor(anchor: &Path, next_id: RecordId, last_hash: Digest) -> Result<(), Error> {
    use std::io::Write as _;

    let fail = |e: std::io::Error| {
        Error::new(
            ErrorKind::Audit,
            format!(
                "the audit anchor {} cannot be written: {e}",
                anchor.display()
            ),
            "check that the state directory is writable by the service's user",
        )
        .member_of(Member::Audit)
    };
    let temporary = anchor.with_extension("anchor.tmp");
    let mut options = std::fs::OpenOptions::new();
    let _same = options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let _same = options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(fail)?;
    file.write_all(format_anchor(next_id, last_hash).as_bytes())
        .map_err(fail)?;
    file.sync_data().map_err(fail)?;
    drop(file);
    std::fs::rename(&temporary, anchor).map_err(fail)
}

/// Refuses a chain whose tail is behind its anchor, or whose record at the
/// anchor is not the one anchored: the chain was shortened, replaced, or
/// rewritten (the 0.5 defence idea, D-072).
fn check_anchor(anchor: &Path, path: &Path, chain: &AuditChain) -> Result<(), Error> {
    let text = match std::fs::read_to_string(anchor) {
        Ok(text) => text,
        // No anchor yet: the first boot with one; it is written now.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(Error::new(
                ErrorKind::Audit,
                format!("the audit anchor {} cannot be read: {e}", anchor.display()),
                "check the state directory's permissions",
            )
            .member_of(Member::Audit));
        }
    };
    let tampered = |detail: String| {
        Error::new(
            ErrorKind::AuditTampered,
            format!("the audit chain {} {detail}", path.display()),
            "do not delete anything: keep the chain and the anchor as evidence and investigate; if the chain was moved aside on purpose, move the anchor aside too",
        )
        .member_of(Member::Audit)
    };
    let Some((anchored_next, anchored_hash)) = parse_anchor(&text) else {
        return Err(tampered(format!(
            "has an anchor {} that does not parse",
            anchor.display()
        )));
    };
    let next = chain.next_id().as_u64();
    if next < anchored_next {
        return Err(tampered(format!(
            "ends at record {next}, but it was anchored at {anchored_next}: records were removed, or the chain was replaced"
        )));
    }
    if anchored_next == 0 {
        return Ok(());
    }
    let hash_at_anchor = if next == anchored_next {
        Some(chain.last_hash())
    } else {
        // Records were appended after the anchor was last moved (a crash
        // between the two): find the anchored record in the file.
        let wanted = anchored_next - 1;
        FileReader::open(path)
            .ok()
            .and_then(|reader| {
                reader
                    .filter_map(std::result::Result::ok)
                    .find(|record| record.id.as_u64() == wanted)
            })
            .map(|record| record.hash)
    };
    match hash_at_anchor {
        Some(hash) if hash == anchored_hash => Ok(()),
        _ => Err(tampered(format!(
            "does not hold the record it was anchored at ({})",
            anchored_next - 1
        ))),
    }
}

impl fmt::Debug for AuditLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuditLog")
            .field("path", &self.path())
            .finish_non_exhaustive()
    }
}

/// What opening the chain found, for the first log lines.
#[derive(Debug)]
pub(crate) struct OpenSummary {
    pub(crate) records: u64,
    pub(crate) created: bool,
    pub(crate) cut: Option<(u64, PathBuf)>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn write_error(path: &Path, detail: &str) -> Error {
    Error::new(
        ErrorKind::Audit,
        format!(
            "the audit chain {} cannot be written: {detail}",
            path.display()
        ),
        "check the disk and the file's permissions; the record was not appended",
    )
    .member_of(Member::Audit)
}

fn open_error(path: &Path, error: audit_trail::Error) -> Error {
    use audit_trail::Error as E;

    let file = path.display();
    let built = match error {
        E::HashMismatch(id) | E::LinkMismatch(id) | E::IdMismatch(id) => Error::new(
            ErrorKind::AuditTampered,
            format!(
                "the audit chain {file} fails verification at record {}: a record was changed, removed, or reordered, or [audit] hash differs from the hash it was written with",
                id.as_u64()
            ),
            "do not delete the file: keep it as evidence, move it aside to start a new chain, and investigate",
        ),
        E::NonMonotonicClock | E::InvalidFormat => Error::new(
            ErrorKind::AuditTampered,
            format!("the audit chain {file} does not verify: {error}"),
            "do not delete the file: keep it as evidence, move it aside to start a new chain, and investigate",
        ),
        E::Truncated => Error::new(
            ErrorKind::Audit,
            format!(
                "the last record of the audit chain {file} was cut short (a crash while writing, or a cut file)"
            ),
            "set [audit] repair_tail = true to keep the incomplete bytes in a side file and continue, after checking the file",
        ),
        _ => Error::new(
            ErrorKind::Audit,
            format!("the audit chain {file} cannot be opened: {error}"),
            "check that the data directory is writable by the service's user",
        ),
    };
    built.member_of(Member::Audit).caused_by(error)
}
