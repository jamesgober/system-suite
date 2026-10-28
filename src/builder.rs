//! `Builder`: the boot's settings.

#[cfg(any(feature = "paths", feature = "env", feature = "config"))]
use std::path::PathBuf;
#[cfg(feature = "time")]
use std::sync::Arc;
use std::time::Duration;

use crate::{Error, ErrorKind, Member, System};

/// The default shutdown deadline: below Kubernetes' default 30 s grace
/// period and systemd's 90 s, so the suite reports what was late before it
/// is killed (D-024).
pub(crate) const DEFAULT_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(25);

/// The default part of the shutdown deadline kept back for the members
/// after the subsystems (D-024).
pub(crate) const DEFAULT_MEMBER_RESERVE: Duration = Duration::from_secs(2);

/// The longest shutdown deadline accepted.
pub(crate) const MAX_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3600);

/// A callback run at boot step 17, before readiness is announced.
#[cfg(feature = "kernel")]
pub(crate) type OnReady =
    Box<dyn FnOnce(&crate::Foundation) -> Result<(), crate::lifecycle::Failure> + Send + 'static>;

/// Settings for the boot.
///
/// Every setting has a secure default; a setting given here wins over the
/// same setting in the configuration file. Invalid settings are reported by
/// [`start`](Builder::start) as an error of kind
/// [`Setup`](ErrorKind::Setup), never by the setter, so a chain of setters
/// never needs `?`. [`start!`](crate::start) covers the common case;
/// `Builder` is for services that configure their boot.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use system_suite::{Member, System};
///
/// # #[cfg(feature = "sys")] {
/// let system = System::builder("builder-doc")
///     .process_wide(false)          // touch no process-wide state (tests, embedding)
///     .single_instance(false)       // several copies may run
///     .disable(Member::Log)         // no log sinks
///     .shutdown_deadline(Duration::from_secs(5))
///     .start()?;
/// assert_eq!(system.name(), "builder-doc");
/// let code = system.shutdown();
/// # let _ = code;
/// # }
/// # Ok::<(), system_suite::Error>(())
/// ```
#[must_use = "a Builder does nothing until start()"]
pub struct Builder {
    pub(crate) settings: Settings,
}

/// The settings a `Builder` collects. `Option` means "not set here": the
/// configuration file, then the default, decides.
pub(crate) struct Settings {
    pub(crate) name: String,
    #[cfg(feature = "sys")]
    pub(crate) build: Option<&'static sys_lib::BuildInfo>,
    #[cfg(feature = "sys")]
    pub(crate) single_instance: Option<bool>,
    #[cfg(feature = "paths")]
    pub(crate) scope: Option<path_lib::Scope>,
    #[cfg(feature = "paths")]
    pub(crate) portable: Option<PathBuf>,
    #[cfg(feature = "env")]
    pub(crate) env_file: Option<PathBuf>,
    #[cfg(feature = "config")]
    pub(crate) config_file: Option<PathBuf>,
    #[cfg(feature = "config")]
    pub(crate) config_str: Option<String>,
    #[cfg(feature = "time")]
    pub(crate) clock: Option<Arc<dyn clock_lib::Clock>>,
    #[cfg(feature = "kernel")]
    pub(crate) crash_capture: Option<bool>,
    #[cfg(feature = "kernel")]
    // In a Mutex only so that `System`, which keeps the settings, stays
    // `Sync`; both are emptied before the system runs.
    pub(crate) subsystems: std::sync::Mutex<Vec<Box<dyn crate::lifecycle::Subsystem>>>,
    #[cfg(feature = "kernel")]
    pub(crate) on_ready: std::sync::Mutex<Vec<OnReady>>,
    pub(crate) disabled: Vec<Member>,
    pub(crate) process_wide: bool,
    pub(crate) shutdown_deadline: Option<Duration>,
    pub(crate) member_reserve: Option<Duration>,
}

impl Settings {
    #[cfg(any(
        feature = "env",
        feature = "log",
        feature = "paths",
        feature = "signals",
        feature = "metrics",
        feature = "config"
    ))]
    pub(crate) fn is_disabled(&self, member: Member) -> bool {
        self.disabled.contains(&member)
    }
}

