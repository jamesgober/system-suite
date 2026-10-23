//! The boot order and its unwinding (`docs/BOOT-ORDER.md`).
//!
//! Each step is a function over the parts started so far. The driver runs
//! them in order, stops at the first failure or at a stop request, and the
//! same teardown that ends a running system then stops whatever started.

use std::time::{Duration, Instant};

#[cfg(any(
    feature = "env",
    feature = "log",
    feature = "sys",
    feature = "paths",
    feature = "signals"
))]
use crate::Member;
#[cfg(feature = "kernel")]
use crate::kernel::Kernel;
#[cfg(feature = "kernel")]
use crate::lifecycle::Phase;
#[cfg(any(feature = "log", feature = "signals"))]
use crate::shutdown::{Outcome, run_with_deadline};
use crate::{
    Error, ErrorKind, Step, System,
    builder::{Settings, validate},
    logging::{LogSlot, Val},
};

/// A boot step.
pub(crate) type StepFn = fn(&mut Parts) -> Result<(), Error>;

/// Everything started so far. The teardown stops it in reverse.
pub(crate) struct Parts {
    pub(crate) settings: Settings,
    pub(crate) log: LogSlot,
    pub(crate) began: Instant,
    timeline: Vec<(Step, Duration)>,
    #[cfg(feature = "kernel")]
    pub(crate) kernel: Kernel,
    #[cfg(feature = "sys")]
    pub(crate) instance: Option<&'static sys_lib::Instance>,
    #[cfg(feature = "sys")]
    pub(crate) guard: Option<sys_lib::process::Guard>,
    #[cfg(feature = "paths")]
    pub(crate) app: Option<path_lib::App>,
    #[cfg(feature = "env")]
    pub(crate) env: Option<env_lib::Env>,
    #[cfg(feature = "log")]
    pub(crate) logger: Option<log_io::Logger>,
    #[cfg(feature = "signals")]
    pub(crate) signals: Option<crate::kernel::signals::SignalIntake>,
    #[cfg(feature = "kernel")]
    pub(crate) crash: Option<crate::kernel::crash::CrashHook>,
    notified_ready: bool,
}

