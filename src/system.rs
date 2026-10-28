//! `System` (the running system) and `Foundation` (its typed handles).

use std::{fmt, process::ExitCode, sync::Arc};

#[cfg(any(
    feature = "sys",
    feature = "paths",
    feature = "env",
    feature = "log",
    feature = "config",
    feature = "metrics"
))]
use crate::ErrorKind;
#[cfg(feature = "kernel")]
use crate::lifecycle::{Phase, StopReason};
use crate::{Builder, Error, boot::Parts, logging::Val};

/// The typed handles of every enabled member, built once at boot.
///
/// It is what a framework receives, what (from `0.4`) every subsystem gets
/// through its `Link`, and what [`System`] delegates to. Members are
/// resolved once; a handle access is a field read. `Clone` is one
/// reference-count increment; it is `Send + Sync`.
///
/// # Examples
///
/// ```
/// use system_suite::{Member, System};
///
/// # #[cfg(feature = "sys")] {
/// let system = System::builder("foundation-doc")
///     .process_wide(false)
///     .single_instance(false)
///     .disable(Member::Log)
///     .start()?;
/// let foundation = system.foundation().clone();
/// let worker = std::thread::spawn(move || foundation.name().to_owned());
/// assert_eq!(worker.join().ok().as_deref(), Some("foundation-doc"));
/// let code = system.shutdown();
/// # let _ = code;
/// # }
/// # Ok::<(), system_suite::Error>(())
/// ```
#[derive(Clone)]
pub struct Foundation {
    shared: Arc<Shared>,
}

struct Shared {
    name: String,
    #[cfg(feature = "sys")]
    build: Option<&'static sys_lib::BuildInfo>,
    #[cfg(feature = "sys")]
    instance: &'static sys_lib::Instance,
    #[cfg(feature = "sys")]
    guard_report: sys_lib::process::GuardReport,
    #[cfg(feature = "paths")]
    paths: path_lib::App,
    #[cfg(feature = "env")]
    env: env_lib::Env,
    #[cfg(feature = "config")]
    config: crate::Snapshot<crate::config::Config>,
    #[cfg(feature = "log")]
    log: log_io::Logger,
    #[cfg(feature = "metrics")]
    metrics: crate::metrics_handle::MetricsRef,
    #[cfg(feature = "audit")]
    audit: crate::AuditLog,
    #[cfg(feature = "registry")]
    events: Arc<registry_io::SyncRegistry<crate::kernel::Event>>,
    #[cfg(feature = "secrets")]
    vault: key_vault::KeyVault,
    #[cfg(feature = "time")]
    clock: Arc<dyn clock_lib::Clock>,
    #[cfg(feature = "ids")]
    ids: id_forge::snowflake::Snowflake,
    #[cfg(feature = "kernel")]
    kernel: crate::kernel::Kernel,
}