impl Builder {
    /// A builder for the application `name`: 1 to 64 ASCII letters,
    /// digits, `-`, `_`, `.`, not starting with `.`. The name names the
    /// directories, the lock, the configuration file (`<name>.conf`), the
    /// environment prefix (`SITE_` for `site`), and the log target.
    pub fn new(name: &str) -> Builder {
        Builder {
            settings: Settings {
                name: name.to_owned(),
                #[cfg(feature = "sys")]
                build: None,
                #[cfg(feature = "sys")]
                single_instance: None,
                #[cfg(feature = "paths")]
                scope: None,
                #[cfg(feature = "paths")]
                portable: None,
                #[cfg(feature = "env")]
                env_file: None,
                #[cfg(feature = "config")]
                config_file: None,
                #[cfg(feature = "config")]
                config_str: None,
                #[cfg(feature = "time")]
                clock: None,
                #[cfg(feature = "kernel")]
                crash_capture: None,
                #[cfg(feature = "kernel")]
                subsystems: std::sync::Mutex::new(Vec::new()),
                #[cfg(feature = "kernel")]
                on_ready: std::sync::Mutex::new(Vec::new()),
                disabled: Vec::new(),
                process_wide: true,
                shutdown_deadline: None,
                member_reserve: None,
            },
        }
    }

    /// The application's build info, from
    /// [`build_info!`](crate::build_info) expanded in the application crate.
    /// [`start!`](crate::start) sets it; without it,
    /// [`Foundation::build`](crate::Foundation::build) is `None` and the
    /// first log lines say so.
    #[cfg(feature = "sys")]
    pub fn build_info(mut self, build: &'static sys_lib::BuildInfo) -> Self {
        self.settings.build = Some(build);
        self
    }

    /// Where the application's directories live: `User` (the default),
    /// `Global` for system services. Without this call, the
    /// `<NAME>_PATH_SCOPE` variable (`user`, `global`, `auto`) decides.
    /// `Scope::Portable` needs [`portable`](Self::portable) instead.
    #[cfg(feature = "paths")]
    pub fn scope(mut self, scope: path_lib::Scope) -> Self {
        self.settings.scope = Some(scope);
        self
    }

    /// Every directory under `root` (`root/data`, `root/config`, ...).
    #[cfg(feature = "paths")]
    pub fn portable(mut self, root: impl Into<PathBuf>) -> Self {
        self.settings.portable = Some(root.into());
        self
    }