impl Parts {
    pub(crate) fn new(settings: Settings) -> Parts {
        Parts {
            settings,
            log: LogSlot::default(),
            began: Instant::now(),
            timeline: Vec::with_capacity(17),
            #[cfg(feature = "kernel")]
            kernel: Kernel::new(),
            #[cfg(feature = "sys")]
            instance: None,
            #[cfg(feature = "sys")]
            guard: None,
            #[cfg(feature = "paths")]
            app: None,
            #[cfg(feature = "env")]
            env: None,
            #[cfg(feature = "log")]
            logger: None,
            #[cfg(feature = "signals")]
            signals: None,
            #[cfg(feature = "kernel")]
            crash: None,
            notified_ready: false,
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.settings.name
    }

    /// Records that readiness was announced, so the teardown announces the
    /// stop too.
    pub(crate) fn mark_ready(&mut self) {
        self.notified_ready = true;
    }

    /// Stops everything that started, in the shutdown order, within the
    /// shutdown deadline. Returns the exit code: `code`, or 79 if a step was
    /// abandoned at the deadline.
    pub(crate) fn teardown(&mut self, code: u8, cause: &str) -> u8 {
        let deadline = Instant::now()
            .checked_add(self.settings.shutdown_deadline)
            .unwrap_or_else(Instant::now);
        let mut code = code;
        #[cfg(feature = "kernel")]
        self.kernel.set_phase(Phase::Draining);
        if self.notified_ready {
            self.notify("STOPPING=1");
        }
        self.log.info("stopping", &[("cause", Val::Str(cause))]);
        // From 0.4: subsystems stop here, in reverse of their start order.
        #[cfg(feature = "kernel")]
        self.kernel.set_phase(Phase::Stopping);
        // From 0.4: the runtime's workers and timers stop here.
        #[cfg(feature = "signals")]
        if let Some(intake) = self.signals.take()
            && let Some(thread) = intake.uninstall()
        {
            let outcome = run_with_deadline("signals", deadline, move || {
                // The intake thread only reads signals; a panic there has
                // already been reported by the crash hook.
                drop(thread.join());
            });
            code = self.account("signals", outcome, code);
        }
        code = self.close_logging(deadline, code);
        #[cfg(feature = "kernel")]
        self.kernel.set_phase(Phase::Stopped);
        #[cfg(feature = "kernel")]
        if let Some(crash) = self.crash.take() {
            crash.uninstall();
        }
        #[cfg(feature = "sys")]
        drop(self.guard.take());
        code
    }

    #[cfg(feature = "signals")]
    fn account(&self, step: &'static str, outcome: Outcome, code: u8) -> u8 {
        match outcome {
            Outcome::Done(_) => code,
            Outcome::Panicked => {
                self.log
                    .error("shutdown step panicked", &[("step", Val::Str(step))]);
                code
            }
            Outcome::Abandoned => {
                self.log.error(
                    "shutdown step abandoned at the deadline",
                    &[("step", Val::Str(step))],
                );
                crate::exit::DEADLINE
            }
        }
    }

    #[cfg(feature = "log")]
    fn close_logging(&mut self, deadline: Instant, code: u8) -> u8 {
        let Some(logger) = self.logger.take() else {
            return code;
        };
        let took = self.began.elapsed();
        self.log.info(
            "stopped",
            &[
                ("exit_code", Val::U64(u64::from(code))),
                ("uptime_ms", Val::U64(millis(took))),
            ],
        );
        let outcome = run_with_deadline("logging", deadline, move || {
            // Errors here can no longer be logged; the exit code reports a
            // missed deadline, and the sinks report their own write errors.
            drop(logger.flush());
            drop(logger.shutdown());
        });
        match outcome {
            Outcome::Abandoned => crate::exit::DEADLINE,
            _ => code,
        }
    }

    #[cfg(not(feature = "log"))]
    fn close_logging(&mut self, _deadline: Instant, code: u8) -> u8 {
        code
    }

    /// Sends a systemd notification when the system may touch process-wide
    /// state; a failure is logged, never fatal.
    pub(crate) fn notify(&self, state: &str) {
        if !self.settings.process_wide {
            return;
        }
        #[cfg(feature = "kernel")]
        if let Err(error) = crate::kernel::notify::notify(state) {
            self.log.warn(
                "systemd notification failed",
                &[
                    ("state", Val::Str(state)),
                    ("error", Val::Str(&error.to_string())),
                ],
            );
        }
        #[cfg(not(feature = "kernel"))]
        let _no_kernel = state;
    }

    /// Writes the boot timeline: one line per step that ran.
    #[cfg(feature = "log")]
    fn log_timeline(&self) {
        for (step, took) in &self.timeline {
            self.log.info(
                "boot step",
                &[
                    ("step", Val::U64(u64::from(step.number()))),
                    ("name", Val::Str(step.name())),
                    ("micros", Val::U64(micros(*took))),
                ],
            );
        }
    }
}

#[cfg(feature = "log")]
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(feature = "log")]
fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// Runs `steps` in order. Before each step, and after the last, a stop
/// request ends the boot with [`ErrorKind::Interrupted`] (threat T8).
pub(crate) fn run_steps(parts: &mut Parts, steps: &[(Step, StepFn)]) -> Result<(), Error> {
    for &(step, run) in steps {
        check_interrupted(parts, step)?;
        let began = Instant::now();
        run(parts).map_err(|error| error.at(step))?;
        parts.timeline.push((step, began.elapsed()));
    }
    check_interrupted(parts, Step::Ready)
}

#[cfg(feature = "kernel")]
fn check_interrupted(parts: &Parts, step: Step) -> Result<(), Error> {
    match parts.kernel.stop_reason() {
        Some(reason) => Err(Error::new(
            ErrorKind::Interrupted,
            format!("{reason} arrived during boot; everything that started was stopped"),
            "nothing: the stop was asked for",
        )
        .at(step)),
        None => Ok(()),
    }
}

#[cfg(not(feature = "kernel"))]
fn check_interrupted(_parts: &Parts, _step: Step) -> Result<(), Error> {
    Ok(())
}

