//! The boot order and its unwinding (`docs/BOOT-ORDER.md`).
//!
//! Each step is a function over the parts started so far. The driver runs
//! them in order, stops at the first failure or at a stop request, and the
//! same teardown that ends a running system then stops whatever started.

#[cfg(feature = "kernel")]
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(any(
    feature = "kernel",
    feature = "env",
    feature = "log",
    feature = "sys",
    feature = "paths",
    feature = "signals",
    feature = "metrics",
    feature = "config"
))]
use crate::Member;
#[cfg(feature = "kernel")]
use crate::kernel::{Kernel, Level};
#[cfg(feature = "kernel")]
use crate::lifecycle::Phase;
#[cfg(any(feature = "log", feature = "signals", feature = "kernel"))]
use crate::shutdown::{Outcome, run_with_deadline};
use crate::{
    Error, ErrorKind, Foundation, Step,
    builder::{DEFAULT_MEMBER_RESERVE, DEFAULT_SHUTDOWN_DEADLINE, Settings, validate},
    logging::{LogSlot, Val},
    sections::Sections,
};

/// The ring of recent log lines the crash report ends with.
#[cfg(feature = "log")]
pub(crate) type RecentLog = log_io::sink::RecentLines<log_io::format::JsonFormat>;

/// A boot step.
pub(crate) type StepFn = fn(&mut Parts) -> Result<(), Error>;

/// Everything started so far. The teardown stops it in reverse.
pub(crate) struct Parts {
    pub(crate) settings: Settings,
    pub(crate) sections: Sections,
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
    #[cfg(feature = "config")]
    pub(crate) config: Option<crate::Snapshot<crate::config::Config>>,
    #[cfg(feature = "config")]
    pub(crate) config_source: Option<crate::config::load::Source>,
    #[cfg(all(feature = "kernel", feature = "config"))]
    pub(crate) reloader: Option<Arc<crate::config::reload::Reloader>>,
    #[cfg(feature = "log")]
    pub(crate) logger: Option<log_io::Logger>,
    #[cfg(feature = "log")]
    pub(crate) recent: Option<std::sync::Arc<RecentLog>>,
    #[cfg(feature = "metrics")]
    pub(crate) metrics: Option<crate::metrics_handle::MetricsRef>,
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
            sections: Sections::default(),
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
            #[cfg(feature = "config")]
            config: None,
            #[cfg(feature = "config")]
            config_source: None,
            #[cfg(all(feature = "kernel", feature = "config"))]
            reloader: None,
            #[cfg(feature = "log")]
            logger: None,
            #[cfg(feature = "log")]
            recent: None,
            #[cfg(feature = "metrics")]
            metrics: None,
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

    /// The suite's sections as currently in force (a reload may have
    /// changed the live ones).
    fn current_sections(&self) -> Sections {
        #[cfg(all(feature = "kernel", feature = "config"))]
        if let Some(reloader) = &self.reloader {
            return reloader.sections();
        }
        self.sections.clone()
    }

    /// The shutdown deadline and member reserve: the builder's, else the
    /// configuration's, else the defaults.
    fn deadlines(&self) -> (Duration, Duration) {
        let sections = self.current_sections();
        let deadline = self
            .settings
            .shutdown_deadline
            .or(sections.kernel.shutdown_deadline)
            .unwrap_or(DEFAULT_SHUTDOWN_DEADLINE);
        let reserve = self
            .settings
            .member_reserve
            .or(sections.kernel.member_reserve)
            .unwrap_or(DEFAULT_MEMBER_RESERVE)
            .min(deadline / 2);
        (deadline, reserve)
    }