    /// The `.env` file to read instead of `<config dir>/.env`. An explicit
    /// file that does not exist is an error of kind
    /// [`Environment`](ErrorKind::Environment).
    #[cfg(feature = "env")]
    pub fn env_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.settings.env_file = Some(path.into());
        self
    }

    /// The configuration file to read instead of searching for one. A file
    /// that does not exist is an error of kind
    /// [`ConfigNotFound`](ErrorKind::ConfigNotFound).
    #[cfg(feature = "config")]
    pub fn config_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.settings.config_file = Some(path.into());
        self
    }

    /// The configuration text itself, in the `conf` format: for tests and
    /// embedded defaults. It disables the file search; a reload parses the
    /// same text again.
    #[cfg(feature = "config")]
    pub fn config_str(mut self, text: impl Into<String>) -> Self {
        self.settings.config_str = Some(text.into());
        self
    }

    /// The clock [`Foundation::clock`](crate::Foundation::clock) returns: a
    /// `ManualClock` in tests. Kernel deadlines always use the OS monotonic
    /// clock (D-029).
    #[cfg(feature = "time")]
    pub fn clock(mut self, clock: impl clock_lib::Clock + 'static) -> Self {
        self.settings.clock = Some(Arc::new(clock));
        self
    }

    /// Turns a compiled-in member off for this run: `Env`, `Config`
    /// (defaults only), `Log` (no sinks), `Metrics`, `Signals`. Members
    /// others need at run time cannot be disabled, and asking is a
    /// [`Setup`](ErrorKind::Setup) error.
    pub fn disable(mut self, member: Member) -> Self {
        if !self.settings.disabled.contains(&member) {
            self.settings.disabled.push(member);
        }
        self
    }

    /// Whether the system may change process-wide state (default `true`):
    /// signal handlers, the panic hook, the umask, the open-file limit, the
    /// global logger and metrics registry, and systemd notifications.
    /// `false` for tests that start several systems in one process and for
    /// hosts that embed one.
    pub fn process_wide(mut self, on: bool) -> Self {
        self.settings.process_wide = on;
        self
    }

    /// Whether to install the crash hook (default `true`, D-016; or
    /// `[crash] enabled`). It is never installed with
    /// `process_wide(false)`.
    #[cfg(feature = "kernel")]
    pub fn crash_capture(mut self, on: bool) -> Self {
        self.settings.crash_capture = Some(on);
        self
    }

    /// Whether to take the single-instance lock (default `true`; or
    /// `[guard] lock`). `false` when a supervisor holds the lock or several
    /// copies are meant to run.
    #[cfg(feature = "sys")]
    pub fn single_instance(mut self, on: bool) -> Self {
        self.settings.single_instance = Some(on);
        self
    }

    /// The whole shutdown's deadline (default 25 s; or
    /// `[kernel] shutdown_deadline`). Zero, or more than an hour, is a
    /// [`Setup`](ErrorKind::Setup) error.
    pub fn shutdown_deadline(mut self, deadline: Duration) -> Self {
        self.settings.shutdown_deadline = Some(deadline);
        self
    }

    /// The part of the shutdown deadline kept back for the members after
    /// the subsystems (default 2 s; or `[kernel] member_reserve`). It must
    /// be shorter than the shutdown deadline.
    pub fn member_reserve(mut self, reserve: Duration) -> Self {
        self.settings.member_reserve = Some(reserve);
        self
    }

    /// Adds a subsystem. It is started at boot step 16 in ascending
    /// [`Position`](crate::lifecycle::Position), ties in registration order,
    /// and stopped in exactly the reverse of the order it actually started.
    /// A duplicate or invalid name is a [`Setup`](ErrorKind::Setup) error at
    /// `start`.
    #[cfg(feature = "kernel")]
    pub fn register(mut self, subsystem: impl crate::lifecycle::Subsystem) -> Self {
        self.settings
            .subsystems
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Box::new(subsystem));
        self
    }

    /// Runs `callback` at boot step 17, after every subsystem started and
    /// before readiness is announced. A failing callback unwinds the boot
    /// like a failing step.
    #[cfg(feature = "kernel")]
    pub fn on_ready<F>(mut self, callback: F) -> Self
    where
        F: FnOnce(&crate::Foundation) -> Result<(), crate::lifecycle::Failure> + Send + 'static,
    {
        self.settings
            .on_ready
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(Box::new(callback));
        self
    }

    /// Runs every boot step and returns the system when all succeeded. On
    /// failure nothing started is left running, and the error names the
    /// step, the member, and what to do.
    ///
    /// # Errors
    ///
    /// [`Setup`](ErrorKind::Setup) for an invalid setting; otherwise the
    /// kind of the step that failed (see [`ErrorKind`]).
    pub fn start(self) -> Result<System, Error> {
        crate::boot::start(self.settings)
    }

    /// Runs boot steps 1 to 15 (the foundation and the kernel) and returns
    /// a [`Boot`](crate::Boot) for frameworks that build their subsystems
    /// from what the foundation provides.
    ///
    /// # Errors
    ///
    /// As for [`start`](Self::start).
    #[cfg(feature = "kernel")]
    pub fn boot(self) -> Result<crate::Boot, Error> {
        crate::boot::boot(self.settings)
    }
}

/// Checks the settings before anything runs.
pub(crate) fn validate(settings: &Settings) -> Result<(), Error> {
    validate_name(&settings.name, "application")?;
    if let Some(member) = settings.disabled.iter().find(|m| !m.can_disable()) {
        return Err(setup(
            format!("the {member} member cannot be disabled at run time"),
            "remove the disable() call, or turn its feature off at compile time",
        ));
    }
    if let Some(deadline) = settings.shutdown_deadline
        && (deadline.is_zero() || deadline > MAX_SHUTDOWN_DEADLINE)
    {
        return Err(setup(
            "the shutdown deadline must be more than zero and at most one hour",
            "pass a deadline between 1 ms and 1 h to shutdown_deadline()",
        ));
    }
    if let Some(reserve) = settings.member_reserve
        && reserve
            >= settings
                .shutdown_deadline
                .unwrap_or(DEFAULT_SHUTDOWN_DEADLINE)
    {
        return Err(setup(
            "the member reserve must be shorter than the shutdown deadline",
            "pass member_reserve() a duration shorter than the shutdown deadline",
        ));
    }
    #[cfg(feature = "paths")]
    if settings.scope == Some(path_lib::Scope::Portable) && settings.portable.is_none() {
        return Err(setup(
            "Scope::Portable needs a root directory",
            "call portable(root) instead of scope(Scope::Portable)",
        ));
    }
    #[cfg(feature = "kernel")]
    {
        let mut seen: Vec<&str> = Vec::new();
        let subsystems = settings
            .subsystems
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for subsystem in subsystems.iter() {
            let name = subsystem.name();
            validate_name(name, "subsystem")?;
            if seen.contains(&name) {
                return Err(setup(
                    format!("two subsystems are named {name}"),
                    "give each subsystem its own name",
                ));
            }
            seen.push(name);
        }
    }
    Ok(())
}