/// The boot steps compiled into this build, in order.
pub(crate) fn steps() -> Vec<(Step, StepFn)> {
    let mut steps: Vec<(Step, StepFn)> = Vec::with_capacity(17);
    steps.push((Step::BuildInfo, step_build_info));
    #[cfg(feature = "sys")]
    steps.push((Step::Instance, step_instance));
    #[cfg(feature = "paths")]
    steps.push((Step::Paths, step_paths));
    #[cfg(feature = "env")]
    steps.push((Step::Environment, step_environment));
    #[cfg(feature = "sys")]
    steps.push((Step::Guard, step_guard));
    #[cfg(feature = "signals")]
    steps.push((Step::Signals, step_signals));
    #[cfg(feature = "paths")]
    steps.push((Step::Directories, step_directories));
    #[cfg(feature = "log")]
    steps.push((Step::Logging, step_logging));
    #[cfg(feature = "kernel")]
    steps.push((Step::CrashCapture, step_crash_capture));
    #[cfg(feature = "kernel")]
    steps.push((Step::Kernel, step_kernel));
    steps
}

/// Boots a system from `settings`.
pub(crate) fn start(settings: Settings) -> Result<System, Error> {
    let name = settings.name.clone();
    validate(&settings).map_err(|error| error.for_app(&name))?;
    let mut parts = Parts::new(settings);
    match run_steps(&mut parts, &steps()) {
        Ok(()) => System::ready(parts).map_err(|error| error.for_app(&name)),
        Err(error) => {
            let kind = error.kind();
            if kind != ErrorKind::Interrupted {
                parts.log.error(
                    "boot failed",
                    &[
                        ("error", Val::Str(&error.to_string())),
                        ("do", Val::Str(error.action())),
                    ],
                );
            }
            let _code = parts.teardown(error.kind().exit_value(), "boot failed");
            Err(error.for_app(&name))
        }
    }
}

// ---------------------------------------------------------------------------
// The steps
// ---------------------------------------------------------------------------

/// Step 1: the build info is compiled in; it is recorded in the settings.
fn step_build_info(_parts: &mut Parts) -> Result<(), Error> {
    Ok(())
}

/// Step 2: the instance (id, start time, host, pid, user).
#[cfg(feature = "sys")]
fn step_instance(parts: &mut Parts) -> Result<(), Error> {
    parts.instance = Some(sys_lib::Instance::start());
    Ok(())
}

/// The prefix of the application's environment variables: the name in
/// upper case, every other character an underscore (`hive-db` gives
/// `HIVE_DB`). A leading digit gets an underscore in front.
#[cfg(feature = "paths")]
pub(crate) fn env_prefix(name: &str) -> String {
    let mut prefix: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    if prefix.starts_with(|c: char| c.is_ascii_digit()) {
        prefix.insert(0, '_');
    }
    prefix
}

/// Step 3: the application's directories, resolved. Nothing is created.
#[cfg(feature = "paths")]
fn step_paths(parts: &mut Parts) -> Result<(), Error> {
    use path_lib::{App, Scope};

    let name = parts.settings.name.clone();
    let prefix = env_prefix(&name);
    let scope = match (&parts.settings.portable, parts.settings.scope) {
        (Some(_), _) => None,
        (None, Some(scope)) => Some(scope),
        (None, None) => scope_from_env(&prefix)?,
    };
    let app = match (&parts.settings.portable, scope) {
        (Some(root), _) => App::portable(&name, root),
        (None, Some(Scope::Global)) => App::global(&name),
        (None, None) if auto_from_env(&prefix) => App::auto(&name),
        (None, _) => App::new(&name),
    };
    let app = app
        .and_then(|app| app.with_env(&prefix))
        .map_err(|e| paths_error(&e).caused_by(e))?;
    parts.app = Some(app);
    Ok(())
}

#[cfg(feature = "paths")]
fn scope_from_env(prefix: &str) -> Result<Option<path_lib::Scope>, Error> {
    let variable = format!("{prefix}_PATH_SCOPE");
    match std::env::var(&variable).ok().as_deref() {
        None | Some("") | Some("user") | Some("auto") => Ok(None),
        Some("global") => Ok(Some(path_lib::Scope::Global)),
        Some(_) => Err(Error::new(
            ErrorKind::Paths,
            format!("{variable} must be user, global, or auto"),
            format!("set {variable} to user, global, or auto, or unset it"),
        )),
    }
}

#[cfg(feature = "paths")]
fn auto_from_env(prefix: &str) -> bool {
    std::env::var(format!("{prefix}_PATH_SCOPE")).as_deref() == Ok("auto")
}