    /// Stops everything that started, in the shutdown order, within the
    /// shutdown deadline. Returns the exit code: `code`, or 79 if a step was
    /// abandoned at the deadline.
    pub(crate) fn teardown(&mut self, code: u8, cause: &str) -> u8 {
        let (budget, reserve) = self.deadlines();
        #[cfg(not(feature = "kernel"))]
        let _no_subsystems = reserve;
        let began = Instant::now();
        let deadline = began.checked_add(budget).unwrap_or(began);
        let mut code = code;

        // 1. Draining: every worker and subsystem learns of the stop at once.
        #[cfg(feature = "kernel")]
        {
            self.kernel.set_phase(Phase::Draining);
            let _raised = self
                .kernel
                .urgent()
                .raise(Level::Draining, "stop requested");
        }
        if self.notified_ready {
            self.notify("STOPPING=1");
            self.notify("STATUS=stopping");
        }
        self.log.info("stopping", &[("cause", Val::Str(cause))]);

        // 2. Subsystems, in reverse of their start order.
        #[cfg(feature = "kernel")]
        {
            let subsystems_deadline = began
                .checked_add(budget.saturating_sub(reserve))
                .unwrap_or(began);
            code = self.stop_subsystems(subsystems_deadline, code);
        }

        // 3. Stopping: workers and timers.
        #[cfg(feature = "kernel")]
        {
            let _raised = self.kernel.urgent().raise(Level::Stopping, "stopping");
            self.kernel.set_phase(Phase::Stopping);
            let workers_deadline = began
                .checked_add(budget.saturating_sub(reserve))
                .unwrap_or(began)
                .max(Instant::now());
            for late in self.kernel.runtime().stop(workers_deadline) {
                self.log.error(
                    "worker did not stop by the deadline and was abandoned",
                    &[("worker", Val::Str(&late))],
                );
                code = crate::exit::DEADLINE;
            }
        }

        // 8. Signals: a further signal gets the operating system's default.
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

        // 9. Logging, last among the members.
        code = self.close_logging(deadline, code);

        // 10. Stopped.
        #[cfg(feature = "kernel")]
        {
            self.kernel.set_phase(Phase::Stopped);
            if let Some(crash) = self.crash.take() {
                crash.uninstall();
            }
        }
        #[cfg(feature = "sys")]
        drop(self.guard.take());
        code
    }