impl Foundation {
    /// The application's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.shared.name
    }

    /// The application's build info, as recorded by
    /// [`start!`](crate::start) or [`Builder::build_info`]; `None` when the
    /// builder was not given it (D-038).
    #[cfg(feature = "sys")]
    #[must_use]
    pub fn build(&self) -> Option<&'static sys_lib::BuildInfo> {
        self.shared.build
    }

    /// This process: instance id, start time, host, pid, user, uptime.
    #[cfg(feature = "sys")]
    #[must_use]
    pub fn instance(&self) -> &'static sys_lib::Instance {
        self.shared.instance
    }

    /// What the process guard did: the lock file, the umask, the open-file
    /// limit, and any warning.
    #[cfg(feature = "sys")]
    #[must_use]
    pub fn guard_report(&self) -> &sys_lib::process::GuardReport {
        &self.shared.guard_report
    }

    /// The application's directories.
    #[cfg(feature = "paths")]
    #[must_use]
    pub fn paths(&self) -> &path_lib::App {
        &self.shared.paths
    }

    /// The `.env` file's values, with the process environment behind them
    /// (prefix `<NAME>_`). Empty when [`Member::Env`](crate::Member::Env)
    /// is disabled.
    #[cfg(feature = "env")]
    #[must_use]
    pub fn env(&self) -> &env_lib::Env {
        &self.shared.env
    }

    /// The configuration as currently in force: a cheap clone of the
    /// latest snapshot. With [`Member::Config`](crate::Member::Config)
    /// disabled, or no file found, every section has its defaults.
    #[cfg(feature = "config")]
    #[must_use]
    pub fn config(&self) -> crate::config::Config {
        (*self.shared.config.load()).clone()
    }

    /// The configuration's snapshot, for a per-thread
    /// [`Reader`](crate::Reader) that sees each reload at its next `get`.
    #[cfg(feature = "config")]
    #[must_use]
    pub fn config_snapshot(&self) -> &crate::Snapshot<crate::config::Config> {
        &self.shared.config
    }

    /// The system's logger. With [`Member::Log`](crate::Member::Log)
    /// disabled it has no sinks.
    #[cfg(feature = "log")]
    #[must_use]
    pub fn log(&self) -> &log_io::Logger {
        &self.shared.log
    }

    /// The metrics registry: the global one of metrics-lib when the system
    /// may touch process-wide state (so its macros record into it),
    /// otherwise the system's own.
    #[cfg(feature = "metrics")]
    #[must_use]
    pub fn metrics(&self) -> &metrics_lib::MetricsCore {
        &self.shared.metrics
    }

    /// The clock: the OS clock, or the one given to [`Builder::clock`].
    #[cfg(feature = "time")]
    #[must_use]
    pub fn clock(&self) -> &dyn clock_lib::Clock {
        &*self.shared.clock
    }

    /// The application's audit chain (boot step 13). With
    /// [`Member::Audit`](crate::Member::Audit) disabled its operations fail
    /// with [`Disabled`](crate::ErrorKind::Disabled).
    #[cfg(feature = "audit")]
    #[must_use]
    pub fn audit(&self) -> &crate::AuditLog {
        &self.shared.audit
    }

    /// The vault (boot step 8): the configuration's resolved secrets and
    /// the keys registered in `on_vault`. Read secrets through
    /// [`Config::secret`](crate::config::Config::secret).
    #[cfg(feature = "secrets")]
    #[must_use]
    pub fn vault(&self) -> &key_vault::KeyVault {
        &self.shared.vault
    }

    /// The kernel's event registry (boot step 14): register callbacks for
    /// [`kernel::Event`](crate::kernel::Event)s. With
    /// [`Member::Registry`](crate::Member::Registry) disabled the kernel
    /// never notifies it.
    #[cfg(feature = "registry")]
    #[must_use]
    pub fn events(&self) -> &Arc<registry_io::SyncRegistry<crate::kernel::Event>> {
        &self.shared.events
    }

    /// The Snowflake generator for entity ids, with the worker id (and
    /// epoch, if set) from `[ids]`. Use `try_next_id`: `next_id` panics on
    /// clock skew.
    #[cfg(feature = "ids")]
    #[must_use]
    pub fn ids(&self) -> &id_forge::snowflake::Snowflake {
        &self.shared.ids
    }

    /// The kernel's control handle.
    #[cfg(feature = "kernel")]
    #[must_use]
    pub fn kernel(&self) -> &crate::kernel::Kernel {
        &self.shared.kernel
    }

    /// The kernel's runtime: supervised workers and timers.
    #[cfg(feature = "kernel")]
    #[must_use]
    pub fn runtime(&self) -> &crate::runtime::Runtime {
        self.shared.kernel.runtime()
    }
}

impl fmt::Debug for Foundation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Foundation")
            .field("name", &self.shared.name)
            .finish_non_exhaustive()
    }
}