#[cfg(feature = "paths")]
fn paths_error(error: &path_lib::Error) -> Error {
    let kind = match error {
        path_lib::Error::InvalidName { .. } => ErrorKind::Setup,
        _ => ErrorKind::Paths,
    };
    Error::new(
        kind,
        format!("the application's directories cannot be resolved: {error}"),
        "fix the directory named in the message, or point <NAME>_<KIND>_DIR at an absolute, writable directory",
    )
    .member_of(Member::Paths)
}

/// Step 4: the `.env` file, read. The process environment is never changed.
#[cfg(feature = "env")]
fn step_environment(parts: &mut Parts) -> Result<(), Error> {
    let prefix = format!("{}_", env_prefix(&parts.settings.name));
    let env = if parts.settings.is_disabled(Member::Env) {
        Ok(env_lib::Env::new())
    } else if let Some(path) = &parts.settings.env_file {
        if !path.is_file() {
            return Err(Error::new(
                ErrorKind::Environment,
                format!("the environment file {} does not exist", path.display()),
                "create it, or remove the env_file() call",
            )
            .member_of(Member::Env));
        }
        env_lib::Env::open(path)
    } else {
        match &parts.app {
            Some(app) => env_lib::Env::for_app(app),
            None => Ok(env_lib::Env::new()),
        }
    };
    let env = env.and_then(|env| env.with_prefix(&prefix)).map_err(|e| {
        Error::new(
            ErrorKind::Environment,
            format!("the environment file cannot be read: {e}"),
            "fix the line named in the message",
        )
        .member_of(Member::Env)
        .caused_by(e)
    })?;
    parts.env = Some(env);
    Ok(())
}

/// Step 6: the single-instance lock, the umask, and the open-file limit.
#[cfg(feature = "sys")]
fn step_guard(parts: &mut Parts) -> Result<(), Error> {
    use sys_lib::process::Guard;

    let mut builder = Guard::builder(&parts.settings.name).lock(parts.settings.single_instance);
    #[cfg(feature = "paths")]
    if let Some(app) = &parts.app {
        if parts.settings.single_instance {
            // Owner-only on every OS before the guard takes the lock in it
            // (threat T3); the guard checks it again.
            let _dir = app
                .ensure_private(path_lib::Kind::Runtime)
                .map_err(|e| paths_error(&e).caused_by(e))?;
        }
        builder = builder.lock_dir(app.dir(path_lib::Kind::Runtime));
    }
    if !parts.settings.process_wide {
        // Leave the umask and the open-file limit alone: with a target and
        // minimum of zero the guard plans no change.
        builder = builder
            .keep_umask()
            .open_files_target(0)
            .open_files_minimum(0);
    }
    let guard = builder.build().map_err(|e| guard_error(&e).caused_by(e))?;
    parts.guard = Some(guard);
    Ok(())
}

#[cfg(feature = "sys")]
fn guard_error(error: &sys_lib::Error) -> Error {
    use sys_lib::Error as E;

    let (kind, action) = match error {
        E::AlreadyRunning { .. } => (
            ErrorKind::AlreadyRunning,
            "stop the other copy, or start this one with a different name",
        ),
        E::RunDirUnavailable { .. } => (
            ErrorKind::LockDirInsecure,
            "make the lock directory owner-only (chmod 700) and local, or point <NAME>_RUNTIME_DIR at one",
        ),
        E::OpenFilesTooLow { .. } => (
            ErrorKind::Limits,
            "raise the hard open-file limit: LimitNOFILE= in the systemd unit, ulimit -Hn, or launchd's limit",
        ),
        E::InvalidInput { .. } => (ErrorKind::Setup, "fix the application name"),
        _ => (ErrorKind::Io, "see the cause in the message"),
    };
    Error::new(kind, error.to_string(), action).member_of(Member::Sys)
}

/// Step 7: the kernel's signal intake.
#[cfg(feature = "signals")]
fn step_signals(parts: &mut Parts) -> Result<(), Error> {
    if !parts.settings.process_wide || parts.settings.is_disabled(Member::Signals) {
        return Ok(());
    }
    let intake =
        crate::kernel::signals::SignalIntake::install(parts.kernel.clone(), parts.log.clone())?;
    parts.signals = Some(intake);
    Ok(())
}

/// Whether logs go to a file: logging is compiled in and enabled, and the
/// application has directories.
#[cfg(feature = "paths")]
fn logs_to_file(parts: &Parts) -> bool {
    cfg!(feature = "log") && !parts.settings.is_disabled(Member::Log)
}