pub(crate) fn validate_name(name: &str, what: &str) -> Result<(), Error> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(setup(
            format!("the {what} name {name:?} is not valid"),
            "use 1 to 64 ASCII letters, digits, '-', '_', or '.', not starting with '.'",
        ))
    }
}

fn setup(detail: impl Into<String>, action: &'static str) -> Error {
    Error::new(ErrorKind::Setup, detail, action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_name_accepts_the_documented_set() {
        for name in ["site", "hive-db", "a_b.c", "x", &"a".repeat(64)] {
            assert!(validate_name(name, "application").is_ok(), "{name}");
        }
    }

    #[test]
    fn test_validate_name_refuses_bad_names() {
        for name in [
            "",
            ".hidden",
            "has space",
            "slash/name",
            "é",
            &"a".repeat(65),
        ] {
            let error = validate_name(name, "application").err();
            assert_eq!(error.map(|e| e.kind()), Some(ErrorKind::Setup), "{name}");
        }
    }

    #[test]
    fn test_validate_refuses_undisableable_member() {
        let builder = Builder::new("app").disable(Member::Kernel);
        let error = validate(&builder.settings).err();
        assert_eq!(error.map(|e| e.kind()), Some(ErrorKind::Setup));
    }

    #[test]
    fn test_validate_refuses_bad_deadlines_and_reserves() {
        for deadline in [Duration::ZERO, Duration::from_secs(3601)] {
            let builder = Builder::new("app").shutdown_deadline(deadline);
            assert!(validate(&builder.settings).is_err(), "{deadline:?}");
        }
        let builder = Builder::new("app").shutdown_deadline(Duration::from_secs(3600));
        assert!(validate(&builder.settings).is_ok());
        let builder = Builder::new("app")
            .shutdown_deadline(Duration::from_secs(5))
            .member_reserve(Duration::from_secs(5));
        assert!(validate(&builder.settings).is_err());
    }

    #[cfg(feature = "paths")]
    #[test]
    fn test_validate_refuses_portable_scope_without_root() {
        let builder = Builder::new("app").scope(path_lib::Scope::Portable);
        assert!(validate(&builder.settings).is_err());
    }

    #[test]
    fn test_disable_is_idempotent() {
        let builder = Builder::new("app")
            .disable(Member::Log)
            .disable(Member::Log);
        assert_eq!(builder.settings.disabled, vec![Member::Log]);
    }

    #[cfg(feature = "kernel")]
    #[test]
    fn test_validate_refuses_duplicate_and_invalid_subsystem_names() {
        use crate::lifecycle::{Deadline, Failure, Link, Subsystem};

        struct Named(&'static str);
        impl Subsystem for Named {
            fn name(&self) -> &str {
                self.0
            }
            fn start(&mut self, _link: &Link) -> Result<(), Failure> {
                Ok(())
            }
            fn stop(&mut self, _deadline: Deadline) -> Result<(), Failure> {
                Ok(())
            }
        }
        let builder = Builder::new("app")
            .register(Named("a"))
            .register(Named("a"));
        assert!(validate(&builder.settings).is_err());
        let builder = Builder::new("app").register(Named("bad name"));
        assert!(validate(&builder.settings).is_err());
        let builder = Builder::new("app")
            .register(Named("a"))
            .register(Named("b"));
        assert!(validate(&builder.settings).is_ok());
    }
}