/// The running system: it owns the lifecycle, the started members, and the
/// application's subsystems.
///
/// `Send + Sync`; not `Clone` (exactly one owner runs the shutdown).
/// Dropping it without calling [`run`](Self::run) or
/// [`shutdown`](Self::shutdown) runs the shutdown order at once.
///
/// # Examples
///
/// ```no_run
/// # #[cfg(all(feature = "kernel", feature = "log"))]
/// fn main() -> std::process::ExitCode {
///     let system = match system_suite::start!() {
///         Ok(system) => system,
///         Err(error) => return error.report(),
///     };
///     system_suite::log::info!(system.log(), "ready");
///     system.run()
/// }
/// # #[cfg(not(all(feature = "kernel", feature = "log")))]
/// # fn main() {}
/// ```
#[must_use = "dropping a System shuts it down at once; call run()"]
pub struct System {
    foundation: Foundation,
    parts: Parts,
    finished: bool,
}

impl System {
    /// A builder for the application `name` (same as [`Builder::new`]).
    pub fn builder(name: &str) -> Builder {
        Builder::new(name)
    }

    /// Step 17: announces readiness and hands over the running system.
    pub(crate) fn ready(mut parts: Parts, foundation: Foundation) -> System {
        let boot_ms = crate::boot::millis(parts.began.elapsed());
        #[cfg(feature = "signals")]
        if let Some(intake) = &parts.signals {
            let dump = foundation.clone();
            intake.set_dump(Box::new(move || write_dump(&dump)));
            #[cfg(feature = "config")]
            if let Some(reloader) = &parts.reloader {
                let reloader = Arc::clone(reloader);
                intake.set_reload(Box::new(move || {
                    // The outcome is logged and counted by the reloader.
                    drop(reloader.reload());
                }));
            }
        }
        #[cfg(feature = "kernel")]
        {
            parts.kernel.set_phase(Phase::Ready);
            if let Some(metrics) = parts.kernel.metrics() {
                metrics.boot_milliseconds(boot_ms);
            }
        }
        #[cfg(feature = "globals")]
        if parts.settings.process_wide && !parts.settings.is_disabled(crate::Member::Globals) {
            crate::globals::publish(&foundation);
            parts.published = true;
        }
        parts.notify("READY=1");
        parts.notify("STATUS=ready");
        parts.mark_ready();
        #[cfg(feature = "kernel")]
        parts.start_watchdog();
        parts.log.info("ready", &[("boot_ms", Val::U64(boot_ms))]);
        System {
            foundation,
            parts,
            finished: false,
        }
    }

    /// The typed handles, to hand to a framework or a thread.
    #[must_use]
    pub fn foundation(&self) -> &Foundation {
        &self.foundation
    }

