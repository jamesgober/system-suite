//! Crash capture: the panic hook (boot step 11).
//!
//! On a panic anywhere in the process the hook logs the panic, flushes the
//! logger and syncs the audit chain (each with a short bound, so a stuck
//! sink cannot hold the process), writes an owner-only crash report to the
//! log directory, and then runs the hook that was installed before it, so
//! the process ends as the application's profile decides (threats T5 and
//! T9).

use std::{
    fs,
    io::{self, Write as _},
    panic::{self, PanicHookInfo},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::logging::{LogSlot, Val};

/// How long the hook waits for the logger to accept and flush the panic
/// record before it gives up and writes the report without it.
#[cfg(feature = "log")]
const FLUSH_BOUND: std::time::Duration = std::time::Duration::from_secs(1);

type PanicHook = Box<dyn Fn(&PanicHookInfo<'_>) + Sync + Send + 'static>;

/// What the hook needs to know, fixed when it is installed.
pub(crate) struct CrashSettings {
    pub(crate) app: String,
    pub(crate) dir: Option<PathBuf>,
    pub(crate) max_reports: u32,
    pub(crate) log: LogSlot,
    #[cfg(feature = "log")]
    pub(crate) recent: Option<Arc<crate::boot::RecentLog>>,
    #[cfg(feature = "sys")]
    pub(crate) build: Option<&'static sys_lib::BuildInfo>,
    #[cfg(feature = "sys")]
    pub(crate) instance: Option<&'static sys_lib::Instance>,
}

struct CrashState {
    settings: CrashSettings,
    reports: AtomicU32,
    /// The audit chain, opened at step 13, after the hook (step 11).
    #[cfg(feature = "audit")]
    audit: std::sync::OnceLock<crate::AuditLog>,
}

/// The installed hook. Uninstalling it puts the previous hook back.
pub(crate) struct CrashHook {
    previous: Arc<Mutex<Option<PanicHook>>>,
    #[cfg(feature = "audit")]
    state: Arc<CrashState>,
}

impl CrashHook {
    /// Installs the hook in front of whatever hook is installed now.
    pub(crate) fn install(settings: CrashSettings) -> CrashHook {
        let previous: Arc<Mutex<Option<PanicHook>>> =
            Arc::new(Mutex::new(Some(panic::take_hook())));
        let state = Arc::new(CrashState {
            settings,
            reports: AtomicU32::new(0),
            #[cfg(feature = "audit")]
            audit: std::sync::OnceLock::new(),
        });
        #[cfg(feature = "audit")]
        let hook_state = Arc::clone(&state);
        #[cfg(not(feature = "audit"))]
        let hook_state = state;
        let chained = Arc::clone(&previous);
        panic::set_hook(Box::new(move |info| {
            capture(&hook_state, info);
            let previous = chained.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(hook) = previous.as_ref() {
                hook(info);
            }
        }));
        CrashHook {
            previous,
            #[cfg(feature = "audit")]
            state,
        }
    }

    /// Gives the hook the audit chain to sync on a panic (step 13).
    #[cfg(feature = "audit")]
    pub(crate) fn set_audit(&self, audit: crate::AuditLog) {
        let _first = self.state.audit.set(audit).is_ok();
    }

    /// Removes the hook and puts the previous one back. During a panic the
    /// hook cannot be changed (std forbids it), so it stays installed; it
    /// still works, because everything it uses is reference-counted.
    pub(crate) fn uninstall(self) {
        if thread::panicking() {
            return;
        }
        drop(panic::take_hook());
        let previous = self
            .previous
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(hook) = previous {
            panic::set_hook(hook);
        }
    }
}

fn capture(state: &CrashState, info: &PanicHookInfo<'_>) {
    let message = panic_message(info);
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "unknown".to_owned());
    let current = thread::current();
    let thread_name = current.name().unwrap_or("unnamed").to_owned();

    log_panic(&state.settings.log, &message, &location, &thread_name);
    #[cfg(feature = "audit")]
    if let Some(audit) = state.audit.get() {
        sync_audit(audit.clone());
    }

    let count = state.reports.fetch_add(1, Ordering::Relaxed);
    if count >= state.settings.max_reports {
        return;
    }
    let Some(dir) = state.settings.dir.as_deref() else {
        return;
    };
    let report = render(&state.settings, &message, &location, &thread_name);
    if let Err(error) = write_report(dir, &report_name(&state.settings, count), &report) {
        // The hook cannot return an error; the log is the only place left.
        report_failure(&state.settings.log, &error.to_string());
    }
}

fn panic_message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "a panic with a non-text payload".to_owned()
    }
}