/// Step 9: the directories the enabled members write to, created
/// owner-only. Only the log directory in this version.
#[cfg(feature = "paths")]
fn step_directories(parts: &mut Parts) -> Result<(), Error> {
    if !logs_to_file(parts) {
        return Ok(());
    }
    if let Some(app) = &parts.app {
        // Owner-only on every OS: mode 0700 on Unix, a protected access
        // control list on Windows (threat T3).
        let _created = app
            .ensure_private(path_lib::Kind::Logs)
            .map_err(|e| paths_error(&e).caused_by(e))?;
    }
    Ok(())
}

/// Step 10: the logger. Human-readable lines on a terminal and JSON
/// otherwise on standard error, and JSON lines in `<logs>/<name>.log`
/// rotated daily (14 files kept).
#[cfg(feature = "log")]
fn step_logging(parts: &mut Parts) -> Result<(), Error> {
    use std::io::IsTerminal as _;

    use log_io::{Level, Logger, sink::NullSink};

    let name = parts.settings.name.clone();
    let logger = if parts.settings.is_disabled(Member::Log) {
        Logger::builder().with_sink(NullSink::new()).build()
    } else {
        let mut builder = Logger::builder()
            .level(Level::Info)
            .default_target(name.clone())
            .redact_common_secrets();
        builder = if std::io::stderr().is_terminal() {
            builder.stderr_human()
        } else {
            builder.stderr_json()
        };
        #[cfg(feature = "paths")]
        if let Some(app) = &parts.app {
            let path = app.dir(path_lib::Kind::Logs).join(format!("{name}.log"));
            let file = log_io::sink::RotatingFile::builder(&path)
                .rotation(log_io::sink::Rotation::Daily)
                .max_files(14)
                .open()
                .map_err(|e| {
                    Error::new(
                        ErrorKind::Logging,
                        format!("the log file {} cannot be opened: {e}", path.display()),
                        "check that the log directory is writable and that the path is not a directory",
                    )
                    .member_of(Member::Log)
                    .caused_by(e)
                })?;
            builder = builder.writer(file, log_io::format::JsonFormat::new());
        }
        builder.try_build().map_err(|e| {
            Error::new(
                ErrorKind::Logging,
                format!("the logger cannot be built: {e}"),
                "report this: the suite's own logger settings are invalid",
            )
            .member_of(Member::Log)
            .caused_by(e)
        })?
    };
    let _first = parts.log.set(logger.clone());
    let mut global_taken = false;
    if parts.settings.process_wide && !parts.settings.is_disabled(Member::Log) {
        global_taken = log_io::set_global(logger.clone()).is_err();
    }
    parts.logger = Some(logger);
    first_lines(parts);
    if global_taken {
        parts.log.warn(
            "a process-wide logger was already installed; this system's logger is reachable only through its handle",
            &[],
        );
    }
    Ok(())
}

/// The first log lines: build, instance, guard warnings, the steps so far.
#[cfg(feature = "log")]
fn first_lines(parts: &Parts) {
    #[cfg(feature = "sys")]
    {
        match parts.settings.build {
            Some(build) => parts.log.info("build", &[("build", Val::Str(&build.to_string()))]),
            None => parts.log.warn(
                "build info not recorded: start with start!() or Builder::build_info(build_info!())",
                &[],
            ),
        }
        if let Some(instance) = parts.instance {
            parts
                .log
                .info("instance", &[("instance", Val::Str(&instance.to_string()))]);
        }
        if let Some(guard) = &parts.guard {
            for warning in &guard.report().warnings {
                parts
                    .log
                    .warn("guard", &[("warning", Val::Str(&format!("{warning:?}")))]);
            }
        }
    }
    parts.log_timeline();
}

/// Step 11: the crash hook.
#[cfg(feature = "kernel")]
fn step_crash_capture(parts: &mut Parts) -> Result<(), Error> {
    use crate::kernel::crash::{CrashHook, CrashSettings};

    if !parts.settings.process_wide || !parts.settings.crash_capture {
        return Ok(());
    }
    #[cfg(feature = "paths")]
    let dir = parts
        .app
        .as_ref()
        .filter(|_| logs_to_file(parts))
        .map(|app| app.dir(path_lib::Kind::Logs).to_path_buf());
    #[cfg(not(feature = "paths"))]
    let dir = None;
    parts.crash = Some(CrashHook::install(CrashSettings {
        app: parts.settings.name.clone(),
        dir,
        max_reports: 8,
        log: parts.log.clone(),
        #[cfg(feature = "sys")]
        build: parts.settings.build,
        #[cfg(feature = "sys")]
        instance: parts.instance,
    }));
    Ok(())
}