    /// The application's name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.foundation.name()
    }

    /// See [`Foundation::build`].
    #[cfg(feature = "sys")]
    #[must_use]
    pub fn build(&self) -> Option<&'static sys_lib::BuildInfo> {
        self.foundation.build()
    }

    /// See [`Foundation::instance`].
    #[cfg(feature = "sys")]
    #[must_use]
    pub fn instance(&self) -> &'static sys_lib::Instance {
        self.foundation.instance()
    }

    /// See [`Foundation::guard_report`].
    #[cfg(feature = "sys")]
    #[must_use]
    pub fn guard_report(&self) -> &sys_lib::process::GuardReport {
        self.foundation.guard_report()
    }

    /// See [`Foundation::paths`].
    #[cfg(feature = "paths")]
    #[must_use]
    pub fn paths(&self) -> &path_lib::App {
        self.foundation.paths()
    }

    /// See [`Foundation::env`].
    #[cfg(feature = "env")]
    #[must_use]
    pub fn env(&self) -> &env_lib::Env {
        self.foundation.env()
    }

    /// See [`Foundation::log`].
    #[cfg(feature = "log")]
    #[must_use]
    pub fn log(&self) -> &log_io::Logger {
        self.foundation.log()
    }

    /// See [`Foundation::clock`].
    #[cfg(feature = "time")]
    #[must_use]
    pub fn clock(&self) -> &dyn clock_lib::Clock {
        self.foundation.clock()
    }

    /// See [`Foundation::audit`].
    #[cfg(feature = "audit")]
    #[must_use]
    pub fn audit(&self) -> &crate::AuditLog {
        self.foundation.audit()
    }

    /// See [`Foundation::vault`].
    #[cfg(feature = "secrets")]
    #[must_use]
    pub fn vault(&self) -> &key_vault::KeyVault {
        self.foundation.vault()
    }

    /// See [`Foundation::events`].
    #[cfg(feature = "registry")]
    #[must_use]
    pub fn events(&self) -> &Arc<registry_io::SyncRegistry<crate::kernel::Event>> {
        self.foundation.events()
    }

    /// See [`Foundation::ids`].
    #[cfg(feature = "ids")]
    #[must_use]
    pub fn ids(&self) -> &id_forge::snowflake::Snowflake {
        self.foundation.ids()
    }

    /// See [`Foundation::kernel`].
    #[cfg(feature = "kernel")]
    #[must_use]
    pub fn kernel(&self) -> &crate::kernel::Kernel {
        self.foundation.kernel()
    }

    /// See [`Foundation::config`].
    #[cfg(feature = "config")]
    #[must_use]
    pub fn config(&self) -> crate::config::Config {
        self.foundation.config()
    }

    /// See [`Foundation::config_snapshot`].
    #[cfg(feature = "config")]
    #[must_use]
    pub fn config_snapshot(&self) -> &crate::Snapshot<crate::config::Config> {
        self.foundation.config_snapshot()
    }

    /// See [`Foundation::metrics`].
    #[cfg(feature = "metrics")]
    #[must_use]
    pub fn metrics(&self) -> &metrics_lib::MetricsCore {
        self.foundation.metrics()
    }

    /// See [`Foundation::runtime`].
    #[cfg(feature = "kernel")]
    #[must_use]
    pub fn runtime(&self) -> &crate::runtime::Runtime {
        self.foundation.runtime()
    }

    /// The current lifecycle phase.
    #[cfg(feature = "kernel")]
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.foundation.kernel().phase()
    }

    /// Asks for a clean stop exactly as a SIGTERM would. Returns `false` if
    /// a stop was already requested (the first reason is kept). Never
    /// blocks.
    #[cfg(feature = "kernel")]
    pub fn request_stop(&self, reason: &'static str) -> bool {
        self.foundation.kernel().request_stop(reason)
    }

    /// Reads the configuration again, validates it, and asks every
    /// subsystem to prepare its change; only if all of them succeed is the
    /// new configuration published and every change committed. Returns the
    /// new configuration version. On any failure nothing changes and the
    /// old configuration stays in force.
    ///
    /// # Errors
    ///
    /// [`ConfigNotFound`](ErrorKind::ConfigNotFound),
    /// [`ConfigInsecure`](ErrorKind::ConfigInsecure),
    /// [`ConfigInvalid`](ErrorKind::ConfigInvalid), and
    /// [`Secret`](ErrorKind::Secret) as at boot step 5;
    /// [`ReloadRefused`](ErrorKind::ReloadRefused) when a subsystem refused;
    /// [`Stopping`](ErrorKind::Stopping) once a stop was requested.
    #[cfg(all(feature = "kernel", feature = "config"))]
    pub fn reload(&self) -> Result<u64, Error> {
        match &self.parts.reloader {
            Some(reloader) => reloader.reload(),
            None => Err(Error::new(
                ErrorKind::Kernel,
                "internal: the reloader was not set up by its boot step",
                "report this as a bug in system-suite",
            )),
        }
    }

    /// Every member, subsystem, worker, and the timer thread. Allocates;
    /// for health endpoints and the dump signal.
    #[cfg(feature = "kernel")]
    #[must_use]
    pub fn health(&self) -> crate::kernel::HealthReport {
        self.foundation.kernel().health()
    }

    /// The aggregate status only. Does not allocate.
    #[cfg(feature = "kernel")]
    #[must_use]
    pub fn status(&self) -> crate::kernel::Status {
        self.foundation.kernel().status()
    }

    /// Parks the calling thread with zero CPU until a stop is requested (a
    /// stop signal, a console event, or `Kernel::request_stop`), then runs
    /// the shutdown order and returns the exit code: `0` for a clean stop,
    /// `79` if a step missed the shutdown deadline.
    #[cfg(feature = "kernel")]
    pub fn run(self) -> ExitCode {
        let reason = self.wait_for_stop();
        let cause = reason.to_string();
        self.finish(reason.exit_code(), &cause)
    }

    /// Parks the calling thread until a stop is requested and returns why.
    /// Several threads may wait. A framework that has work to do between
    /// the stop request and the shutdown calls this, then
    /// [`shutdown`](Self::shutdown).
    #[cfg(feature = "kernel")]
    #[must_use = "the stop reason says why the system is stopping"]
    pub fn wait_for_stop(&self) -> StopReason {
        self.foundation.kernel().wait_for_stop()
    }

    /// Runs the shutdown order now, whether or not a stop was requested,
    /// and returns the exit code.
    pub fn shutdown(self) -> ExitCode {
        #[cfg(feature = "kernel")]
        let (code, cause) = match self.foundation.kernel().stop_reason() {
            Some(reason) => (reason.exit_code(), reason.to_string()),
            None => (crate::exit::OK, "shutdown called".to_owned()),
        };
        #[cfg(not(feature = "kernel"))]
        let (code, cause) = (crate::exit::OK, "shutdown called".to_owned());
        self.finish(code, &cause)
    }

    fn finish(mut self, code: u8, cause: &str) -> ExitCode {
        self.finished = true;
        ExitCode::from(self.parts.teardown(code, cause))
    }
}

