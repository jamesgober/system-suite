//! The kernel's services in one process: subsystems and their order, the
//! subsystem-level deadline (threat T6), the two-phase reload (threat T7),
//! the staged `Boot`, the urgent line, workers, timers, and health.
//!
//! Every system here is built with `process_wide(false)` and a portable
//! root under the temporary directory, so several run side by side and
//! nothing touches the user's own directories.

#![cfg(all(feature = "kernel", feature = "sys", feature = "config"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use system_suite::{
    Builder, ErrorKind, Member, Step, System,
    kernel::{Component, Level, Status},
    lifecycle::{Commit, Deadline, Failure, Link, Position, Subsystem},
    runtime::{Backoff, Restart, Worker, WorkerState},
};

/// A private directory tree, removed when dropped.
struct Root(PathBuf);

impl Root {
    fn new(label: &str) -> Root {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = env::temp_dir().join(format!(
            "system-suite-kernel-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        Root(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// Writes a configuration file only its owner can change.
    fn write_config(&self, text: &str) -> PathBuf {
        let path = self.0.join("app.conf");
        write_private(&path, text);
        path
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

fn write_private(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn quiet(name: &str, root: &Root) -> Builder {
    System::builder(name)
        .process_wide(false)
        .single_instance(false)
        .disable(Member::Log)
        .portable(root.path())
}

type Journal = Arc<Mutex<Vec<String>>>;

fn entries(journal: &Journal) -> Vec<String> {
    journal.lock().unwrap().clone()
}

/// A subsystem that writes what happens to it into a journal.
struct Recorder {
    name: &'static str,
    position: Position,
    journal: Journal,
}

impl Recorder {
    fn new(name: &'static str, position: Position, journal: &Journal) -> Recorder {
        Recorder {
            name,
            position,
            journal: Arc::clone(journal),
        }
    }
}

impl Subsystem for Recorder {
    fn name(&self) -> &str {
        self.name
    }
    fn position(&self) -> Position {
        self.position
    }
    fn start(&mut self, _link: &Link) -> Result<(), Failure> {
        self.journal
            .lock()
            .unwrap()
            .push(format!("start {}", self.name));
        Ok(())
    }
    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> {
        self.journal
            .lock()
            .unwrap()
            .push(format!("stop {}", self.name));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Subsystems
// ---------------------------------------------------------------------------

#[test]
fn test_subsystems_start_by_position_and_stop_in_reverse() {
    let root = Root::new("order");
    let journal = Journal::default();
    let system = quiet("kernel-order", &root)
        .register(Recorder::new("door", Position::LAST, &journal))
        .register(Recorder::new("store", Position::FIRST, &journal))
        .register(Recorder::new("cache", Position::DEFAULT, &journal))
        .register(Recorder::new("jobs", Position::DEFAULT, &journal))
        .start()
        .unwrap();
    assert_eq!(
        entries(&journal),
        ["start store", "start cache", "start jobs", "start door"]
    );
    let report = system.health();
    let subsystems = report
        .components()
        .iter()
        .filter(|c| c.kind == Component::Subsystem)
        .count();
    assert_eq!(subsystems, 4);
    assert!(report.ready());
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
    assert_eq!(
        entries(&journal)[4..],
        ["stop door", "stop jobs", "stop cache", "stop store"]
    );
}

/// Starts, then fails or panics.
struct Broken {
    panic: bool,
}

impl Subsystem for Broken {
    fn name(&self) -> &str {
        "broken"
    }
    fn start(&mut self, _link: &Link) -> Result<(), Failure> {
        if self.panic {
            panic!("broken subsystem");
        }
        Err(Failure::new("the database is unreachable"))
    }
    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> {
        Ok(())
    }
}

#[test]
fn test_a_failing_subsystem_unwinds_the_ones_started_before_it() {
    for panic in [false, true] {
        let root = Root::new("unwind");
        let journal = Journal::default();
        let error = quiet("kernel-unwind", &root)
            .register(Recorder::new("store", Position::FIRST, &journal))
            .register(Broken { panic })
            .register(Recorder::new("door", Position::LAST, &journal))
            .start()
            .err()
            .unwrap();
        assert_eq!(error.kind(), ErrorKind::Subsystem);
        assert_eq!(error.step(), Some(Step::Subsystems));
        assert!(error.to_string().contains("broken"), "{error}");
        assert_eq!(entries(&journal), ["start store", "stop store"]);
    }
}

/// Never returns from `stop`.
struct Stuck {
    released: Arc<AtomicBool>,
}

impl Subsystem for Stuck {
    fn name(&self) -> &str {
        "stuck"
    }
    fn start(&mut self, _link: &Link) -> Result<(), Failure> {
        Ok(())
    }
    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> {
        while !self.released.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

#[test]
fn test_t6_a_subsystem_that_never_stops_is_abandoned_at_the_deadline() {
    let root = Root::new("t6");
    let journal = Journal::default();
    let released = Arc::new(AtomicBool::new(false));
    let system = quiet("kernel-t6", &root)
        .shutdown_deadline(Duration::from_millis(800))
        .member_reserve(Duration::from_millis(300))
        .register(Recorder::new("store", Position::FIRST, &journal))
        .register(Stuck {
            released: Arc::clone(&released),
        })
        .start()
        .unwrap();
    let began = Instant::now();
    let code = system.shutdown();
    let took = began.elapsed();
    released.store(true, Ordering::SeqCst);
    assert_eq!(code, ExitCode::from(system_suite::exit::DEADLINE));
    // The subsystem that started before the stuck one was still stopped.
    assert_eq!(entries(&journal), ["start store", "stop store"]);
    assert!(took < Duration::from_secs(5), "the shutdown took {took:?}");
}

// ---------------------------------------------------------------------------
// Reload (threat T7)
// ---------------------------------------------------------------------------

/// Accepts or refuses a reload depending on `[app] accept`, and applies
/// `[app] limit` only at commit.
struct Limits {
    applied: Arc<AtomicU32>,
}

impl Subsystem for Limits {
    fn name(&self) -> &str {
        "limits"
    }
    fn start(&mut self, link: &Link) -> Result<(), Failure> {
        let limit: u32 = link.foundation().config().get_or("app.limit", 0)?;
        self.applied.store(limit, Ordering::SeqCst);
        Ok(())
    }
    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> {
        Ok(())
    }
    fn reload(&mut self, config: &system_suite::config::Config) -> Result<Commit, Failure> {
        if !config.get_or("app.accept", true)? {
            return Err(Failure::new("this change needs a restart"));
        }
        let limit: u32 = config.get_or("app.limit", 0)?;
        let applied = Arc::clone(&self.applied);
        Ok(Commit::new(move || applied.store(limit, Ordering::SeqCst)))
    }
}

#[test]
fn test_t7_reload_publishes_only_a_valid_accepted_configuration() {
    let root = Root::new("t7");
    let file = root.write_config("[app]\nlimit = 10\n");
    let applied = Arc::new(AtomicU32::new(0));
    let system = quiet("kernel-t7", &root)
        .config_file(&file)
        .register(Limits {
            applied: Arc::clone(&applied),
        })
        .start()
        .unwrap();
    let mut reader = system.config_snapshot().reader();
    assert_eq!(system.config().version(), 1);
    assert_eq!(applied.load(Ordering::SeqCst), 10);

    // A file that does not parse changes nothing.
    write_private(&file, "[app\nlimit = 20\n");
    let error = system.reload().err().unwrap();
    assert_eq!(error.kind(), ErrorKind::ConfigInvalid);
    assert_eq!(system.config().version(), 1);
    assert_eq!(applied.load(Ordering::SeqCst), 10);

    // An unknown key in a suite section changes nothing.
    write_private(&file, "[app]\nlimit = 20\n[log]\nlevle = \"debug\"\n");
    assert_eq!(
        system.reload().err().map(|e| e.kind()),
        Some(ErrorKind::ConfigInvalid)
    );

    // A subsystem that refuses changes nothing, and is named.
    write_private(&file, "[app]\nlimit = 20\naccept = false\n");
    let error = system.reload().err().unwrap();
    assert_eq!(error.kind(), ErrorKind::ReloadRefused);
    assert!(error.to_string().contains("limits"), "{error}");
    assert_eq!(system.config().version(), 1);
    assert_eq!(applied.load(Ordering::SeqCst), 10);
    assert_eq!(reader.get().get_or("app.limit", 0_u32).unwrap(), 10);

    // A secret reference is refused until the secrets feature (0.5).
    write_private(
        &file,
        "[app]\nlimit = 20\npassword = \"env:APP_PASSWORD\"\n",
    );
    assert_eq!(
        system.reload().err().map(|e| e.kind()),
        Some(ErrorKind::Secret)
    );

    // A valid, accepted change is published and committed.
    write_private(&file, "[app]\nlimit = 30\n");
    assert_eq!(system.reload().unwrap(), 2);
    assert_eq!(system.config().version(), 2);
    assert_eq!(applied.load(Ordering::SeqCst), 30);
    assert_eq!(reader.get().get_or("app.limit", 0_u32).unwrap(), 30);
    assert_eq!(reader.version(), 2);

    // After a stop request, a reload is refused.
    assert!(system.request_stop("test"));
    assert_eq!(
        system.reload().err().map(|e| e.kind()),
        Some(ErrorKind::Stopping)
    );
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
}

#[test]
fn test_config_str_is_read_again_on_reload() {
    let root = Root::new("str");
    let system = quiet("kernel-str", &root)
        .config_str("[app]\nname = \"inline\"\n")
        .start()
        .unwrap();
    let config = system.config();
    assert_eq!(config.require::<String>("app.name").unwrap(), "inline");
    assert!(config.source().is_none());
    assert_eq!(system.reload().unwrap(), 2);
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
}

#[test]
fn test_configuration_failures_stop_the_boot_at_step_5() {
    let root = Root::new("step5");
    let cases: [(&str, ErrorKind); 4] = [
        ("[app\n", ErrorKind::ConfigInvalid),
        (
            "[kernel]\nshutdown_deadline = \"2h\"\n",
            ErrorKind::ConfigInvalid,
        ),
        ("[ids]\nworker = 5000\n", ErrorKind::ConfigInvalid),
        ("[db]\npassword = \"vault:db\"\n", ErrorKind::Secret),
    ];
    for (text, kind) in cases {
        let error = quiet("kernel-step5", &root)
            .config_str(text)
            .start()
            .err()
            .unwrap();
        assert_eq!(error.kind(), kind, "{text}");
        // With the secrets feature a reference is resolved (and here fails)
        // at step 8; without it, it is refused at step 5.
        let step = if kind == ErrorKind::Secret && cfg!(feature = "secrets") {
            Step::Secrets
        } else {
            Step::Configuration
        };
        assert_eq!(error.step(), Some(step), "{text}");
    }
    let error = quiet("kernel-step5", &root)
        .config_file(root.path().join("missing.conf"))
        .start()
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::ConfigNotFound);
}

#[cfg(unix)]
#[test]
fn test_a_configuration_file_others_can_change_is_refused() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = Root::new("insecure");
    let file = root.write_config("[app]\nlimit = 1\n");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o664)).unwrap();
    let error = quiet("kernel-insecure", &root)
        .config_file(&file)
        .start()
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::ConfigInsecure);
    assert!(!error.to_string().contains("limit"), "{error}");
}

#[cfg(feature = "config-toml")]
#[test]
fn test_a_toml_file_is_read_and_reloaded() {
    let root = Root::new("toml");
    let file = root.path().join("app.toml");
    write_private(
        &file,
        "[app]
limit = 5

[log]
level = \"debug\"
",
    );
    let system = quiet("kernel-toml", &root)
        .config_file(&file)
        .start()
        .unwrap();
    assert_eq!(system.config().get_or("app.limit", 0_u32).unwrap(), 5);
    write_private(
        &file,
        "[app]
limit = 6
",
    );
    assert_eq!(system.reload().unwrap(), 2);
    assert_eq!(system.config().get_or("app.limit", 0_u32).unwrap(), 6);
    write_private(
        &file,
        "[app
limit = 7
",
    );
    assert_eq!(
        system.reload().err().map(|e| e.kind()),
        Some(ErrorKind::ConfigInvalid)
    );
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
}

#[cfg(not(feature = "config-toml"))]
#[test]
fn test_a_toml_file_needs_its_feature() {
    let root = Root::new("notoml");
    let file = root.path().join("app.toml");
    write_private(
        &file,
        "[app]
limit = 5
",
    );
    let error = quiet("kernel-notoml", &root)
        .config_file(&file)
        .start()
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::ConfigInvalid);
    assert!(error.to_string().contains("toml"), "{error}");
}

#[test]
fn test_ids_take_the_worker_from_the_configuration() {
    let root = Root::new("ids");
    let system = quiet("kernel-ids", &root)
        .config_str("[ids]\nworker = 12\n")
        .start()
        .unwrap();
    assert_eq!(system.ids().worker_id(), 12);
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
}

// ---------------------------------------------------------------------------
// Boot, the urgent line, workers, timers
// ---------------------------------------------------------------------------

#[test]
fn test_staged_boot_starts_in_call_order_and_drop_unwinds() {
    let root = Root::new("staged");
    let journal = Journal::default();
    let mut boot = quiet("kernel-staged", &root)
        .register(Recorder::new("registered", Position::FIRST, &journal))
        .boot()
        .unwrap();
    boot.start(Recorder::new("provider", Position::LAST, &journal))
        .unwrap();
    let duplicate = boot.start(Recorder::new("provider", Position::LAST, &journal));
    assert_eq!(duplicate.err().map(|e| e.kind()), Some(ErrorKind::Setup));
    let failed = boot.start(Broken { panic: false });
    assert_eq!(failed.err().map(|e| e.kind()), Some(ErrorKind::Subsystem));
    drop(boot);
    // Registered subsystems start only at ready(); the provider was stopped.
    assert_eq!(entries(&journal), ["start provider", "stop provider"]);

    let journal = Journal::default();
    let mut boot = quiet("kernel-staged", &root)
        .register(Recorder::new("registered", Position::FIRST, &journal))
        .on_ready({
            let journal = Arc::clone(&journal);
            move |_foundation| {
                journal.lock().unwrap().push("on_ready".to_owned());
                Ok(())
            }
        })
        .boot()
        .unwrap();
    boot.start(Recorder::new("provider", Position::LAST, &journal))
        .unwrap();
    let system = boot.ready().unwrap();
    assert_eq!(
        entries(&journal),
        ["start provider", "start registered", "on_ready"]
    );
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
    assert_eq!(entries(&journal)[3..], ["stop registered", "stop provider"]);
}

#[test]
fn test_a_stop_during_a_staged_boot_interrupts_it() {
    let root = Root::new("interrupt");
    let journal = Journal::default();
    let mut boot = quiet("kernel-interrupt", &root).boot().unwrap();
    boot.start(Recorder::new("first", Position::FIRST, &journal))
        .unwrap();
    assert!(boot.kernel().request_stop("operator"));
    let error = boot
        .start(Recorder::new("second", Position::DEFAULT, &journal))
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::Interrupted);
    let error = boot.ready().err().unwrap();
    assert_eq!(error.kind(), ErrorKind::Interrupted);
    assert_eq!(entries(&journal), ["start first", "stop first"]);
}

/// Waits on its mailbox and records the level it was told about.
struct Listener {
    seen: Arc<Mutex<Option<Level>>>,
}

impl Subsystem for Listener {
    fn name(&self) -> &str {
        "listener"
    }
    fn start(&mut self, link: &Link) -> Result<(), Failure> {
        let mailbox = link.mailbox().clone();
        let seen = Arc::clone(&self.seen);
        thread::spawn(move || {
            let notice = mailbox.wait();
            *seen.lock().unwrap() = Some(notice.level);
        });
        Ok(())
    }
    fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> {
        Ok(())
    }
}

#[test]
fn test_the_urgent_line_reaches_subsystem_mailboxes() {
    let root = Root::new("urgent");
    let seen = Arc::new(Mutex::new(None));
    let system = quiet("kernel-urgent", &root)
        .register(Listener {
            seen: Arc::clone(&seen),
        })
        .start()
        .unwrap();
    let urgent = system.kernel().urgent();
    assert_eq!(urgent.level(), Level::Normal);
    urgent.set(Level::Shed, "memory").unwrap();
    let began = Instant::now();
    while seen.lock().unwrap().is_none() && began.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(*seen.lock().unwrap(), Some(Level::Shed));
    assert_eq!(system.health().urgent(), Level::Shed);
    assert_eq!(
        urgent.set(Level::Draining, "no").err().map(|e| e.kind()),
        Some(ErrorKind::UrgentRefused)
    );
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
}

#[test]
fn test_workers_restart_then_stop_at_shutdown() {
    let root = Root::new("workers");
    let system = quiet("kernel-workers", &root).start().unwrap();
    let runs = Arc::new(AtomicU32::new(0));
    let counted = Arc::clone(&runs);
    let worker = Worker::new("flaky")
        .backoff(Backoff::Fixed(Duration::from_millis(5)))
        .spawn(system.runtime(), move |ctx| {
            if counted.fetch_add(1, Ordering::SeqCst) < 2 {
                return Err(Failure::new("not yet"));
            }
            while !ctx.should_stop() {
                ctx.heartbeat();
                ctx.park(Some(Duration::from_millis(50)));
            }
            Ok(())
        })
        .unwrap();
    let began = Instant::now();
    // The restart count rises at the second failure, a moment before the
    // worker backs off; wait until the third run has actually started.
    while (runs.load(Ordering::SeqCst) < 3 || worker.state() != WorkerState::Running)
        && began.elapsed() < Duration::from_secs(5)
    {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(worker.restarts(), 2);
    assert_eq!(system.status(), Status::Healthy);
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
    assert_eq!(worker.state(), WorkerState::Stopped);
}

#[test]
fn test_a_critical_worker_that_gives_up_stops_the_system() {
    let root = Root::new("critical");
    let system = quiet("kernel-critical", &root).start().unwrap();
    let _worker = Worker::new("essential")
        .restart(Restart::Never)
        .critical(true)
        .spawn(system.runtime(), |_ctx| Err(Failure::new("lost the disk")))
        .unwrap();
    let reason = system.wait_for_stop();
    assert!(reason.to_string().contains("essential"), "{reason}");
    assert_eq!(system.status(), Status::Unhealthy);
    let code = system.shutdown();
    assert_ne!(code, ExitCode::SUCCESS);
}

#[test]
fn test_timers_tick_and_stop_with_the_system() {
    let root = Root::new("timers");
    let system = quiet("kernel-timers", &root).start().unwrap();
    let ticks = Arc::new(AtomicU32::new(0));
    let counted = Arc::clone(&ticks);
    let timer = system
        .runtime()
        .every("tick", Duration::from_millis(5), move || {
            counted.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
    let fired = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&fired);
    let once = system
        .runtime()
        .after("once", Duration::from_millis(5), move || {
            flag.store(true, Ordering::SeqCst);
        })
        .unwrap();
    let began = Instant::now();
    while (ticks.load(Ordering::SeqCst) < 3 || !fired.load(Ordering::SeqCst))
        && began.elapsed() < Duration::from_secs(5)
    {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(ticks.load(Ordering::SeqCst) >= 3);
    assert!(fired.load(Ordering::SeqCst));
    assert!(!once.is_active());
    assert!(timer.is_active());
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
    assert!(!timer.is_active());
}

#[cfg(feature = "metrics")]
#[test]
fn test_kernel_metrics_are_recorded_in_the_systems_registry() {
    let root = Root::new("metrics");
    let file = root.write_config("[app]\nlimit = 1\n");
    let system = quiet("kernel-metrics", &root)
        .config_file(&file)
        .start()
        .unwrap();
    write_private(&file, "[app\n");
    assert!(system.reload().is_err());
    write_private(
        &file,
        "[app]
limit = 2
",
    );
    assert_eq!(system.reload().unwrap(), 2);
    let reloads = |outcome: &str| {
        let labels = system_suite::metrics::LabelSet::EMPTY.with("outcome", outcome.to_owned());
        system
            .metrics()
            .counter_with("system_suite_reloads_total", &labels)
            .get()
    };
    assert_eq!(reloads("refused"), 1);
    assert_eq!(reloads("accepted"), 1);
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
}

// ---------------------------------------------------------------------------
// Audit (boot step 13)
// ---------------------------------------------------------------------------

#[cfg(feature = "audit")]
fn audit_file(root: &Root, name: &str) -> PathBuf {
    root.path()
        .join("data")
        .join("audit")
        .join(format!("{name}.audit"))
}

#[cfg(feature = "audit")]
#[test]
fn test_audit_records_persist_and_resume_across_boots() {
    use system_suite::audit::Outcome;

    let root = Root::new("audit");
    let system = quiet("kernel-audit", &root).start().unwrap();
    let first = system
        .audit()
        .record("admin:1", "user.delete", "user:9", Outcome::Success)
        .unwrap();
    assert_eq!(first.as_u64(), 0);
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
    let path = audit_file(&root, "kernel-audit");
    assert!(path.is_file());

    // The second boot verifies the chain and appends after it; the first
    // boot's shutdown recorded the stop.
    let system = quiet("kernel-audit", &root).start().unwrap();
    let next = system
        .audit()
        .record("admin:1", "user.create", "user:10", Outcome::Success)
        .unwrap();
    assert!(next.as_u64() >= 2, "{next:?}");
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
}

#[cfg(feature = "audit")]
#[test]
fn test_a_changed_audit_record_stops_the_boot_as_tampered() {
    use system_suite::audit::Outcome;

    let root = Root::new("tamper");
    let system = quiet("kernel-tamper", &root).start().unwrap();
    let _id = system
        .audit()
        .record("admin:1", "role.grant", "user:42", Outcome::Success)
        .unwrap();
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
    let path = audit_file(&root, "kernel-tamper");
    let mut bytes = fs::read(&path).unwrap();
    let at = bytes.windows(7).position(|w| w == b"user:42").unwrap();
    bytes[at + 6] = b'3';
    fs::write(&path, &bytes).unwrap();

    let error = quiet("kernel-tamper", &root).start().err().unwrap();
    assert_eq!(error.kind(), ErrorKind::AuditTampered);
    assert_eq!(error.step(), Some(Step::Audit));
    assert_eq!(fs::read(&path).unwrap(), bytes, "the evidence was changed");
}

#[cfg(feature = "audit")]
#[test]
fn test_a_cut_audit_tail_fails_until_repair_is_allowed() {
    use system_suite::audit::Outcome;

    let root = Root::new("cut");
    let system = quiet("kernel-cut", &root).start().unwrap();
    let _id = system
        .audit()
        .record("admin:1", "key.rotate", "key:a", Outcome::Success)
        .unwrap();
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
    let path = audit_file(&root, "kernel-cut");
    let bytes = fs::read(&path).unwrap();
    fs::write(&path, &bytes[..bytes.len() - 5]).unwrap();

    let error = quiet("kernel-cut", &root).start().err().unwrap();
    assert_eq!(error.kind(), ErrorKind::Audit);
    assert!(error.action().contains("repair_tail"), "{}", error.action());

    let system = quiet("kernel-cut", &root)
        .config_str("[audit]\nrepair_tail = true\n")
        .start()
        .unwrap();
    assert!(system.audit().is_enabled());
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
    let kept: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".cut-"))
        .collect();
    assert_eq!(kept.len(), 1, "the cut bytes were not kept");
}

#[cfg(feature = "audit")]
#[test]
fn test_disabled_audit_refuses_records() {
    use system_suite::audit::Outcome;

    let root = Root::new("noaudit");
    let system = quiet("kernel-noaudit", &root)
        .disable(Member::Audit)
        .start()
        .unwrap();
    assert!(!system.audit().is_enabled());
    let error = system
        .audit()
        .record("a", "b", "c", Outcome::Success)
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::Disabled);
    assert!(!audit_file(&root, "kernel-noaudit").exists());
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
}

#[cfg(feature = "audit")]
#[test]
fn test_reloads_and_lockdowns_are_audited() {
    use system_suite::audit::Outcome;

    let root = Root::new("auditkernel");
    let file = root.write_config("[app]\nlimit = 1\n");
    let system = quiet("kernel-auditk", &root)
        .config_file(&file)
        .start()
        .unwrap();
    let before = system.audit().next_id().unwrap().as_u64();
    assert_eq!(system.reload().unwrap(), 2);
    system
        .kernel()
        .urgent()
        .set(system_suite::kernel::Level::Lockdown, "intrusion suspected")
        .unwrap();
    let after = system.audit().next_id().unwrap().as_u64();
    assert_eq!(after, before + 2);
    let _id = system
        .audit()
        .record("t", "t", "t", Outcome::Success)
        .unwrap();
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
    let text = String::from_utf8_lossy(&fs::read(audit_file(&root, "kernel-auditk")).unwrap())
        .into_owned();
    for needle in [
        "config.reload",
        "urgent.lockdown",
        "intrusion suspected",
        "stop",
    ] {
        assert!(text.contains(needle), "missing {needle}");
    }
}

// ---------------------------------------------------------------------------
// Events (boot step 14)
// ---------------------------------------------------------------------------

#[cfg(feature = "registry")]
#[test]
fn test_events_reach_handlers_and_a_panicking_handler_is_isolated() {
    use system_suite::kernel::Event;

    let root = Root::new("events");
    let file = root.write_config("[app]\nlimit = 1\n");
    let journal = Journal::default();
    let system = quiet("kernel-events", &root)
        .config_file(&file)
        .register(Recorder::new("store", Position::FIRST, &journal))
        .start()
        .unwrap();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let _guard = system.events().register_guard(move |event: &Event| {
        let line = match event {
            Event::Phase { to, .. } => format!("phase {to}"),
            Event::Reloaded { version } => format!("reloaded {version}"),
            Event::ReloadRefused { by, .. } => format!("refused by {by}"),
            Event::SubsystemStopped {
                name, met_deadline, ..
            } => {
                format!("stopped {name} {met_deadline}")
            }
            Event::Urgent { notice } => format!("urgent {}", notice.level),
            other => format!("{other:?}"),
        };
        sink.lock().unwrap().push(line);
    });
    let _boom = system
        .events()
        .register_guard(|_event: &Event| panic!("a handler that always panics"));

    assert_eq!(system.reload().unwrap(), 2);
    write_private(&file, "[app\n");
    assert!(system.reload().is_err());
    system
        .kernel()
        .urgent()
        .set(system_suite::kernel::Level::Shed, "test")
        .unwrap();
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);

    let seen = seen.lock().unwrap().clone();
    for want in [
        "reloaded 2",
        "refused by configuration",
        "urgent shed",
        "phase draining",
        "stopped store true",
        "phase stopping",
    ] {
        assert!(
            seen.iter().any(|line| line == want),
            "missing {want} in {seen:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Secrets (boot step 8, threat T2)
// ---------------------------------------------------------------------------

#[cfg(feature = "secrets")]
fn secret_text(system: &System, key: &str) -> String {
    let secret = system.config().secret(key).unwrap();
    secret
        .with(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap()
}

#[cfg(feature = "secrets")]
#[test]
fn test_t2_references_resolve_from_env_file_and_vault() {
    let root = Root::new("secrets");
    fs::create_dir_all(root.path().join("config")).unwrap();
    write_private(
        &root.path().join("config").join(".env"),
        "KERNEL_SECRET_DB=from-the-env-file\n",
    );
    let secret_file = root.path().join("api.secret");
    write_private(&secret_file, "from-a-file\n");
    let config = format!(
        "[db]\npassword = \"env:KERNEL_SECRET_DB\"\n[api]\ntoken = \"file:{}\"\n[hsm]\nkey = \"vault:hsm-key\"\n",
        secret_file.display().to_string().replace('\\', "/")
    );
    let system = quiet("kernel-secrets", &root)
        .config_str(&config)
        .on_vault(|vault| {
            let _handle = vault.register(
                "hsm-key",
                system_suite::vault::RawKey::new(b"from-the-hsm".to_vec()),
            )?;
            Ok(())
        })
        .start()
        .unwrap();
    assert_eq!(secret_text(&system, "db.password"), "from-the-env-file");
    assert_eq!(secret_text(&system, "api.token"), "from-a-file");
    assert_eq!(secret_text(&system, "hsm.key"), "from-the-hsm");
    let secret = system.config().secret("db.password").unwrap();
    let shown = format!("{secret:?}");
    assert!(!shown.contains("from-the"), "{shown}");
    assert_eq!(
        system.config().secret("db.host").err().map(|e| e.kind()),
        Some(ErrorKind::Secret)
    );
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
    // Shutdown step 7 removed the configuration's secrets from the vault.
    assert!(secret.with(|bytes| bytes.len()).is_err());
}

#[cfg(feature = "secrets")]
#[test]
fn test_t2_unresolvable_references_name_the_key_never_a_value() {
    let root = Root::new("badrefs");
    let cases = [
        "[db]\npassword = \"env:KERNEL_NEVER_SET_ANYWHERE\"\n",
        "[db]\npassword = \"file:/definitely/not/here\"\n",
        "[db]\npassword = \"file:relative/path\"\n",
        "[db]\npassword = \"vault:unregistered\"\n",
    ];
    for text in cases {
        let error = quiet("kernel-badrefs", &root)
            .config_str(text)
            .start()
            .err()
            .unwrap();
        assert_eq!(error.kind(), ErrorKind::Secret, "{text}");
        assert_eq!(error.step(), Some(Step::Secrets), "{text}");
        assert!(error.to_string().contains("db.password"), "{error}");
    }
}

#[cfg(feature = "secrets")]
#[test]
fn test_t2_plain_secrets_are_refused_unless_allowed() {
    let root = Root::new("plain");
    for key in ["password", "api_token", "client_secret"] {
        let text = format!("[db]\n{key} = \"hunter2\"\n");
        let error = quiet("kernel-plain", &root)
            .config_str(&text)
            .start()
            .err()
            .unwrap();
        assert_eq!(error.kind(), ErrorKind::PlainSecret, "{key}");
        assert!(!error.to_string().contains("hunter2"), "{error}");
    }
    let system = quiet("kernel-plain", &root)
        .config_str("[db]\npassword = \"hunter2\"\n[secrets]\nallow_plain = true\n")
        .start()
        .unwrap();
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
}

#[cfg(all(feature = "secrets", unix))]
#[test]
fn test_t2_a_secret_file_others_can_read_is_refused() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = Root::new("loosefile");
    let secret_file = root.path().join("db.secret");
    write_private(&secret_file, "hunter2");
    fs::set_permissions(&secret_file, fs::Permissions::from_mode(0o640)).unwrap();
    let error = quiet("kernel-loose", &root)
        .config_str(format!(
            "[db]\npassword = \"file:{}\"\n",
            secret_file.display()
        ))
        .start()
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::Secret);
    assert!(error.action().contains("chmod 600"), "{}", error.action());
}

#[cfg(feature = "secrets")]
#[test]
fn test_t2_a_reload_resolves_again_and_an_older_secret_keeps_its_value() {
    let root = Root::new("reloadsecret");
    let secret_file = root.path().join("db.secret");
    write_private(&secret_file, "first");
    let reference = secret_file.display().to_string().replace('\\', "/");
    let file = root.write_config(&format!("[db]\npassword = \"file:{reference}\"\n"));
    let system = quiet("kernel-reloadsecret", &root)
        .config_file(&file)
        .start()
        .unwrap();
    let before = system.config().secret("db.password").unwrap();
    write_private(&secret_file, "second");
    assert_eq!(system.reload().unwrap(), 2);
    assert_eq!(secret_text(&system, "db.password"), "second");
    assert_eq!(before.with(<[u8]>::to_vec).unwrap(), b"first");

    // A reload that fails to resolve changes nothing.
    fs::remove_file(&secret_file).unwrap();
    assert_eq!(
        system.reload().err().map(|e| e.kind()),
        Some(ErrorKind::Secret)
    );
    assert_eq!(secret_text(&system, "db.password"), "second");
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
}

#[test]
fn test_the_boot_timeline_lists_every_step_in_order() {
    let root = Root::new("timeline");
    let journal = Journal::default();
    let system = quiet("kernel-timeline", &root)
        .register(Recorder::new("store", Position::FIRST, &journal))
        .start()
        .unwrap();
    let steps: Vec<Step> = system
        .boot_timeline()
        .iter()
        .map(|(step, _)| *step)
        .collect();
    assert_eq!(steps.first(), Some(&Step::BuildInfo));
    assert!(steps.windows(2).all(|pair| pair[0] < pair[1]), "{steps:?}");
    assert!(steps.contains(&Step::Configuration));
    assert!(steps.contains(&Step::Subsystems));
    assert_eq!(steps.last(), Some(&Step::Ready));
    assert_eq!(system.shutdown(), ExitCode::SUCCESS);
}