/// Logs the panic on a helper thread and waits at most [`FLUSH_BOUND`]: if
/// the panic happened while a sink held its lock on this thread, logging
/// here would wait forever.
#[cfg(feature = "log")]
fn log_panic(log: &LogSlot, message: &str, location: &str, thread_name: &str) {
    let Some(logger) = log.get().cloned() else {
        return;
    };
    let (message, location, thread_name) = (
        message.to_owned(),
        location.to_owned(),
        thread_name.to_owned(),
    );
    let (done, finished) = std::sync::mpsc::channel::<()>();
    let spawned = thread::Builder::new()
        .name("system-suite-crash".to_owned())
        .spawn(move || {
            logger.log(
                log_io::Level::Error,
                "panic",
                &[
                    log_io::Field::new("message", message.as_str()),
                    log_io::Field::new("location", location.as_str()),
                    log_io::Field::new("thread", thread_name.as_str()),
                ],
            );
            // A flush error has nowhere to go from inside the panic hook.
            drop(logger.flush());
            // The hook may have stopped waiting; then nobody needs the signal.
            let _delivered = done.send(());
        });
    if spawned.is_ok() {
        // A timeout means the logger is stuck; the report is still written.
        let _flushed = finished.recv_timeout(FLUSH_BOUND);
    }
}

/// Logs that the crash report could not be written, on a helper thread
/// with the same bound as the panic line: if the panic happened inside a
/// log sink, logging on this thread would wait forever.
fn report_failure(log: &LogSlot, error: &str) {
    let (log, error) = (log.clone(), error.to_owned());
    let (done, finished) = std::sync::mpsc::channel::<()>();
    let spawned = thread::Builder::new()
        .name("system-suite-crash-log".to_owned())
        .spawn(move || {
            log.error(
                "crash report could not be written",
                &[("error", Val::Str(&error))],
            );
            let _delivered = done.send(());
        });
    if spawned.is_ok() {
        let _logged = finished.recv_timeout(std::time::Duration::from_secs(1));
    }
}

/// Syncs the audit chain on a helper thread and waits at most one second:
/// if the panic happened while this thread held the chain (inside
/// `record`), syncing here would wait forever.
#[cfg(feature = "audit")]
fn sync_audit(audit: crate::AuditLog) {
    let (done, finished) = std::sync::mpsc::channel::<()>();
    let spawned = thread::Builder::new()
        .name("system-suite-crash-audit".to_owned())
        .spawn(move || {
            // A sync error has nowhere to go from inside the panic hook.
            let _synced = audit.sync();
            let _delivered = done.send(());
        });
    if spawned.is_ok() {
        let _synced = finished.recv_timeout(std::time::Duration::from_secs(1));
    }
}

#[cfg(not(feature = "log"))]
fn log_panic(_log: &LogSlot, _message: &str, _location: &str, _thread_name: &str) {}

fn report_name(settings: &CrashSettings, count: u32) -> String {
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("crash-{}-{unix}-{count}.txt", instance_label(settings))
}

#[cfg(feature = "sys")]
fn instance_label(settings: &CrashSettings) -> String {
    match settings.instance {
        Some(instance) => instance.id().to_string(),
        None => std::process::id().to_string(),
    }
}

#[cfg(not(feature = "sys"))]
fn instance_label(_settings: &CrashSettings) -> String {
    std::process::id().to_string()
}