impl Drop for System {
    fn drop(&mut self) {
        if !self.finished {
            self.finished = true;
            let _code = self
                .parts
                .teardown(crate::exit::OK, "the System was dropped");
        }
    }
}

impl fmt::Debug for System {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("System")
            .field("name", &self.foundation.name())
            .finish_non_exhaustive()
    }
}

/// Builds the foundation from the parts started by steps 1 to 15.
pub(crate) fn foundation_from(parts: &mut Parts) -> Result<Foundation, Error> {
    #[cfg(any(
        feature = "sys",
        feature = "paths",
        feature = "env",
        feature = "log",
        feature = "config",
        feature = "metrics"
    ))]
    let missing = |what: &str| {
        Error::new(
            ErrorKind::Kernel,
            format!("internal: {what} was not set up by its boot step"),
            "report this as a bug in system-suite",
        )
    };
    Ok(Foundation {
        shared: Arc::new(Shared {
            name: parts.name().to_owned(),
            #[cfg(feature = "sys")]
            build: parts.settings.build,
            #[cfg(feature = "sys")]
            instance: parts.instance.ok_or_else(|| missing("the instance"))?,
            #[cfg(feature = "sys")]
            guard_report: parts
                .guard
                .as_ref()
                .map(|guard| guard.report().clone())
                .ok_or_else(|| missing("the guard"))?,
            #[cfg(feature = "paths")]
            paths: parts.app.clone().ok_or_else(|| missing("the paths"))?,
            #[cfg(feature = "env")]
            env: parts
                .env
                .clone()
                .ok_or_else(|| missing("the environment"))?,
            #[cfg(feature = "config")]
            config: parts
                .config
                .clone()
                .ok_or_else(|| missing("the configuration"))?,
            #[cfg(feature = "log")]
            log: parts.logger.clone().ok_or_else(|| missing("the logger"))?,
            #[cfg(feature = "metrics")]
            metrics: parts
                .metrics
                .clone()
                .ok_or_else(|| missing("the metrics registry"))?,
            #[cfg(feature = "audit")]
            audit: parts
                .audit
                .clone()
                .unwrap_or_else(crate::AuditLog::disabled),
            #[cfg(feature = "registry")]
            events: parts
                .events
                .clone()
                .unwrap_or_else(|| Arc::new(registry_io::SyncRegistry::new())),
            #[cfg(feature = "secrets")]
            vault: parts
                .secrets
                .as_ref()
                .map(|state| state.vault().clone())
                .ok_or_else(|| missing("the vault"))?,
            #[cfg(feature = "time")]
            clock: parts
                .settings
                .clock
                .take()
                .unwrap_or_else(|| Arc::new(clock_lib::SystemClock::new())),
            #[cfg(feature = "ids")]
            ids: match parts.sections.ids.epoch_ms {
                Some(epoch) => {
                    id_forge::snowflake::Snowflake::with_epoch(parts.sections.ids.worker, epoch)
                }
                None => id_forge::snowflake::Snowflake::new(parts.sections.ids.worker),
            },
            #[cfg(feature = "kernel")]
            kernel: parts.kernel.clone(),
        }),
    })
}