    /// Shutdown step 2: every started subsystem, last started first, each
    /// on a stop thread so the deadline holds (threat T6).
    #[cfg(feature = "kernel")]
    fn stop_subsystems(&self, deadline: Instant, mut code: u8) -> u8 {
        while let Some(slot) = self.kernel.pop_slot() {
            let name = slot.name.clone();
            let stop_slot = Arc::clone(&slot);
            let stop_deadline = crate::lifecycle::Deadline::at(deadline);
            let result = Arc::new(std::sync::Mutex::new(None));
            let result_slot = Arc::clone(&result);
            let outcome = run_with_deadline(&format!("subsystem-{name}"), deadline, move || {
                let stopped = stop_slot
                    .subsystem
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .stop(stop_deadline);
                *result_slot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(stopped);
            });
            match outcome {
                Outcome::Done(took) => {
                    let failure = result
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                        .and_then(Result::err);
                    match failure {
                        None => self.log.info(
                            "subsystem stopped",
                            &[
                                ("subsystem", Val::Str(&name)),
                                ("millis", Val::U64(millis(took))),
                            ],
                        ),
                        Some(failure) => self.log.error(
                            "subsystem stop failed",
                            &[
                                ("subsystem", Val::Str(&name)),
                                ("error", Val::Str(failure.message())),
                            ],
                        ),
                    }
                }
                Outcome::Panicked => {
                    self.log.error(
                        "subsystem panicked while stopping",
                        &[("subsystem", Val::Str(&name))],
                    );
                }
                Outcome::Abandoned => {
                    self.log.error(
                        "subsystem did not stop by the deadline and was abandoned",
                        &[("subsystem", Val::Str(&name))],
                    );
                    code = crate::exit::DEADLINE;
                }
            }
        }
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
        self.log.info(
            "stopped",
            &[
                ("exit_code", Val::U64(u64::from(code))),
                ("uptime_ms", Val::U64(millis(self.began.elapsed()))),
            ],
        );
        let outcome = run_with_deadline("logging", deadline.max(Instant::now()), move || {
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
    /// state and `[kernel] notify` allows it; a failure is logged, never
    /// fatal.
    pub(crate) fn notify(&self, state: &str) {
        if !self.settings.process_wide || !self.sections.kernel.notify {
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

    /// Starts the systemd keep-alive when the service manager asked for one
    /// (`WATCHDOG_USEC`, for this process): `WATCHDOG=1` at half the
    /// interval while the system is not `Unhealthy`, so a wedged process
    /// is restarted, and `STATUS=` when the status changes (D-030).
    #[cfg(feature = "kernel")]
    pub(crate) fn start_watchdog(&self) {
        use crate::kernel::Status;

        if !self.settings.process_wide || !self.sections.kernel.notify {
            return;
        }
        let Some(period) = watchdog_period() else {
            return;
        };
        let kernel = self.kernel.clone();
        let mut last = Status::Healthy;
        let started = self
            .kernel
            .runtime()
            .every("systemd-watchdog", period, move || {
                let status = kernel.status();
                // A failed keep-alive is not logged: at this rate it would
                // flood the log, and systemd reports the missed keep-alive.
                if status != last {
                    last = status;
                    let _sent = crate::kernel::notify::notify(&format!("STATUS={}", status.name()));
                }
                if status != Status::Unhealthy {
                    let _sent = crate::kernel::notify::notify("WATCHDOG=1");
                }
            });
        if let Err(error) = started {
            self.log.warn(
                "the systemd keep-alive could not be started",
                &[("error", Val::Str(&error.to_string()))],
            );
        }
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

    /// Fails the boot when a stop was requested (threat T8).
    #[cfg(feature = "kernel")]
    pub(crate) fn check_interrupted(&self, step: Step) -> Result<(), Error> {
        match self.kernel.stop_reason() {
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
    pub(crate) fn check_interrupted(&self, _step: Step) -> Result<(), Error> {
        Ok(())
    }

    /// Logs a failed boot and stops what started.
    pub(crate) fn fail(&mut self, error: Error) -> Error {
        if error.kind() != ErrorKind::Interrupted {
            self.log.error(
                "boot failed",
                &[
                    ("error", Val::Str(&error.to_string())),
                    ("do", Val::Str(error.action())),
                ],
            );
        }
        let _code = self.teardown(error.kind().exit_value(), "boot failed");
        let name = self.settings.name.clone();
        error.for_app(&name)
    }
}

/// Half of `WATCHDOG_USEC` when it is set for this process (`WATCHDOG_PID`
/// unset or ours).
#[cfg(feature = "kernel")]
fn watchdog_period() -> Option<Duration> {
    let usec: u64 = std::env::var("WATCHDOG_USEC").ok()?.parse().ok()?;
    if let Ok(pid) = std::env::var("WATCHDOG_PID")
        && pid.parse::<u32>().ok() != Some(std::process::id())
    {
        return None;
    }
    Some(Duration::from_micros(usec / 2)).filter(|period| *period >= Duration::from_millis(1))
}

pub(crate) fn millis(duration: Duration) -> u64 {
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
        parts.check_interrupted(step)?;
        let began = Instant::now();
        run(parts).map_err(|error| error.at(step))?;
        parts.timeline.push((step, began.elapsed()));
    }
    parts.check_interrupted(Step::Kernel)
}

/// The boot steps 1 to 15 compiled into this build, in order.
pub(crate) fn steps() -> Vec<(Step, StepFn)> {
    let mut steps: Vec<(Step, StepFn)> = Vec::with_capacity(17);
    steps.push((Step::BuildInfo, step_build_info));
    #[cfg(feature = "sys")]
    steps.push((Step::Instance, step_instance));
    #[cfg(feature = "paths")]
    steps.push((Step::Paths, step_paths));
    #[cfg(feature = "env")]
    steps.push((Step::Environment, step_environment));
    #[cfg(feature = "config")]
    steps.push((Step::Configuration, step_configuration));
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
    #[cfg(feature = "metrics")]
    steps.push((Step::Metrics, step_metrics));
    #[cfg(feature = "kernel")]
    steps.push((Step::Kernel, step_kernel));
    steps
}

/// Runs steps 1 to 15 and builds the foundation. On failure everything that
/// started is stopped.
pub(crate) fn foundation(settings: Settings) -> Result<(Parts, Foundation), Error> {
    let name = settings.name.clone();
    validate(&settings).map_err(|error| error.for_app(&name))?;
    let mut parts = Parts::new(settings);
    if let Err(error) = run_steps(&mut parts, &steps()) {
        return Err(parts.fail(error));
    }
    match crate::system::foundation_from(&mut parts) {
        Ok(foundation) => Ok((parts, foundation)),
        Err(error) => Err(parts.fail(error)),
    }
}

/// Boots a system: every step, then ready.
pub(crate) fn start(settings: Settings) -> Result<crate::System, Error> {
    #[cfg(feature = "kernel")]
    {
        boot(settings)?.ready()
    }
    #[cfg(not(feature = "kernel"))]
    {
        let (parts, foundation) = foundation(settings)?;
        Ok(crate::System::ready(parts, foundation))
    }
}

/// Runs steps 1 to 15 and returns a `Boot`.
#[cfg(feature = "kernel")]
pub(crate) fn boot(settings: Settings) -> Result<crate::Boot, Error> {
    let (mut parts, foundation) = foundation(settings)?;
    let registered = std::mem::take(
        parts
            .settings
            .subsystems
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    let on_ready = std::mem::take(
        parts
            .settings
            .on_ready
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    Ok(crate::Boot::new(parts, foundation, registered, on_ready))
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

/// Step 4: the `.env` file, checked and read. The process environment is
/// never changed. A `.env` file another user can change is refused before
/// it is read: it can redirect the whole configuration through
/// `<NAME>_CONFIG` (the 0.2 design-lock defence idea).
#[cfg(feature = "env")]
fn step_environment(parts: &mut Parts) -> Result<(), Error> {
    let prefix = format!("{}_", env_prefix(&parts.settings.name));
    let path = if parts.settings.is_disabled(Member::Env) {
        None
    } else if let Some(path) = &parts.settings.env_file {
        if !path.is_file() {
            return Err(Error::new(
                ErrorKind::Environment,
                format!("the environment file {} does not exist", path.display()),
                "create it, or remove the env_file() call",
            )
            .member_of(Member::Env));
        }
        Some(path.clone())
    } else {
        match &parts.app {
            Some(app) => Some(
                app.config_at(".env")
                    .map_err(|e| paths_error(&e).caused_by(e))?,
            ),
            None => None,
        }
    };
    let env = match &path {
        Some(path) => {
            if path.is_file() {
                crate::secure_file::check(path, "environment file")?;
            }
            env_lib::Env::open(path)
        }
        None => Ok(env_lib::Env::new()),
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

/// Step 5: the configuration: found, permission-checked, parsed, its
/// suite sections validated. Secret references are refused until 0.5.
#[cfg(feature = "config")]
fn step_configuration(parts: &mut Parts) -> Result<(), Error> {
    use crate::config::load::{self, Search, Source};

    let prefix = env_prefix(&parts.settings.name);
    let disabled = parts.settings.is_disabled(Member::Config);
    #[cfg(feature = "env")]
    let env_file_value = if std::env::var_os(format!("{prefix}_CONFIG")).is_none() {
        parts
            .env
            .as_ref()
            .and_then(|env| env.get("CONFIG").map(|value| value.into_owned()))
    } else {
        None
    };
    #[cfg(not(feature = "env"))]
    let env_file_value = None;
    #[cfg(feature = "sys")]
    let elevated = sys_lib::process::is_elevated().ok();
    #[cfg(not(feature = "sys"))]
    let elevated = None;
    let config_dir = parts
        .app
        .as_ref()
        .map(|app| app.dir(path_lib::Kind::Config).to_path_buf());
    let (source, origin) = if disabled {
        (Source::Defaults, load::Origin::NotFound)
    } else {
        load::find(&Search {
            name: &parts.settings.name,
            prefix: &prefix,
            explicit_file: parts.settings.config_file.as_deref(),
            explicit_text: parts.settings.config_str.as_deref(),
            env_file_value,
            config_dir: config_dir.as_deref(),
            elevated,
        })?
    };
    let config = load::load(&source, 1)?;
    let sections = crate::config::sections::read(&config)?;
    let (deadline, reserve) = (
        parts
            .settings
            .shutdown_deadline
            .or(sections.kernel.shutdown_deadline)
            .unwrap_or(DEFAULT_SHUTDOWN_DEADLINE),
        parts
            .settings
            .member_reserve
            .or(sections.kernel.member_reserve)
            .unwrap_or(DEFAULT_MEMBER_RESERVE),
    );
    if deadline.is_zero() || deadline > crate::builder::MAX_SHUTDOWN_DEADLINE || reserve >= deadline
    {
        return Err(Error::new(
            ErrorKind::ConfigInvalid,
            "`kernel.shutdown_deadline` must be more than zero and at most one hour, and `kernel.member_reserve` shorter than it",
            "fix the [kernel] section",
        ));
    }
    parts.sections = sections;
    parts.config = Some(crate::Snapshot::new(config));
    parts.config_source = Some(source);
    parts
        .log
        .info("configuration", &[("source", Val::Str(origin.describe()))]);
    Ok(())
}

/// Step 6: the single-instance lock, the umask, and the open-file limit.
#[cfg(feature = "sys")]
fn step_guard(parts: &mut Parts) -> Result<(), Error> {
    use sys_lib::process::Guard;

    let section = parts.sections.guard.clone();
    let lock = parts
        .settings
        .single_instance
        .or(section.lock)
        .unwrap_or(true);
    let mut builder = Guard::builder(&parts.settings.name)
        .lock(lock)
        .allow_network_lock(section.allow_network_lock);
    match &section.lock_dir {
        Some(dir) => builder = builder.lock_dir(dir),
        None => {
            #[cfg(feature = "paths")]
            if let Some(app) = &parts.app {
                if lock {
                    // Owner-only on every OS before the guard takes the lock
                    // in it (threat T3); the guard checks it again.
                    let _dir = app
                        .ensure_private(path_lib::Kind::Runtime)
                        .map_err(|e| paths_error(&e).caused_by(e))?;
                }
                builder = builder.lock_dir(app.dir(path_lib::Kind::Runtime));
            }
        }
    }
    if parts.settings.process_wide {
        if let Some(mask) = section.umask {
            builder = builder.umask(mask);
        }
        if let Some(target) = section.open_files_target {
            builder = builder.open_files_target(target);
        }
        if let Some(minimum) = section.open_files_minimum {
            builder = builder.open_files_minimum(minimum);
        }
    } else {
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
    let intake = crate::kernel::signals::SignalIntake::install(
        parts.kernel.clone(),
        parts.log.clone(),
        parts.sections.kernel.reload_signal,
        parts.sections.kernel.dump_signal,
    )?;
    parts.signals = Some(intake);
    Ok(())
}

/// Whether logs go to a file: logging is compiled in and enabled, and
/// `[log] file` allows it.
#[cfg(feature = "paths")]
fn logs_to_file(parts: &Parts) -> bool {
    cfg!(feature = "log") && !parts.settings.is_disabled(Member::Log) && parts.sections.log.file
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

/// Step 10: the logger, from `[log]` (D-041 for the defaults): standard
/// error (human on a terminal, JSON otherwise), JSON lines in
/// `<logs>/<name>.log` (owner only, rotated, 14 files kept), and the last
/// lines in memory for crash reports.
#[cfg(feature = "log")]
fn step_logging(parts: &mut Parts) -> Result<(), Error> {
    use std::io::IsTerminal as _;

    use log_io::{Logger, sink::NullSink};

    #[cfg(feature = "paths")]
    use crate::sections::LogRotation;
    use crate::sections::{LogFormat, LogLevel};

    let name = parts.settings.name.clone();
    let section = parts.sections.log.clone();
    let logger = if parts.settings.is_disabled(Member::Log) {
        Logger::builder().with_sink(NullSink::new()).build()
    } else {
        let level = match section.level {
            LogLevel::Trace => log_io::Level::Trace,
            LogLevel::Debug => log_io::Level::Debug,
            LogLevel::Info => log_io::Level::Info,
            LogLevel::Warn => log_io::Level::Warn,
            LogLevel::Error => log_io::Level::Error,
        };
        let mut builder = Logger::builder().level(level).default_target(name.clone());
        if let Some(filter) = &section.filter {
            builder = builder.try_filter_directive(filter).map_err(|e| {
                Error::new(
                    ErrorKind::ConfigInvalid,
                    format!("`log.filter` is not a valid filter: {e}"),
                    "fix `log.filter`, for example \"info,db=debug\"",
                )
            })?;
        }
        if section.redact_common {
            builder = builder.redact_common_secrets();
        }
        for key in &section.redact {
            builder = builder.redact_key(key.clone());
        }
        if section.recent_lines > 0 {
            let recent = std::sync::Arc::new(RecentLog::new(
                section.recent_lines,
                log_io::format::JsonFormat::new(),
            ));
            parts.recent = Some(std::sync::Arc::clone(&recent));
            builder = builder.with_sink(recent);
        }
        if section.stderr {
            builder = match section.format {
                LogFormat::Human => builder.stderr_human(),
                LogFormat::Json => builder.stderr_json(),
                LogFormat::Logfmt => builder.stderr_logfmt(),
                LogFormat::Auto if std::io::stderr().is_terminal() => builder.stderr_human(),
                LogFormat::Auto => builder.stderr_json(),
            };
        }
        #[cfg(feature = "paths")]
        if logs_to_file(parts)
            && let Some(app) = &parts.app
        {
            let path = app.dir(path_lib::Kind::Logs).join(format!("{name}.log"));
            let rotation = match section.rotation {
                LogRotation::Never => log_io::sink::Rotation::Never,
                LogRotation::Hourly => log_io::sink::Rotation::Hourly,
                LogRotation::Daily => log_io::sink::Rotation::Daily,
            };
            let mut file = log_io::sink::RotatingFile::builder(&path)
                .rotation(rotation)
                .max_files(section.max_files)
                .mode(0o600);
            if let Some(bytes) = section.max_bytes {
                file = file.max_bytes(bytes);
            }
            let file = file.open().map_err(|e| {
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
                "turn on at least one sink: [log] stderr or file",
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
            Some(build) => parts
                .log
                .info("build", &[("build", Val::Str(&build.to_string()))]),
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

    let enabled = parts
        .settings
        .crash_capture
        .or(parts.sections.crash.enabled)
        .unwrap_or(true);
    if !parts.settings.process_wide || !enabled {
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
        max_reports: parts.sections.crash.max_reports,
        log: parts.log.clone(),
        #[cfg(feature = "log")]
        recent: parts.recent.clone(),
        #[cfg(feature = "sys")]
        build: parts.settings.build,
        #[cfg(feature = "sys")]
        instance: parts.instance,
    }));
    Ok(())
}

/// Step 12: the metrics registry: metrics-lib's global one when the system
/// may touch process-wide state (its macros record into it), else the
/// system's own.
#[cfg(feature = "metrics")]
fn step_metrics(parts: &mut Parts) -> Result<(), Error> {
    use crate::metrics_handle::MetricsRef;

    let section = parts.sections.metrics;
    if parts.settings.is_disabled(Member::Metrics) || !section.enabled {
        parts.metrics = Some(MetricsRef::Owned(std::sync::Arc::new(
            metrics_lib::MetricsCore::new(),
        )));
        return Ok(());
    }
    let config = metrics_lib::MetricsConfig::default().with_max_series(section.max_series);
    let metrics_error = |e: metrics_lib::MetricsError| {
        Error::new(
            ErrorKind::Metrics,
            format!("the metrics registry refused its settings: {e}"),
            "fix the [metrics] section",
        )
        .member_of(Member::Metrics)
        .caused_by(e)
    };
    let core = if parts.settings.process_wide {
        match metrics_lib::init_with_config(config) {
            Ok(core) => MetricsRef::Global(core),
            Err(metrics_lib::MetricsError::AlreadyInitialized) => {
                match metrics_lib::try_metrics() {
                    Some(core) => {
                        parts.log.warn(
                        "metrics-lib's global registry was already set up; its settings were kept",
                        &[],
                    );
                        MetricsRef::Global(core)
                    }
                    None => {
                        return Err(metrics_error(metrics_lib::MetricsError::AlreadyInitialized));
                    }
                }
            }
            Err(e) => return Err(metrics_error(e)),
        }
    } else {
        MetricsRef::Owned(std::sync::Arc::new(
            metrics_lib::MetricsCore::with_config(config).map_err(metrics_error)?,
        ))
    };
    parts.metrics = Some(core);
    Ok(())
}

/// Step 15: the kernel: connected to the log and metrics, the reloader set
/// up. Its runtime starts its threads on first use.
#[cfg(feature = "kernel")]
fn step_kernel(parts: &mut Parts) -> Result<(), Error> {
    let members: Vec<Member> = enabled_members(parts);
    #[cfg(feature = "metrics")]
    let metrics = match &parts.metrics {
        Some(core) => crate::kernel::metrics::KernelMetrics::new(core.clone()),
        None => crate::kernel::metrics::KernelMetrics::none(),
    };
    #[cfg(not(feature = "metrics"))]
    let metrics = crate::kernel::metrics::KernelMetrics::none();
    parts.kernel.wire(members, parts.log.clone(), metrics);
    #[cfg(feature = "config")]
    if let (Some(snapshot), Some(source)) = (&parts.config, &parts.config_source) {
        parts.reloader = Some(Arc::new(crate::config::reload::Reloader::new(
            source.clone(),
            snapshot.clone(),
            parts.sections.clone(),
            parts.kernel.clone(),
            parts.log.clone(),
        )));
    }
    Ok(())
}

/// The members compiled in and not disabled, for health reports.
#[cfg(feature = "kernel")]
fn enabled_members(parts: &Parts) -> Vec<Member> {
    [
        Member::Sys,
        Member::Paths,
        Member::Env,
        Member::Config,
        Member::Log,
        Member::Metrics,
        Member::Signals,
        Member::Time,
        Member::Ids,
    ]
    .into_iter()
    .filter(|m| m.is_compiled() && !parts.settings.disabled.contains(m))
    .collect()
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
            Step::Configuration,
            Step::Guard,
            Step::Logging,
            Step::Metrics,
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

    #[test]
    fn test_deadlines_default_and_reserve_is_capped() {
        let parts = parts();
        let (deadline, reserve) = parts.deadlines();
        assert_eq!(deadline, DEFAULT_SHUTDOWN_DEADLINE);
        assert_eq!(reserve, DEFAULT_MEMBER_RESERVE);
    }
}