/// Step 15: the kernel. Its runtime (workers, timers) starts here from 0.4.
#[cfg(feature = "kernel")]
fn step_kernel(_parts: &mut Parts) -> Result<(), Error> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Builder;

    fn parts() -> Parts {
        let settings = Builder::new("boot-test").process_wide(false).settings;
        Parts::new(settings)
    }

    fn ok(_parts: &mut Parts) -> Result<(), Error> {
        Ok(())
    }

    fn fail(_parts: &mut Parts) -> Result<(), Error> {
        Err(Error::new(ErrorKind::Io, "injected", "nothing"))
    }

    #[test]
    fn test_run_steps_runs_every_step_in_order() {
        let mut parts = parts();
        let steps: [(Step, StepFn); 3] = [
            (Step::BuildInfo, ok),
            (Step::Paths, ok),
            (Step::Logging, ok),
        ];
        assert!(run_steps(&mut parts, &steps).is_ok());
        let ran: Vec<Step> = parts.timeline.iter().map(|(step, _)| *step).collect();
        assert_eq!(ran, vec![Step::BuildInfo, Step::Paths, Step::Logging]);
    }

    #[test]
    fn test_run_steps_stops_at_the_failing_step_and_names_it() {
        // Threat T4: a failure at any step stops the boot there.
        let order = [
            Step::BuildInfo,
            Step::Instance,
            Step::Paths,
            Step::Guard,
            Step::Logging,
        ];
        for failing in 0..order.len() {
            let mut parts = parts();
            let steps: Vec<(Step, StepFn)> = order
                .iter()
                .enumerate()
                .map(|(i, step)| {
                    (
                        *step,
                        if i == failing {
                            fail as StepFn
                        } else {
                            ok as StepFn
                        },
                    )
                })
                .collect();
            let error = run_steps(&mut parts, &steps).err();
            assert_eq!(error.as_ref().and_then(Error::step), Some(order[failing]));
            assert_eq!(
                parts.timeline.len(),
                failing,
                "steps after the failure must not run"
            );
            assert_eq!(parts.teardown(0, "test"), 0);
        }
    }

    #[cfg(feature = "kernel")]
    #[test]
    fn test_run_steps_honours_a_stop_at_every_boundary() {
        // Threat T8: a stop requested during a step is acted on at the next
        // step boundary, and the boot unwinds with Interrupted (exit 0).
        fn stop(parts: &mut Parts) -> Result<(), Error> {
            parts.kernel.request_stop("test");
            Ok(())
        }
        let order = [
            Step::BuildInfo,
            Step::Instance,
            Step::Paths,
            Step::Signals,
            Step::Kernel,
        ];
        for stopping in 0..order.len() {
            let mut parts = parts();
            let steps: Vec<(Step, StepFn)> = order
                .iter()
                .enumerate()
                .map(|(i, step)| {
                    (
                        *step,
                        if i == stopping {
                            stop as StepFn
                        } else {
                            ok as StepFn
                        },
                    )
                })
                .collect();
            let error = run_steps(&mut parts, &steps).err();
            assert_eq!(
                error.as_ref().map(Error::kind),
                Some(ErrorKind::Interrupted)
            );
            assert_eq!(parts.timeline.len(), stopping + 1);
            assert_eq!(
                error.map(|e| e.exit_code()),
                Some(std::process::ExitCode::SUCCESS)
            );
            assert_eq!(parts.teardown(0, "test"), 0);
            assert_eq!(parts.kernel.phase(), Phase::Stopped);
        }
    }

    #[cfg(feature = "paths")]
    #[test]
    fn test_env_prefix_upper_cases_and_replaces() {
        assert_eq!(env_prefix("site"), "SITE");
        assert_eq!(env_prefix("hive-db.v2"), "HIVE_DB_V2");
        assert_eq!(env_prefix("9lives"), "_9LIVES");
    }

    #[test]
    fn test_steps_follow_boot_order() {
        let steps = steps();
        assert!(steps.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(steps.first().map(|s| s.0), Some(Step::BuildInfo));
    }
}