/// The dump signal's output: one log line with the system's state and its
/// health, component by component.
#[cfg(feature = "signals")]
fn write_dump(foundation: &Foundation) {
    #[cfg(feature = "log")]
    {
        let report = foundation.kernel().health();
        #[cfg(feature = "sys")]
        let instance = foundation.instance().to_string();
        #[cfg(not(feature = "sys"))]
        let instance = String::from("unknown");
        let mut health = String::new();
        // Writing into a String cannot fail.
        let _written = report.write_kv(&mut health);
        log_io::info!(
            foundation.log(),
            "state",
            phase = report.phase().name(),
            status = report.status().name(),
            urgent = report.urgent().name(),
            instance = instance.as_str(),
            health = health.as_str()
        );
    }
    #[cfg(not(feature = "log"))]
    let _no_log = foundation;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErrorKind as Kind, Member};

    fn quiet(name: &str) -> Builder {
        let builder = Builder::new(name).process_wide(false).disable(Member::Log);
        #[cfg(feature = "sys")]
        let builder = builder.single_instance(false);
        builder
    }

    #[test]
    fn test_system_starts_and_shuts_down_cleanly() {
        let system = quiet("system-unit");
        let system = system.start();
        assert!(system.is_ok(), "{:?}", system.as_ref().err());
        if let Ok(system) = system {
            assert_eq!(system.name(), "system-unit");
            #[cfg(feature = "kernel")]
            assert_eq!(system.phase(), Phase::Ready);
            assert_eq!(system.shutdown(), ExitCode::SUCCESS);
        }
    }

    #[test]
    fn test_two_systems_coexist_without_process_wide_state() {
        let first = quiet("system-unit-a").start();
        let second = quiet("system-unit-b").start();
        assert!(first.is_ok() && second.is_ok());
    }

    #[cfg(feature = "kernel")]
    #[test]
    fn test_run_returns_after_request_stop() {
        let system = quiet("system-unit-run").start();
        assert!(system.is_ok());
        if let Ok(system) = system {
            let kernel = system.kernel().clone();
            let stopper = std::thread::spawn(move || kernel.request_stop("unit test"));
            assert_eq!(system.run(), ExitCode::SUCCESS);
            assert!(stopper.join().unwrap_or(false));
        }
    }

    #[test]
    fn test_invalid_name_is_a_setup_error() {
        let error = quiet("not valid").start().err();
        assert_eq!(error.map(|e| e.kind()), Some(Kind::Setup));
    }

    #[test]
    fn test_system_and_foundation_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<System>();
        assert_send_sync::<Foundation>();
    }
}