/// The report: build and instance info, the panic message, its location,
/// its thread, and the last log lines, already redacted by the logger
/// (threat T9). The application's own panic message is the one text it
/// controls.
fn render(settings: &CrashSettings, message: &str, location: &str, thread_name: &str) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("system-suite crash report\n");
    out.push_str(&format!("app: {}\n", settings.app));
    describe_sys(settings, &mut out);
    out.push_str(&format!("pid: {}\n", std::process::id()));
    out.push_str(&format!("thread: {thread_name}\n"));
    out.push_str(&format!("location: {location}\n"));
    out.push_str(&format!("panic: {message}\n"));
    recent_lines(settings, &mut out);
    out
}

/// The last log lines, read without waiting: if the panic happened while
/// the ring was being written on this thread, the report says so instead
/// of deadlocking.
#[cfg(feature = "log")]
fn recent_lines(settings: &CrashSettings, out: &mut String) {
    let Some(recent) = settings.recent.as_ref() else {
        return;
    };
    out.push_str("\nrecent log lines (oldest first):\n");
    match recent.try_write_to(out) {
        Ok(true) => {}
        Ok(false) => out.push_str("(the log was being written when the panic happened)\n"),
        Err(_) => out.push_str("(the recent lines could not be written)\n"),
    }
}

#[cfg(not(feature = "log"))]
fn recent_lines(_settings: &CrashSettings, _out: &mut String) {}

#[cfg(feature = "sys")]
fn describe_sys(settings: &CrashSettings, out: &mut String) {
    if let Some(build) = settings.build {
        out.push_str(&format!("build: {build}\n"));
    }
    if let Some(instance) = settings.instance {
        out.push_str(&format!("instance: {instance}\n"));
    }
}

#[cfg(not(feature = "sys"))]
fn describe_sys(_settings: &CrashSettings, _out: &mut String) {}

/// Writes the report as a new, owner-only file. It never overwrites one.
fn write_report(dir: &Path, name: &str, report: &str) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    restrict(&mut options);
    let mut file = options.open(dir.join(name))?;
    file.write_all(report.as_bytes())?;
    file.sync_all()
}

#[cfg(unix)]
fn restrict(options: &mut fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt as _;
    options.mode(0o600);
}

/// Windows has no mode bits: the file inherits the log directory's ACL,
/// which the suite creates owner-only through path-lib.
#[cfg(windows)]
fn restrict(_options: &mut fs::OpenOptions) {}

#[cfg(not(any(unix, windows)))]
fn restrict(_options: &mut fs::OpenOptions) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(dir: Option<PathBuf>) -> CrashSettings {
        CrashSettings {
            app: "crash-test".to_owned(),
            dir,
            max_reports: 2,
            log: LogSlot::default(),
            #[cfg(feature = "log")]
            recent: None,
            #[cfg(feature = "sys")]
            build: None,
            #[cfg(feature = "sys")]
            instance: None,
        }
    }

    #[test]
    fn test_render_contains_only_the_documented_fields() {
        let report = render(&settings(None), "boom", "src/main.rs:1:1", "main");
        assert!(report.starts_with("system-suite crash report\n"));
        assert!(report.contains("app: crash-test\n"));
        assert!(report.contains("panic: boom\n"));
        assert!(report.contains("location: src/main.rs:1:1\n"));
        assert!(report.contains("thread: main\n"));
    }

    #[test]
    fn test_write_report_is_new_and_owner_only() {
        let dir = std::env::temp_dir().join(format!("system-suite-crash-{}", std::process::id()));
        fs::create_dir_all(&dir).ok();
        let name = "crash-unit-0.txt";
        fs::remove_file(dir.join(name)).ok();
        assert!(write_report(&dir, name, "report").is_ok());
        // Never overwrites an existing report.
        assert!(write_report(&dir, name, "again").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(dir.join(name)).map(|m| m.permissions().mode() & 0o777);
            assert_eq!(mode.ok(), Some(0o600));
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_report_name_uses_pid_without_instance() {
        let name = report_name(&settings(None), 3);
        assert!(name.starts_with(&format!("crash-{}-", std::process::id())));
        assert!(name.ends_with("-3.txt"));
    }
}
